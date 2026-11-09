# Nyx Voice

The desktop's speech service: `nyx-voiced` (daemon), `nyx-voice` (CLI), `nyx-voice-asr`
(recognizer worker), `nyx-voice-engine` (library the Assistant links), `nyx-voice-proto`
and `nyx-voice-client`. See `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`, `docs/VOXTYPE.md`.

Commands: `cargo build --release --locked`; `cargo test --locked`; `./install.sh`

- Never write raw audio to disk or log transcripts.
- Listening and typing need a trusted program (`nyx-peer`); keep it that way.
- Speech never runs in AI Core's llama.cpp worker; language models never run here.
- The Assistant's PipeWire names and folders come from `paths::set_app`; don't hard-code
  a program's name in the engine.
- Do not download models or run real speech for routine verification; tests use no audio.

Common guidance: https://github.com/design-nexus/nyx/blob/main/docs/AGENTS_COMMON.md
Shared dependencies use immutable GitHub revisions.
