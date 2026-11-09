//! The streaming recognizer end to end: the worker binary, the model and the client.
//! Runs real recognition, so it's ignored by default:
//! `cargo build --release -p nyx-voice-asr && cargo test -p nyx-voice-engine --test asr -- --ignored`
//! (`ASR_TEST_WAV` adds a recording with speech).
use nyx_voice_engine::asr::AsrWorker;
use nyx_voice_engine::config::Settings;

#[tokio::test]
#[ignore = "runs real speech recognition"]
async fn hears_speech_and_ignores_silence() {
    let worker = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/nyx-voice-asr");
    let Some(model) = nyx_voice_engine::asr::installed_model() else {
        eprintln!("skipped: nyx-voice models fetch asr");
        return;
    };
    assert!(worker.is_file(), "build -p nyx-voice-asr --release first");
    let mut cfg = Settings::default();
    cfg.stt.model = model.display().to_string();
    let w = AsrWorker::start_with(&cfg, &worker).await.expect("worker starts");
    let cancel = tokio_util::sync::CancellationToken::new();
    let u = w.begin().await.unwrap();
    w.feed(u, &vec![0i16; 16_000]);
    assert_eq!(w.finish(u, &cancel).await.unwrap(), "", "silence has no words");
    if let Ok(path) = std::env::var("ASR_TEST_WAV") {
        let mut r = hound::WavReader::open(path).unwrap();
        let pcm: Vec<i16> = r.samples::<i16>().map(Result::unwrap).collect();
        let mut partials = w.subscribe();
        let u = w.begin().await.unwrap();
        for chunk in pcm.chunks(512) {
            w.feed(u, chunk);
        }
        assert!(!w.finish(u, &cancel).await.unwrap().is_empty());
        let mut saw_partial = false;
        while let Ok(r) = partials.try_recv() {
            saw_partial |= matches!(r, nyx_voice_proto::asr::Reply::Partial { .. });
        }
        assert!(saw_partial, "partials while it listened");
    }
}
