//! The socket: same-user peers only; listening and dictation for trusted programs only.

use crate::voice::{Progress, Shared};
use anyhow::{Context, Result};
use nyx_voice_proto::voice::{
    encode, ClientLine, Error, ErrorKind, Event, Op, Reply, Request, ServerLine, Welcome, API_VERSION, MAX_LINE, MAX_SPEAK,
};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Who's calling.
pub struct Caller {
    pub name: String,
    /// May use the microphone and type.
    pub trusted: bool,
}

pub fn bind(path: &Path) -> Result<UnixListener> {
    let dir = path.parent().context("socket has no folder")?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).with_context(|| format!("bind {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn caller(stream: &UnixStream, declared: &str, own_dir: &Path, allowed: &[String]) -> Option<Caller> {
    let cred = stream.peer_cred().ok()?;
    // SAFETY: getuid has no preconditions.
    if cred.uid() != unsafe { libc::getuid() } {
        return None;
    }
    let exe: Option<PathBuf> = cred.pid().and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok());
    let program = exe.as_deref().and_then(|e| nyx_peer::program(e, own_dir));
    let named = exe.as_deref().and_then(|e| e.file_name()).map(|n| n.to_string_lossy().into_owned());
    let allowed_here = named.as_ref().is_some_and(|n| allowed.iter().any(|a| a == n));
    Some(match program {
        Some(name) => Caller { name, trusted: true },
        None => Caller { name: format!("untrusted:{declared}"), trusted: allowed_here },
    })
}

pub async fn serve(voice: Shared, listener: UnixListener, own_dir: PathBuf) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let voice = voice.clone();
        let own_dir = own_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(voice, stream, own_dir).await {
                tracing::debug!("connection ended: {e:#}");
            }
        });
    }
}

async fn connection(voice: Shared, stream: UnixStream, own_dir: PathBuf) -> Result<()> {
    let mut first = String::new();
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    reader.read_line(&mut first).await?;
    let stream = reader.get_ref().as_ref();
    let hello = match serde_json::from_str::<ClientLine>(first.trim()) {
        Ok(ClientLine::Hello(h)) => h,
        _ => return Ok(()),
    };
    let Some(who) = caller(stream, &hello.client, &own_dir, &voice.cfg.permissions.microphone) else { return Ok(()) };
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerLine>();
    let mut write = write;
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if write.write_all(encode(&line).as_bytes()).await.is_err() {
                break;
            }
        }
    });
    if hello.api_version != API_VERSION {
        let _ = tx.send(ServerLine::Error {
            id: None,
            error: Error::new(ErrorKind::IncompatibleVersion, format!("Nyx Voice speaks version {API_VERSION}")),
        });
        return Ok(());
    }
    let _ = tx.send(ServerLine::Hello(Welcome { api_version: API_VERSION, server: env!("CARGO_PKG_VERSION").into(), client: who.name.clone() }));
    let who = Arc::new(who);
    let open: Arc<Mutex<HashMap<u64, CancellationToken>>> = Default::default();
    let closed = CancellationToken::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 || line.len() > MAX_LINE {
            break;
        }
        let request = match serde_json::from_slice::<ClientLine>(&line) {
            Ok(ClientLine::Request(r)) => r,
            Ok(ClientLine::Hello(_)) => continue,
            Err(e) => {
                let _ = tx.send(ServerLine::Error { id: None, error: Error::new(ErrorKind::InvalidRequest, e.to_string()) });
                continue;
            }
        };
        if let Op::Cancel { request: target } = request.op {
            if let Some(t) = open.lock().unwrap().get(&target) {
                t.cancel();
            }
            let _ = tx.send(ServerLine::Result { id: request.id, result: Reply::Ok });
            continue;
        }
        let cancel = closed.child_token();
        open.lock().unwrap().insert(request.id, cancel.clone());
        let (voice, who, tx, open) = (voice.clone(), who.clone(), tx.clone(), open.clone());
        tokio::spawn(async move {
            let id = request.id;
            let line = match handle(&voice, &who, request, &tx, cancel).await {
                Ok(result) => ServerLine::Result { id, result },
                Err(error) => ServerLine::Error { id: Some(id), error },
            };
            let _ = tx.send(line);
            open.lock().unwrap().remove(&id);
        });
    }
    closed.cancel();
    Ok(())
}

async fn handle(voice: &Shared, who: &Caller, request: Request, tx: &mpsc::UnboundedSender<ServerLine>, cancel: CancellationToken) -> Result<Reply, Error> {
    let id = request.id;
    let trusted = || {
        if who.trusted {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::PermissionDenied, format!("{} may not use the microphone", who.name)))
        }
    };
    match request.op {
        Op::Status {} => Ok(Reply::Status(voice.state())),
        Op::Listen { mode, until, partials, max_seconds } => {
            trusted()?;
            tracing::info!(caller = %who.name, ?mode, "listening");
            let (ptx, mut prx) = mpsc::unbounded_channel();
            let forward = {
                let tx = tx.clone();
                tokio::spawn(async move {
                    while let Some(p) = prx.recv().await {
                        let event = match p {
                            Progress::Capturing => Event::Capturing,
                            Progress::Partial(text) if partials => Event::Partial { text },
                            Progress::Partial(_) => continue,
                            Progress::Transcribing => Event::Transcribing,
                        };
                        let _ = tx.send(ServerLine::Event { id, event });
                    }
                })
            };
            let text = voice.listen(mode, until, max_seconds.unwrap_or(120).min(120), cancel, Some(ptx)).await;
            forward.abort();
            text.map(|text| Reply::Transcript { text })
        }
        Op::StopListening {} => {
            trusted()?;
            voice.stop_listening();
            Ok(Reply::Ok)
        }
        Op::Dictate { action } => {
            trusted()?;
            Ok(match voice.dictate(action).await? {
                Some(text) => Reply::Dictated { text },
                None => Reply::Ok,
            })
        }
        Op::Speak { text, voice: v, speed } => {
            if text.len() > MAX_SPEAK {
                return Err(Error::new(ErrorKind::InvalidRequest, format!("at most {MAX_SPEAK} bytes of text")));
            }
            if speed.is_some_and(|s| !s.is_finite() || !(0.5..=2.0).contains(&s)) {
                return Err(Error::new(ErrorKind::InvalidRequest, "speed is 0.5 to 2"));
            }
            let _ = tx.send(ServerLine::Event { id, event: Event::Speaking });
            voice.speak(&text, v, speed, cancel).await.map(|ms| Reply::Spoken { ms })
        }
        Op::StopSpeaking {} => {
            voice.stop_speaking();
            Ok(Reply::Ok)
        }
        Op::Devices {} => {
            let (inputs, outputs) = crate::desktop::devices().await.map_err(|e| Error::new(ErrorKind::Unavailable, format!("{e:#}")))?;
            Ok(Reply::Devices { inputs, outputs })
        }
        Op::Watch {} => {
            let mut rx = voice.subscribe();
            let _ = tx.send(ServerLine::Event { id, event: Event::State(rx.borrow().clone()) });
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(Error::new(ErrorKind::Cancelled, "cancelled")),
                    changed = rx.changed() => {
                        if changed.is_err() {
                            return Ok(Reply::Ok);
                        }
                        let _ = tx.send(ServerLine::Event { id, event: Event::State(rx.borrow().clone()) });
                    }
                }
            }
        }
        Op::Cancel { .. } => Ok(Reply::Ok),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_voice_client::{Client, Item};
    use nyx_voice_proto::voice::{DictateAction, Mode, Until};

    async fn start(trusted_here: bool) -> (tempfile::TempDir, Client) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("run/voice.sock");
        let listener = bind(&socket).unwrap();
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        let own = if trusted_here { exe_dir } else { dir.path().to_path_buf() };
        let voice = crate::voice::Voice::new(crate::config::VoiceConfig::default());
        tokio::spawn(serve(voice, listener, own));
        (dir, Client::with_socket("nyx-test", socket))
    }

    #[tokio::test]
    async fn status_watch_and_checks() {
        let (_dir, client) = start(true).await;
        assert_eq!(client.status().await.unwrap().activity, "idle");
        let mut watch = client.watch().await.unwrap();
        assert!(matches!(watch.next().await, Some(Ok(Item::Event(Event::State(s)))) if s.activity == "idle"));
        watch.cancel().await.unwrap();
        let bad = client.call(Op::Speak { text: "hi".into(), voice: None, speed: Some(9.0) }).await.unwrap_err();
        assert_eq!(bad.kind, ErrorKind::InvalidRequest);
        assert_eq!(client.dictate(DictateAction::Stop).await.unwrap(), None);
        client.stop_speaking().await.unwrap();
    }

    #[tokio::test]
    async fn only_trusted_programs_listen_or_type() {
        let (_dir, client) = start(false).await;
        let e = client.listen_text(Mode::Assistant, Until::Silence).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::PermissionDenied);
        let e = client.dictate(DictateAction::Toggle).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::PermissionDenied);
        assert_eq!(client.status().await.unwrap().activity, "idle");
    }
}
