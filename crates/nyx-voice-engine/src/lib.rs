//! Nyx Voice's speech engine: what listening and speaking need, in one library.
//!
//! - [`AudioEngine`]: PipeWire capture (16 kHz) and playback (24 kHz), echo cancelling
//!   for listening while speaking, other playback lowered while speech plays.
//! - [`StreamingVad`] (Silero, or an energy gate): where speech starts and ends.
//! - [`asr::Stt`]: streaming recognition in the `nyx-voice-asr` worker (Nemotron through
//!   sherpa-onnx), partial words while someone speaks.
//! - [`tts::KokoroProvider`]: Kokoro synthesis, or a cloned voice through Chatterbox.
//!
//! `nyx-voiced` serves it to every Nyx app; the Assistant links it directly so its
//! barge-in and duplex listening keep their latency. Call [`paths::set_app`] first.

pub mod asr;
pub mod config;
pub mod paths;
pub mod speech_text;
pub mod tts;
mod engine;
mod duck;
mod playq;
mod silero;
mod vad;

pub use engine::{aec_barge_threshold, record_until, record_until_vad, AudioEngine, UNTAGGED};
pub use playq::{PlayQueue, PlayedTo};
pub use silero::{Silero, SileroState};
pub use vad::{EnergyVad, StreamingVad, VadDecision, VadGate};

/// Linear resample Kokoro 24 kHz f32 mono → the recognizer's 16 kHz i16 mono.
pub fn f32_24k_to_i16_16k(pcm: &[f32]) -> Vec<i16> {
    if pcm.is_empty() {
        return Vec::new();
    }
    let out_len = (pcm.len() as u64 * 16_000 / 24_000) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 * 24_000.0 / 16_000.0;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(pcm.len() - 1);
        let frac = src - i0 as f32;
        let s = pcm[i0] * (1.0 - frac) + pcm[i1] * frac;
        let s = s.clamp(-1.0, 1.0);
        out.push((s * 32767.0) as i16);
    }
    out
}
