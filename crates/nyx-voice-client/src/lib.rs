//! The client apps use to reach Nyx Voice (`nyx-voiced`).
//!
//! ```no_run
//! # async fn f() -> nyx_voice_client::Result<()> {
//! use nyx_voice_client::{Client, proto::{Mode, Until}};
//! let voice = Client::new("nyx-chat");
//! let text = voice.listen_text(Mode::Dictation, Until::Silence).await?;
//! voice.speak("Done.").await?;
//! # Ok(()) }
//! ```

pub use nyx_voice_proto::voice as proto;

use proto::{ClientLine, DictateAction, Error, ErrorKind, Event, Hello, Mode, Op, Reply, Request, ServerLine, State, Until, API_VERSION};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

pub type Result<T> = std::result::Result<T, Error>;

/// Where the daemon listens: `NYX_VOICE_SOCKET`, else `$XDG_RUNTIME_DIR/nyx-voice/voice.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(proto::SOCKET_ENV).filter(|p| !p.is_empty()) {
        return path.into();
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        // SAFETY: getuid has no preconditions.
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    runtime.join("nyx-voice").join(proto::SOCKET_NAME)
}

/// The socket must be a socket, ours, and closed to everyone else.
fn check_socket(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| Error::new(ErrorKind::Unavailable, format!("Nyx Voice isn't running ({}: {e})", path.display())))?;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    if !meta.file_type().is_socket() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(Error::new(ErrorKind::PermissionDenied, format!("{} isn't a private socket of this user", path.display())));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Client {
    app: String,
    socket: PathBuf,
}

/// An event, or the request's result.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Event(Event),
    Done(Reply),
}

/// One request's events and result. Dropping it closes the connection, which cancels
/// the request.
pub struct Stream {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    id: u64,
    done: bool,
}

impl Client {
    pub fn new(app: &str) -> Client {
        Client { app: app.into(), socket: socket_path() }
    }

    pub fn with_socket(app: &str, socket: impl Into<PathBuf>) -> Client {
        Client { app: app.into(), socket: socket.into() }
    }

    /// Whether the daemon's socket is there.
    pub fn available(&self) -> bool {
        check_socket(&self.socket).is_ok()
    }

    async fn connect(&self) -> Result<(BufReader<OwnedReadHalf>, OwnedWriteHalf)> {
        check_socket(&self.socket)?;
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| Error::new(ErrorKind::Unavailable, format!("Nyx Voice isn't reachable: {e}")))?;
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        send(&mut write, &ClientLine::Hello(Hello { api_version: API_VERSION, client: self.app.clone() })).await?;
        let welcome = tokio::time::timeout(Duration::from_secs(10), read_line(&mut reader))
            .await
            .map_err(|_| Error::new(ErrorKind::Unavailable, "Nyx Voice didn't answer"))??;
        match welcome {
            ServerLine::Hello(w) if w.api_version == API_VERSION => Ok((reader, write)),
            ServerLine::Hello(w) => Err(Error::new(
                ErrorKind::IncompatibleVersion,
                format!("Nyx Voice speaks version {}, this app version {API_VERSION}", w.api_version),
            )),
            ServerLine::Error { error, .. } => Err(error),
            _ => Err(Error::new(ErrorKind::Internal, "unexpected reply from Nyx Voice")),
        }
    }

    /// Send `op` and read its events and result as they come.
    pub async fn stream(&self, op: Op) -> Result<Stream> {
        let (reader, mut writer) = self.connect().await?;
        send(&mut writer, &ClientLine::Request(Request { id: 1, op })).await?;
        Ok(Stream { reader, writer, id: 1, done: false })
    }

    /// Send `op` and wait for its result, skipping events.
    pub async fn call(&self, op: Op) -> Result<Reply> {
        let mut s = self.stream(op).await?;
        while let Some(item) = s.next().await {
            if let Item::Done(reply) = item? {
                return Ok(reply);
            }
        }
        Err(Error::new(ErrorKind::Unavailable, "Nyx Voice closed the connection"))
    }

    pub async fn status(&self) -> Result<State> {
        match self.call(Op::Status {}).await? {
            Reply::Status(s) => Ok(s),
            other => Err(unexpected(&other)),
        }
    }

    /// Capture one take; partial words come as events when `partials`.
    pub async fn listen(&self, mode: Mode, until: Until, partials: bool) -> Result<Stream> {
        self.stream(Op::Listen { mode, until, partials, max_seconds: None }).await
    }

    /// Capture one take and return its text.
    pub async fn listen_text(&self, mode: Mode, until: Until) -> Result<String> {
        match self.call(Op::Listen { mode, until, partials: false, max_seconds: None }).await? {
            Reply::Transcript { text } => Ok(text),
            other => Err(unexpected(&other)),
        }
    }

    pub async fn stop_listening(&self) -> Result<()> {
        self.call(Op::StopListening {}).await.map(|_| ())
    }

    /// Dictation into the focused window. `stop` (and a `toggle` that stops) returns what
    /// was typed.
    pub async fn dictate(&self, action: DictateAction) -> Result<Option<String>> {
        match self.call(Op::Dictate { action }).await? {
            Reply::Dictated { text } => Ok(Some(text)),
            _ => Ok(None),
        }
    }

    /// Say `text`; returns once it has played.
    pub async fn speak(&self, text: &str) -> Result<()> {
        self.call(Op::Speak { text: text.into(), voice: None, speed: None }).await.map(|_| ())
    }

    pub async fn stop_speaking(&self) -> Result<()> {
        self.call(Op::StopSpeaking {}).await.map(|_| ())
    }

    /// `state` events as Nyx Voice starts and stops listening and speaking.
    pub async fn watch(&self) -> Result<Stream> {
        self.stream(Op::Watch {}).await
    }
}

impl Stream {
    /// The next event or the result; `None` after the result or an error.
    pub async fn next(&mut self) -> Option<Result<Item>> {
        if self.done {
            return None;
        }
        loop {
            let line = match read_line(&mut self.reader).await {
                Ok(line) => line,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            };
            match line {
                ServerLine::Event { id, event } if id == self.id => return Some(Ok(Item::Event(event))),
                ServerLine::Result { id, result } if id == self.id => {
                    self.done = true;
                    return Some(Ok(Item::Done(result)));
                }
                ServerLine::Error { id, error } if id == Some(self.id) || id.is_none() => {
                    self.done = true;
                    return Some(Err(error));
                }
                _ => continue,
            }
        }
    }

    /// Ask the daemon to stop this request; the stream then ends with `cancelled`.
    pub async fn cancel(&mut self) -> Result<()> {
        if self.done {
            return Ok(());
        }
        send(&mut self.writer, &ClientLine::Request(Request { id: self.id + 1, op: Op::Cancel { request: self.id } })).await
    }
}

async fn send(writer: &mut OwnedWriteHalf, line: &ClientLine) -> Result<()> {
    writer.write_all(proto::encode(line).as_bytes()).await.map_err(lost)
}

async fn read_line(reader: &mut BufReader<OwnedReadHalf>) -> Result<ServerLine> {
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf).await.map_err(lost)?;
    if n == 0 {
        return Err(Error::new(ErrorKind::Unavailable, "Nyx Voice closed the connection"));
    }
    if buf.len() > proto::MAX_LINE {
        return Err(Error::new(ErrorKind::Internal, "reply line too long"));
    }
    serde_json::from_slice(&buf).map_err(|e| Error::new(ErrorKind::Internal, format!("unreadable reply from Nyx Voice: {e}")))
}

fn lost(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Unavailable, format!("lost the connection to Nyx Voice: {e}"))
}

fn unexpected(reply: &Reply) -> Error {
    Error::new(ErrorKind::Internal, format!("unexpected reply from Nyx Voice: {reply:?}"))
}

/// Blocking calls, for programs without a tokio runtime.
pub mod blocking {
    use super::*;

    pub fn call(client: &Client, op: Op) -> Result<Reply> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?
            .block_on(client.call(op))
    }
}
