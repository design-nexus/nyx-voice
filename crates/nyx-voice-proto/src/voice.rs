//! The `nyx-voiced` protocol, API version 1.
//!
//! Newline-delimited JSON over `$XDG_RUNTIME_DIR/nyx-voice/voice.sock` (`NYX_VOICE_SOCKET`
//! overrides it). The client sends [`ClientLine::Hello`], then requests; the server
//! answers each request with zero or more events and then one result or one error, as
//! Nyx AI Core does. Closing the connection cancels its requests (a dictation the
//! daemon runs for the keyboard goes on).
//!
//! Only the same user may connect. Listening (the microphone) and dictation (typing
//! into the focused window) need a trusted program: one installed beside `nyx-voiced`,
//! one listed in `/etc/nyx/trusted-bin`, or one named in `[permissions] microphone`.

use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;
pub const SOCKET_ENV: &str = "NYX_VOICE_SOCKET";
pub const SOCKET_NAME: &str = "voice.sock";
/// Longest line either side accepts.
pub const MAX_LINE: usize = 1 << 20;
/// Longest text `speak` takes.
pub const MAX_SPEAK: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClientLine {
    Hello(Hello),
    Request(Request),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub api_version: u32,
    /// What the client calls itself (`nyx-assistant`, `nyx-chat`).
    pub client: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub op: Op,
}

/// Who a take is for. Dictation is text for a destination, never a command; assistant
/// mode is input for an assistant to interpret.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Dictation,
    Assistant,
}

/// When a take ends.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Until {
    /// When the speaker stops (voice activity).
    #[default]
    Silence,
    /// When `stop_listening` comes (push to talk).
    Stop,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DictateAction {
    Start,
    Stop,
    Toggle,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    Status {},
    /// Capture one take and return its text to this client.
    Listen {
        #[serde(default)]
        mode: Mode,
        #[serde(default)]
        until: Until,
        /// Send `partial` events while the take runs.
        #[serde(default)]
        partials: bool,
        /// At most this long (default and limit 120 s).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_seconds: Option<u32>,
    },
    /// End the current take now (a push-to-talk key released).
    StopListening {},
    /// Dictation to the focused window: the daemon captures, transcribes and types.
    Dictate { action: DictateAction },
    /// Say `text`; the result comes when it has been played.
    Speak {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        voice: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<f32>,
    },
    StopSpeaking {},
    Devices {},
    /// `state` events whenever capture or speech starts or stops, until the connection closes.
    Watch {},
    /// Cancel request `request` on this connection.
    Cancel { request: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ServerLine {
    Hello(Welcome),
    Event {
        id: u64,
        event: Event,
    },
    Result {
        id: u64,
        result: Reply,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        error: Error,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Welcome {
    pub api_version: u32,
    pub server: String,
    /// The name the daemon gave this connection: the program, or `untrusted:<name>`.
    pub client: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The microphone is open for this take.
    Capturing,
    /// Words so far.
    Partial { text: String },
    /// The take ended; its text is being finished.
    Transcribing,
    Speaking,
    State(State),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Status(State),
    /// A take's text (empty when nothing was said).
    Transcript { text: String },
    /// What dictation typed.
    Dictated { text: String },
    Spoken { ms: u64 },
    Devices { inputs: Vec<Device>, outputs: Vec<Device> },
}

/// What Nyx Voice is doing, and what it can do.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// `idle`, `recording`, `transcribing` or `speaking` (the values VoxType's state file has).
    pub activity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// Why speech recognition can't run, when it can't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognizer_unavailable: Option<String>,
    /// Why speech synthesis can't run, when it can't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesizer_unavailable: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    /// The PipeWire/Pulse name `[audio] source`/`sink` take.
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    IncompatibleVersion,
    InvalidRequest,
    /// Not a program allowed to use the microphone or type.
    PermissionDenied,
    /// The microphone is already in use by another take.
    Busy,
    /// The recognizer, synthesizer or audio isn't available; the message says why.
    Unavailable,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error { kind, message: message.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// One line of JSON with its newline.
pub fn encode<T: Serialize>(line: &T) -> String {
    let mut s = serde_json::to_string(line).expect("serializable");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let req = ClientLine::Request(Request { id: 3, op: Op::Listen { mode: Mode::Assistant, until: Until::Stop, partials: true, max_seconds: None } });
        let line = encode(&req);
        assert_eq!(line, "{\"request\":{\"id\":3,\"op\":{\"type\":\"listen\",\"mode\":\"assistant\",\"until\":\"stop\",\"partials\":true}}}\n");
        assert_eq!(serde_json::from_str::<ClientLine>(line.trim()).unwrap(), req);
        let dictate: ClientLine = serde_json::from_str(r#"{"request":{"id":1,"op":{"type":"dictate","action":"toggle"}}}"#).unwrap();
        assert!(matches!(dictate, ClientLine::Request(Request { op: Op::Dictate { action: DictateAction::Toggle }, .. })));
        assert!(serde_json::from_str::<ClientLine>(r#"{"request":{"id":1,"op":{"type":"status","extra":1}}}"#).is_err());
        let ev = encode(&ServerLine::Event { id: 3, event: Event::Partial { text: "hel".into() } });
        assert_eq!(ev, "{\"event\":{\"id\":3,\"event\":{\"type\":\"partial\",\"text\":\"hel\"}}}\n");
    }
}
