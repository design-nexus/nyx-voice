# Nyx Voice protocol, API version 1

Newline-delimited JSON over the Unix socket `$XDG_RUNTIME_DIR/nyx-voice/voice.sock`
(folder `0700`, socket `0600`; `NYX_VOICE_SOCKET` overrides the path). The Rust types are
in `crates/nyx-voice-proto` (`voice`); applications should use `nyx-voice-client`.

## Connection

1. The server accepts only peers of the same user (`SO_PEERCRED`) and names them by the
   program they run (nyx-core's `nyx-peer`: Nyx Voice's own folder or
   `/etc/nyx/trusted-bin`), or `untrusted:<declared name>`.
2. The client sends `{"hello":{"api_version":1,"client":"nyx-chat"}}`; the server answers
   `{"hello":{"api_version":1,"server":"0.1.0","client":"nyx-chat"}}`.
3. Requests: `{"request":{"id":1,"op":{"type":"status"}}}`. For each the server sends zero
   or more `{"event":{"id":1,"event":{…}}}` and then one `{"result":{"id":1,"result":{…}}}`
   or `{"error":{"id":1,"error":{"kind":…,"message":…}}}`.

Closing the connection cancels its requests. A dictation started by `dictate` belongs to
the daemon and goes on until `dictate stop` (or its time limit).

## Operations

| `type` | Fields | Events | Result | Who |
|---|---|---|---|---|
| `status` | | | `status` | anyone |
| `listen` | `mode` (`dictation`, `assistant`), `until` (`silence`, `stop`), `partials`, `max_seconds` (≤ 120) | `capturing`, `partial`, `transcribing` | `transcript` | trusted |
| `stop_listening` | | | `ok` | trusted |
| `dictate` | `action` (`start`, `stop`, `toggle`) | | `ok`, or `dictated` when it stopped | trusted |
| `speak` | `text` (≤ 20,000 bytes), `voice`, `speed` (0.5 to 2) | `speaking` | `spoken` (`ms`) | anyone |
| `stop_speaking` | | | `ok` | anyone |
| `devices` | | | `devices` (`inputs`, `outputs`: `id`, `name`, `default`) | anyone |
| `watch` | | `state` on every change | | anyone |
| `cancel` | `request` | | `ok` | |

`state` / `status`: `activity` (`idle`, `recording`, `transcribing`, `speaking`), `mode`,
and `recognizer_unavailable` / `synthesizer_unavailable` with the reason when one can't
run.

Errors: `incompatible_version`, `invalid_request`, `permission_denied` (not allowed to
listen or type), `busy` (another take has the microphone), `unavailable` (the recognizer,
synthesizer or audio can't run; the message says why), `cancelled`, `internal`.

## Modes

Dictation is text for a destination: an app's field, or, through `dictate`, the focused
window. It is never interpreted as a command. Assistant mode is input for an assistant
to interpret; Nyx Voice doesn't act on it either.
