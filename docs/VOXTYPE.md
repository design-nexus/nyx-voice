# VoxType and Nyx Voice

What VoxType did on Nyx machines, and where it is in Nyx Voice. ✓ same, ◐ different,
✗ missing.

| VoxType | Who used it | Nyx Voice | |
|---|---|---|---|
| Push to talk (F9 hold, Right Alt hold) and toggle (Super+Ctrl+X) | Omarchy's dictation | `nyx-voice dictate start/stop/toggle`, bound by NyxOS | ✓ |
| Whisper `base.en` (dictation) | Omarchy's dictation | Nemotron streaming 0.6B, the Assistant's recognizer (English) | ◐ |
| Parakeet TDT 0.6B v3 (Assistant fallback) | Assistant | No fallback: the streaming worker is the recognizer; a missing model says how to fetch it | ◐ |
| Typed output, 1 ms per key | Omarchy's dictation | `[dictation] output = "type"`, `type_delay_ms` | ✓ |
| Clipboard when typing fails | Omarchy's dictation | `fallback_to_clipboard` | ✓ |
| Pause media while recording | Omarchy's dictation | `pause_media` (with `playerctl`) | ✓ |
| 60 s cap | Omarchy's dictation | `max_seconds` (5 to 300) | ✓ |
| State file (`idle`, `recording`, `transcribing`) | bars | `$XDG_RUNTIME_DIR/nyx-voice/state`, plus `speaking` | ✓ |
| Notifications on start, stop, transcription (off here) | | `watch` and the state file; no notifications | ◐ |
| Languages other than English, translation | | English only | ✗ |
| A warm daemon | both | `nyx-voiced`, started by its socket, engines unloaded when idle | ✓ |

## Where each stays

- **NyxOS**: Nyx Voice replaces VoxType. F9, Right Alt and Super+Ctrl+X run
  `nyx-voice dictate …`; VoxType isn't installed.
- **Omarchy (Ken's laptop)**: Omarchy's dictation keeps VoxType, as decided on
  2026-11-08. Nyx Voice is installed beside it for the Assistant and apps.
- **The Assistant**: no VoxType anywhere; it links Nyx Voice's engine.

Recognizing languages other than English would need a second sherpa-onnx model; that is
a separate choice, not part of replacing VoxType.
