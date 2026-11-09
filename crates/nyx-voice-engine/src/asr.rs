//! Client for the streaming recognizer worker (`nyx-voice-asr`, protocol in
//! `nyx_voice_proto::asr`).
//!
//! One warm worker serves every listening mode. Each utterance gets an id from
//! [`AsrWorker::begin`]; audio goes in with [`AsrWorker::feed`] as it is captured, partial
//! words come back on [`AsrWorker::subscribe`], and [`AsrWorker::finish`] waits for the
//! final text. A worker that exits is started again on the next utterance.

use anyhow::{bail, Context, Result};
use nyx_voice_proto::asr::{Frame, Reply};
use crate::config::Settings as Config;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc};

/// The model directory and worker binary for `[stt]`, or why streaming can't be used.
pub fn resolve(cfg: &Config) -> Result<(PathBuf, PathBuf)> {
    let model = if cfg.stt.model == "auto" || cfg.stt.model.is_empty() {
        let mut dirs = crate::paths::model_dirs().into_iter().map(|d| d.join(nyx_voice_proto::asr::MODEL_NAME));
        let first = crate::paths::models_dir().join(nyx_voice_proto::asr::MODEL_NAME);
        dirs.find(|d| d.join(nyx_voice_proto::asr::ENCODER).is_file()).unwrap_or(first)
    } else {
        PathBuf::from(&cfg.stt.model)
    };
    anyhow::ensure!(
        model.join(nyx_voice_proto::asr::ENCODER).is_file(),
        "the streaming speech model isn't installed (run: nyx-voice models fetch asr)"
    );
    let worker = worker_binary().context("nyx-voice-asr isn't installed (install Nyx Voice)")?;
    Ok((model, worker))
}

/// `nyx-voice-asr` beside this program, else on `PATH`.
pub fn worker_binary() -> Option<PathBuf> {
    let here = std::env::current_exe().ok().map(|e| e.with_file_name("nyx-voice-asr")).filter(|p| p.is_file());
    here.or_else(|| {
        std::env::var_os("PATH")?.to_str()?.split(':').map(|d| PathBuf::from(d).join("nyx-voice-asr")).find(|p| p.is_file())
    })
}

struct Running {
    frames: mpsc::UnboundedSender<Vec<u8>>,
    alive: std::sync::Arc<AtomicBool>,
    _child: tokio::process::Child,
}

pub struct AsrWorker {
    worker: PathBuf,
    model: PathBuf,
    threads: u32,
    provider: String,
    running: Mutex<Option<Running>>,
    replies: broadcast::Sender<Reply>,
    next: AtomicU32,
    /// How long the model took to load, the last time the worker started.
    pub load_ms: std::sync::atomic::AtomicU64,
}

impl AsrWorker {
    /// Start the worker and wait until its model is loaded.
    pub async fn start(cfg: &Config) -> Result<Self> {
        let (model, worker) = resolve(cfg)?;
        Self::launch(cfg, model, worker).await
    }

    /// As [`start`](Self::start), with the worker binary given (tests use the build's).
    pub async fn start_with(cfg: &Config, worker: &std::path::Path) -> Result<Self> {
        let model = PathBuf::from(&cfg.stt.model);
        Self::launch(cfg, model, worker.to_path_buf()).await
    }

    async fn launch(cfg: &Config, model: PathBuf, worker: PathBuf) -> Result<Self> {
        let (replies, _) = broadcast::channel(256);
        let w = Self {
            worker,
            model,
            threads: cfg.stt.threads.max(1),
            provider: cfg.stt.device.clone(),
            running: Mutex::new(None),
            replies,
            next: AtomicU32::new(1),
            load_ms: std::sync::atomic::AtomicU64::new(0),
        };
        w.spawn().await?;
        Ok(w)
    }

    async fn spawn(&self) -> Result<()> {
        let mut child = tokio::process::Command::new(&self.worker)
            .arg("--model")
            .arg(&self.model)
            .args(["--threads", &self.threads.to_string(), "--provider", &self.provider])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start {}", self.worker.display()))?;
        let mut stdin = child.stdin.take().context("worker stdin")?;
        let mut lines = BufReader::new(child.stdout.take().context("worker stdout")?).lines();
        // The first line says whether the model loaded.
        let first = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
            .await
            .context("the speech recognizer took too long to load")??
            .context("the speech recognizer exited while loading")?;
        match serde_json::from_str::<Reply>(&first)? {
            Reply::Ready { load_ms } => {
                self.load_ms.store(load_ms, Ordering::Relaxed);
                tracing::info!(load_ms, "streaming recognizer ready")
            }
            Reply::Error { message } => bail!("speech recognizer: {message}"),
            other => bail!("unexpected first reply from the speech recognizer: {other:?}"),
        }
        let alive = std::sync::Arc::new(AtomicBool::new(true));
        let (frames, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let alive_w = alive.clone();
        tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            alive_w.store(false, Ordering::SeqCst);
        });
        let alive_r = alive.clone();
        let replies = self.replies.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<Reply>(&line) {
                    Ok(reply) => {
                        let _ = replies.send(reply);
                    }
                    Err(err) => tracing::warn!(%err, "unreadable reply from the speech recognizer"),
                }
            }
            alive_r.store(false, Ordering::SeqCst);
            tracing::warn!("streaming recognizer exited");
        });
        *self.running.lock().unwrap() = Some(Running { frames, alive, _child: child });
        Ok(())
    }

    fn send(&self, frame: Frame) -> bool {
        let running = self.running.lock().unwrap();
        match running.as_ref() {
            Some(r) if r.alive.load(Ordering::SeqCst) => r.frames.send(frame.encode()).is_ok(),
            _ => false,
        }
    }

    fn alive(&self) -> bool {
        self.running.lock().unwrap().as_ref().is_some_and(|r| r.alive.load(Ordering::SeqCst))
    }

    /// A new utterance id; starts the worker again first if it had exited.
    pub async fn begin(&self) -> Result<u32> {
        if !self.alive() {
            tracing::info!("restarting the streaming recognizer");
            self.spawn().await?;
        }
        Ok(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Audio for an utterance. Never blocks; audio for a dead worker is dropped.
    pub fn feed(&self, utterance: u32, samples: &[i16]) {
        if !samples.is_empty() {
            self.send(Frame::audio(utterance, samples));
        }
    }

    pub fn abort(&self, utterance: u32) {
        self.send(Frame::abort(utterance));
    }

    /// Partial and final results for every utterance.
    pub fn subscribe(&self) -> broadcast::Receiver<Reply> {
        self.replies.subscribe()
    }

    /// End the utterance and wait for its final text.
    pub async fn finish(&self, utterance: u32, cancel: &tokio_util::sync::CancellationToken) -> Result<String> {
        let mut rx = self.subscribe();
        anyhow::ensure!(self.send(Frame::finish(utterance)), "the speech recognizer isn't running");
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(Reply::Final { u, text, decode_ms, .. }) if u == utterance => {
                        tracing::debug!(decode_ms, "final transcript");
                        return Ok(text);
                    }
                    Ok(Reply::Error { message }) => bail!("speech recognizer: {message}"),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => bail!("the speech recognizer stopped"),
                }
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => {
                self.abort(utterance);
                bail!("stt cancelled")
            }
            r = tokio::time::timeout(Duration::from_secs(10), wait) => r.context("the speech recognizer didn't answer")?,
        }
    }
}

/// A take's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub is_partial: bool,
}

/// Speech to text for a listening program: one warm streaming worker. `begin` starts a
/// take, `tap` feeds each captured chunk, `finish` gives the take's text. Without the
/// worker (model or binary missing, failed to load) every take fails with the reason.
pub struct Stt {
    asr: Option<AsrWorker>,
    unavailable: String,
    current: Mutex<Option<u32>>,
}

impl Stt {
    pub async fn new(cfg: &Config) -> Self {
        match AsrWorker::start(cfg).await {
            Ok(w) => Self { asr: Some(w), unavailable: String::new(), current: Mutex::new(None) },
            Err(err) => {
                tracing::warn!("speech recognition unavailable: {err:#}");
                Self { asr: None, unavailable: format!("{err:#}"), current: Mutex::new(None) }
            }
        }
    }

    /// The streaming worker, when it runs (partials, duplex).
    pub fn asr(&self) -> Option<&AsrWorker> {
        self.asr.as_ref()
    }

    pub fn streaming(&self) -> bool {
        self.asr.is_some()
    }

    /// Why there's no recognizer, when there isn't.
    pub fn unavailable(&self) -> Option<&str> {
        (self.asr.is_none()).then_some(self.unavailable.as_str())
    }

    fn worker(&self) -> Result<&AsrWorker> {
        self.asr.as_ref().ok_or_else(|| anyhow::anyhow!("speech recognition isn't available: {}", self.unavailable))
    }

    /// Start listening for a take (PTT key-down, a wake utterance's onset).
    pub async fn begin(&self) -> Result<()> {
        self.stream_begin().await
    }

    /// Start a streamed utterance.
    pub async fn stream_begin(&self) -> Result<()> {
        let asr = self.worker()?;
        let u = asr.begin().await?;
        if let Some(old) = self.current.lock().unwrap().replace(u) {
            asr.abort(old);
        }
        Ok(())
    }

    /// Captured audio for the current take.
    pub fn tap(&self, pcm: &[i16]) {
        if let (Some(asr), Some(u)) = (&self.asr, *self.current.lock().unwrap()) {
            asr.feed(u, pcm);
        }
    }

    pub async fn cancel_listen(&self) {
        if let (Some(asr), Some(u)) = (&self.asr, self.current.lock().unwrap().take()) {
            asr.abort(u);
        }
    }

    /// The take's text. Streamed audio was decoded while it was captured, so only the tail
    /// is left; a take that wasn't streamed is transcribed whole.
    pub async fn finish(&self, pcm: &[i16], cancel: tokio_util::sync::CancellationToken) -> Result<Transcript> {
        let asr = self.worker()?;
        let current = self.current.lock().unwrap().take();
        let text = match current {
            Some(u) => asr.finish(u, &cancel).await?,
            None => return self.transcribe(pcm, cancel).await,
        };
        Ok(Transcript { text, is_partial: false })
    }

    /// Transcribe a whole buffer.
    pub async fn transcribe(&self, pcm: &[i16], cancel: tokio_util::sync::CancellationToken) -> Result<Transcript> {
        let asr = self.worker()?;
        let u = asr.begin().await?;
        for chunk in pcm.chunks(8_000) {
            asr.feed(u, chunk);
        }
        Ok(Transcript { text: asr.finish(u, &cancel).await?, is_partial: false })
    }
}

/// Where the streaming model is, if any Nyx speech program installed it.
pub fn installed_model() -> Option<PathBuf> {
    crate::paths::model_dirs()
        .into_iter()
        .map(|d| d.join(nyx_voice_proto::asr::MODEL_NAME))
        .find(|d| d.join(nyx_voice_proto::asr::ENCODER).is_file())
}

/// Download the streaming speech model into this program's models folder (about 650 MB),
/// through a temporary folder so an interrupted download never looks installed. Only on
/// request: nothing downloads it by itself.
pub fn fetch_model() -> Result<PathBuf> {
    use nyx_voice_proto::asr::{ENCODER, MODEL_NAME, MODEL_URL};
    if let Some(found) = installed_model() {
        return Ok(found);
    }
    let dir = crate::paths::models_dir();
    let target = dir.join(MODEL_NAME);
    std::fs::create_dir_all(&dir)?;
    let tmp = tempfile::tempdir_in(&dir)?;
    let archive = tmp.path().join("model.tar.bz2");
    let ok = std::process::Command::new("curl").args(["-fL", "--progress-bar", "-o"]).arg(&archive).arg(MODEL_URL).status()?.success();
    anyhow::ensure!(ok, "the download failed");
    let ok = std::process::Command::new("tar").arg("xjf").arg(&archive).arg("-C").arg(tmp.path()).status()?.success();
    anyhow::ensure!(ok, "the archive could not be unpacked");
    let unpacked = tmp.path().join(MODEL_NAME);
    anyhow::ensure!(unpacked.join(ENCODER).is_file(), "the archive doesn't hold {MODEL_NAME}");
    let _ = std::fs::remove_dir_all(unpacked.join("test_wavs"));
    std::fs::rename(&unpacked, &target)?;
    Ok(target)
}
