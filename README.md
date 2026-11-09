# Nyx Voice

The Nyx desktop's speech service: dictation into any window, listening and speaking for
Nyx apps, on this computer only. It replaces VoxType on NyxOS.

- **Dictation** (`nyx-voice dictate start|stop|toggle`): push to talk or toggle; the
  text is typed into the focused window (or put on the clipboard), never read as a
  command. Playing media pauses while you talk when `playerctl` is installed.
- **Listening** for apps: one take, in dictation or assistant mode, ending at silence or
  when the app says, with partial words as you speak.
- **Speaking** for apps: Kokoro voices, or a cloned voice through Chatterbox when
  installed.
- **One microphone owner**: one take at a time; a second gets `busy`.
- **Private**: raw audio is never written to disk and transcripts are never logged.
  Only Nyx's own programs (or ones you list) may listen or type.

The engine (`nyx-voice-engine`) is also a library: the Assistant links it directly so
its barge-in and listening-while-speaking keep their latency, while everything else
talks to `nyx-voiced`.

## Install

```sh
./install.sh
```

It builds this checkout and installs `nyx-voiced`, `nyx-voice` and `nyx-voice-asr` to
`~/.local/bin`, with a user socket that starts Nyx Voice on first use. Nothing is
downloaded at install. Speech models already downloaded by the Assistant are used where
they are; otherwise:

```sh
nyx-voice models fetch asr     # the streaming recognizer, about 650 MB
```

Kokoro's voices download on first speech (about 300 MB) unless the Assistant already
has them.

## Use

```
nyx-voice dictate start|stop|toggle
nyx-voice listen [--assistant] [--hold] [--partials]
nyx-voice stop | say TEXT… | quiet
nyx-voice status [--json] | watch | devices
```

Bind push to talk in Hyprland:

```lua
o.bind_hold("F9", "Start dictation", "nyx-voice dictate start", "Stop dictation", "nyx-voice dictate stop")
o.bind("SUPER + CTRL + X", "Toggle dictation", "nyx-voice dictate toggle")
```

`$XDG_RUNTIME_DIR/nyx-voice/state` holds `idle`, `recording`, `transcribing` or
`speaking`, as VoxType's state file does, for bars and scripts.

## Settings

`~/.config/nyx-voice/config.toml` takes the Assistant's `[audio]` (microphone, output),
`[vad]`, `[stt]` and `[tts]` sections, and without a file of its own Nyx Voice reads
them from the Assistant's. Its own sections:

```toml
[dictation]
output = "type"               # or "clipboard"
fallback_to_clipboard = true
type_delay_ms = 1
pause_media = true
max_seconds = 60

[permissions]
microphone = []               # more programs that may listen, by executable name
```

See `docs/ARCHITECTURE.md` and `docs/PROTOCOL.md`; `docs/VOXTYPE.md` compares Nyx Voice
with what VoxType did.
