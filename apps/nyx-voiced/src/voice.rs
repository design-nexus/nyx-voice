//! What the daemon does with the microphone and the speakers: one take at a time,
//! speech one phrase at a time, engines loaded when first needed and let go when idle.

use crate::config::VoiceConfig;
use anyhow::{anyhow, Context, Result};
use nyx_voice_engine::asr::Stt;
use nyx_voice_engine::tts::KokoroProvider;
use nyx_voice_engine::{AudioEngine, EnergyVad, Silero, StreamingVad, VadDecision, VadGate};
use nyx_voice_proto::voice::{Error, ErrorKind, Mode, State, Until};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

/// Engines unload after this long unused.
const IDLE: Duration = Duration::from_secs(300);
/// A take that waits for silence ends empty when nobody speaks this long.
const NO_SPEECH: Duration = Duration::from_secs(8);

struct Audio {
    engine: AudioEngine,
    samples: tokio::sync::Mutex<mpsc::Receiver<Vec<i16>>>,
}

/// The take in progress.
struct Take {
    stop: watch::Sender<bool>,
    /// A dictation the keyboard runs: its text, when it's done.
    dictation: Option<tokio::sync::oneshot::Receiver<Result<String>>>,
}

pub struct Voice {
    pub cfg: VoiceConfig,
    state: watch::Sender<State>,
    audio: tokio::sync::Mutex<Option<Arc<Audio>>>,
    stt: tokio::sync::Mutex<Option<Arc<Stt>>>,
    tts: tokio::sync::Mutex<Option<Arc<KokoroProvider>>>,
    silero: Mutex<Option<Arc<Silero>>>,
    take: Mutex<Option<Take>>,
    speak_lock: tokio::sync::Mutex<()>,
    speaking: Mutex<Option<CancellationToken>>,
    last_used: Mutex<Instant>,
}

pub type Shared = Arc<Voice>;

fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, e.to_string())
}

impl Voice {
    pub fn new(cfg: VoiceConfig) -> Shared {
        let (state, _) = watch::channel(State { activity: "idle".into(), ..Default::default() });
        let voice = Arc::new(Voice {
            cfg,
            state,
            audio: Default::default(),
            stt: Default::default(),
            tts: Default::default(),
            silero: Default::default(),
            take: Default::default(),
            speak_lock: Default::default(),
            speaking: Default::default(),
            last_used: Mutex::new(Instant::now()),
        });
        voice.write_state_file();
        voice
    }

    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }

    fn set_activity(&self, activity: &str, mode: Option<Mode>) {
        self.state.send_modify(|s| {
            s.activity = activity.into();
            s.mode = mode;
        });
        self.write_state_file();
    }

    /// `$XDG_RUNTIME_DIR/nyx-voice/state`: `idle`, `recording`, `transcribing` or
    /// `speaking`, as VoxType's state file, for bars and scripts.
    fn write_state_file(&self) {
        let dir = nyx_voice_engine::paths::runtime_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("state.tmp");
        if std::fs::write(&tmp, self.state.borrow().activity.as_bytes()).is_ok() {
            let _ = std::fs::rename(tmp, dir.join("state"));
        }
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    async fn audio(&self) -> Result<Arc<Audio>, Error> {
        let mut slot = self.audio.lock().await;
        if let Some(a) = slot.as_ref() {
            return Ok(a.clone());
        }
        let settings = self.cfg.speech.clone();
        let engine = tokio::task::spawn_blocking(move || AudioEngine::start(&settings))
            .await
            .map_err(unavailable)?
            .map_err(|e| unavailable(format!("the audio system isn't available: {e:#}")))?;
        let samples = engine.take_samples().ok_or_else(|| unavailable("the microphone stream is already taken"))?;
        let audio = Arc::new(Audio { engine, samples: tokio::sync::Mutex::new(samples) });
        *slot = Some(audio.clone());
        Ok(audio)
    }

    async fn stt(&self) -> Result<Arc<Stt>, Error> {
        let mut slot = self.stt.lock().await;
        if let Some(s) = slot.as_ref().filter(|s| s.streaming()) {
            return Ok(s.clone());
        }
        let stt = Arc::new(Stt::new(&self.cfg.speech).await);
        if let Some(why) = stt.unavailable() {
            let why = why.to_string();
            self.state.send_modify(|s| s.recognizer_unavailable = Some(why.clone()));
            return Err(unavailable(format!("speech recognition isn't available: {why}")));
        }
        self.state.send_modify(|s| s.recognizer_unavailable = None);
        *slot = Some(stt.clone());
        Ok(stt)
    }

    async fn tts(&self) -> Result<Arc<KokoroProvider>, Error> {
        let mut slot = self.tts.lock().await;
        if let Some(t) = slot.as_ref() {
            return Ok(t.clone());
        }
        let settings = self.cfg.speech.clone();
        let loaded = tokio::task::spawn_blocking(move || KokoroProvider::load(&settings)).await.map_err(unavailable)?;
        match loaded {
            Ok(t) => {
                let t = Arc::new(t);
                self.state.send_modify(|s| s.synthesizer_unavailable = None);
                *slot = Some(t.clone());
                Ok(t)
            }
            Err(e) => {
                let why = format!("{e:#}");
                self.state.send_modify(|s| s.synthesizer_unavailable = Some(why.clone()));
                Err(unavailable(format!("speech synthesis isn't available: {why}")))
            }
        }
    }

    fn vad(&self, until: Until) -> StreamingVad {
        let v = &self.cfg.speech.vad;
        if until == Until::Silence && v.wants_silero() {
            let mut slot = self.silero.lock().unwrap();
            if slot.is_none() {
                match Silero::load(v) {
                    Ok(s) => *slot = Some(Arc::new(s)),
                    Err(e) => tracing::warn!("Silero unavailable, using the energy gate: {e:#}"),
                }
            }
            return StreamingVad::new(v, slot.clone(), v.hangover_ms.max(700), v.start_confirm_frames, v.energy_threshold);
        }
        StreamingVad::Energy(EnergyVad::with_confirm(v.energy_threshold, v.hangover_ms.max(700), 32, v.start_confirm_frames))
    }

    /// Start a take; `Busy` while another runs.
    fn claim(&self, dictation: bool) -> Result<(watch::Receiver<bool>, CancellationToken), Error> {
        let mut take = self.take.lock().unwrap();
        if take.is_some() {
            return Err(Error::new(ErrorKind::Busy, "the microphone is in use by another take"));
        }
        let (stop, stop_rx) = watch::channel(false);
        let cancel = CancellationToken::new();
        *take = Some(Take { stop, dictation: dictation.then(|| tokio::sync::oneshot::channel().1) });
        Ok((stop_rx, cancel))
    }

    fn release(&self) {
        *self.take.lock().unwrap() = None;
        self.set_activity(if self.speaking.lock().unwrap().is_some() { "speaking" } else { "idle" }, None);
        self.touch();
    }

    /// End the current take now (push to talk released).
    pub fn stop_listening(&self) {
        if let Some(t) = self.take.lock().unwrap().as_ref() {
            let _ = t.stop.send(true);
        }
    }

    /// Capture a take and return its text. `partial` gets words as they're recognised.
    pub async fn listen(
        self: &Arc<Self>,
        mode: Mode,
        until: Until,
        max_seconds: u32,
        cancel: CancellationToken,
        events: Option<mpsc::UnboundedSender<Progress>>,
    ) -> Result<String, Error> {
        let (stop, take_cancel) = self.claim(false)?;
        let joined = CancellationToken::new();
        let j = joined.clone();
        let (a, b) = (cancel.clone(), take_cancel.clone());
        tokio::spawn(async move {
            tokio::select! { _ = a.cancelled() => {}, _ = b.cancelled() => {} }
            j.cancel();
        });
        let result = self.capture(mode, until, max_seconds, stop, joined.clone(), events).await;
        joined.cancel();
        self.release();
        result
    }

    async fn capture(
        &self,
        mode: Mode,
        until: Until,
        max_seconds: u32,
        mut stop: watch::Receiver<bool>,
        cancel: CancellationToken,
        events: Option<mpsc::UnboundedSender<Progress>>,
    ) -> Result<String, Error> {
        self.touch();
        let audio = self.audio().await?;
        let stt = self.stt().await?;
        let mut vad = self.vad(until);
        let mut samples = audio.samples.lock().await;
        while samples.try_recv().is_ok() {}
        stt.stream_begin().await.map_err(unavailable)?;
        audio.engine.start_capture().map_err(unavailable)?;
        self.set_activity("recording", Some(mode));
        if let Some(tx) = &events {
            let _ = tx.send(Progress::Capturing);
        }
        let partials = events.clone().and_then(|tx| {
            let mut replies = stt.asr()?.subscribe();
            Some(tokio::spawn(async move {
                while let Ok(reply) = replies.recv().await {
                    if let nyx_voice_proto::asr::Reply::Partial { text, .. } = reply {
                        let _ = tx.send(Progress::Partial(text));
                    }
                }
            }))
        });
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_secs(max_seconds.clamp(1, 300) as u64);
        let mut pcm: Vec<i16> = Vec::new();
        let mut cancelled = false;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { cancelled = true; break; }
                changed = stop.changed() => if changed.is_err() || *stop.borrow() { break },
                chunk = samples.recv() => {
                    let Some(chunk) = chunk else { break };
                    stt.tap(&chunk);
                    pcm.extend_from_slice(&chunk);
                    let decision = vad.push(&chunk);
                    if until == Until::Silence {
                        if matches!(decision, VadDecision::SpeechEnd) {
                            break;
                        }
                        if !vad.heard_speech() && started.elapsed() > NO_SPEECH {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        let _ = audio.engine.stop_capture();
        drop(samples);
        if let Some(p) = partials {
            p.abort();
        }
        if cancelled {
            stt.cancel_listen().await;
            return Err(Error::new(ErrorKind::Cancelled, "cancelled"));
        }
        self.set_activity("transcribing", Some(mode));
        if let Some(tx) = &events {
            let _ = tx.send(Progress::Transcribing);
        }
        let text = stt.finish(&pcm, cancel).await.map_err(unavailable)?;
        self.touch();
        Ok(text.text.trim().to_string())
    }

    /// Dictation for the keyboard: start, stop (returns what was typed) or toggle.
    pub async fn dictate(self: &Arc<Self>, action: nyx_voice_proto::voice::DictateAction) -> Result<Option<String>, Error> {
        use nyx_voice_proto::voice::DictateAction::*;
        let running = self.take.lock().unwrap().as_ref().map(|t| t.dictation.is_some());
        match (action, running) {
            (Start, Some(true)) => Ok(None),
            (Start | Toggle, Some(false)) => Err(Error::new(ErrorKind::Busy, "the microphone is in use by another take")),
            (Start | Toggle, None) => {
                self.start_dictation()?;
                Ok(None)
            }
            (Stop | Toggle, Some(true)) => self.finish_dictation().await.map(Some),
            (Stop, _) => Ok(None),
        }
    }

    fn start_dictation(self: &Arc<Self>) -> Result<(), Error> {
        let (stop, cancel) = self.claim(true)?;
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        if let Some(t) = self.take.lock().unwrap().as_mut() {
            t.dictation = Some(done_rx);
        }
        let voice = self.clone();
        let max = self.cfg.dictation.max_seconds;
        let pause = self.cfg.dictation.pause_media;
        tokio::spawn(async move {
            let paused = if pause { crate::desktop::pause_media().await } else { Vec::new() };
            let result = voice.capture(Mode::Dictation, Until::Stop, max, stop, cancel, None).await;
            crate::desktop::resume_media(&paused).await;
            let typed = match result {
                Ok(text) if !text.is_empty() => crate::desktop::deliver(&voice.cfg.dictation, &text).await.map(|_| text),
                Ok(text) => Ok(text),
                Err(e) => Err(anyhow!("{e}")),
            };
            if let Err(e) = &typed {
                tracing::warn!("dictation: {e:#}");
            }
            voice.release();
            let _ = done_tx.send(typed);
        });
        Ok(())
    }

    async fn finish_dictation(&self) -> Result<String, Error> {
        let done = {
            let mut take = self.take.lock().unwrap();
            let Some(t) = take.as_mut() else { return Ok(String::new()) };
            let _ = t.stop.send(true);
            t.dictation.take()
        };
        match done {
            Some(rx) => match tokio::time::timeout(Duration::from_secs(30), rx).await {
                Ok(Ok(Ok(text))) => Ok(text),
                Ok(Ok(Err(e))) => Err(unavailable(format!("{e:#}"))),
                _ => Err(Error::new(ErrorKind::Internal, "dictation didn't finish")),
            },
            None => Ok(String::new()),
        }
    }

    /// Say `text` and wait until it has played, or until `cancel`.
    pub async fn speak(&self, text: &str, voice: Option<String>, speed: Option<f32>, cancel: CancellationToken) -> Result<u64, Error> {
        let _one = self.speak_lock.lock().await;
        self.touch();
        let tts = self.tts().await?;
        let audio = self.audio().await?;
        let started = Instant::now();
        let token = CancellationToken::new();
        *self.speaking.lock().unwrap() = Some(token.clone());
        let mode = self.state.borrow().mode;
        if self.take.lock().unwrap().is_none() {
            self.set_activity("speaking", mode);
        }
        let result = async {
            let text = text.to_string();
            let synth_cancel = token.clone();
            let pcm = tokio::task::spawn_blocking(move || match (voice, speed) {
                (None, None) => tts.speak_cancellable(&text, &synth_cancel),
                (v, s) => tts.speak_with(&text, v.as_deref().unwrap_or(&tts.voice()), s.unwrap_or(1.0)),
            })
            .await
            .map_err(unavailable)?
            .map_err(|e| unavailable(format!("{e:#}")))?;
            audio.engine.play(pcm).map_err(unavailable)?;
            tokio::select! {
                r = audio.engine.wait_playback() => r.map_err(unavailable),
                _ = cancel.cancelled() => { let _ = audio.engine.cancel_playback(); Err(Error::new(ErrorKind::Cancelled, "cancelled")) }
                _ = token.cancelled() => { let _ = audio.engine.cancel_playback(); Err(Error::new(ErrorKind::Cancelled, "stopped")) }
            }
        }
        .await;
        *self.speaking.lock().unwrap() = None;
        if self.take.lock().unwrap().is_none() {
            self.set_activity("idle", None);
        }
        self.touch();
        result.map(|_| started.elapsed().as_millis() as u64)
    }

    pub fn stop_speaking(&self) {
        if let Some(t) = self.speaking.lock().unwrap().take() {
            t.cancel();
        }
    }

    /// Let go of the engines after a while unused: the recognizer worker, Kokoro and the
    /// PipeWire streams.
    pub async fn release_idle(&self) {
        let busy = self.take.lock().unwrap().is_some() || self.speaking.lock().unwrap().is_some();
        if busy || self.last_used.lock().unwrap().elapsed() < IDLE {
            return;
        }
        let had = self.stt.lock().await.take().is_some() | self.tts.lock().await.take().is_some() | self.audio.lock().await.take().is_some();
        self.silero.lock().unwrap().take();
        if had {
            tracing::info!("idle: speech engines unloaded");
        }
    }
}

/// What a take reports while it runs.
pub enum Progress {
    Capturing,
    Partial(String),
    Transcribing,
}

/// The process's own folder, for the trust check.
pub fn own_dir() -> Result<std::path::PathBuf> {
    std::env::current_exe()?.parent().map(|p| p.to_path_buf()).context("no folder for this program")
}
