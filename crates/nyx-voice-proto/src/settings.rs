//! The speech settings Nyx Voice's engine takes. They keep the names and shapes the
//! Assistant's `config.toml` has always used for `[audio]`, `[vad]`, `[stt]`, `[tts]` and
//! `[barge_in]`, so the Assistant reads them from its file as before and `nyx-voiced`
//! reads the same sections from its own.

use serde::{Deserialize, Serialize};

/// Everything the engine needs, in one value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub audio: AudioCfg,
    pub vad: VadCfg,
    pub stt: SttCfg,
    pub tts: TtsCfg,
    pub barge_in: BargeInCfg,
}

/// Speech recognition: the streaming Nemotron worker (sherpa-onnx), which hears the
/// echo-cancelled stream and gives partial words.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttCfg {
    /// "streaming", the one recognizer. Older files may still say "voxtype"; it's read as
    /// "streaming".
    pub provider: String,
    /// The streaming model's directory, or "auto" for the one `nyx-voice models fetch asr`
    /// installs.
    pub model: String,
    pub threads: u32,
    /// "cpu" or "cuda". The int8 model is no faster on the GPU here.
    pub device: String,
}

impl SttCfg {
    pub fn streaming(&self) -> bool {
        true
    }
}

impl Default for SttCfg {
    fn default() -> Self {
        Self { provider: "streaming".into(), model: "auto".into(), threads: 2, device: "cpu".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsCfg {
    pub provider: String,
    pub voice: String,
    /// Installed Kokoro voice applied to every persona while set.
    pub voice_override: String,
    pub speed: f32,
    pub device: String,
    pub variant: String,
    pub threads: Option<usize>,
    pub force_cuda: bool,
}

impl Default for TtsCfg {
    fn default() -> Self {
        Self {
            provider: "kokoro".into(),
            voice: "af_heart".into(),
            voice_override: String::new(),
            speed: 1.0,
            device: "cuda".into(),
            variant: "fp32".into(),
            threads: None,
            force_cuda: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VadCfg {
    pub enabled: bool,
    pub backend: String,
    pub threshold: f32,
    pub energy_threshold: f32,
    pub hangover_ms: u64,
    #[serde(default = "default_vad_confirm")]
    pub start_confirm_frames: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_silero_url")]
    pub url: String,
    /// The model's SHA-256; a download that doesn't match is refused. Empty skips the
    /// check (for a `url` of your own).
    #[serde(default = "default_silero_sha256")]
    pub sha256: String,
}

fn default_vad_confirm() -> u32 {
    2
}

impl Default for VadCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "silero".into(),
            threshold: 0.5,
            energy_threshold: 0.012,
            hangover_ms: 400,
            start_confirm_frames: 2,
            model: String::new(),
            url: default_silero_url(),
            sha256: default_silero_sha256(),
        }
    }
}

/// Silero VAD v6.2.3, at its commit (a branch or tag could change under it).
pub fn default_silero_url() -> String {
    "https://raw.githubusercontent.com/snakers4/silero-vad/5cd7945676eb32225748052e2e6a0580e4686a08/src/silero_vad/data/silero_vad.onnx".into()
}

pub fn default_silero_sha256() -> String {
    "1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3".into()
}

/// The address older settings saved, from the moving `master` branch: the pinned one
/// stands in for it.
pub const OLD_SILERO_URL: &str = "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx";

impl VadCfg {
    pub fn wants_silero(&self) -> bool {
        self.enabled && self.backend.trim().eq_ignore_ascii_case("silero")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioCfg {
    pub capture_rate: u32,
    pub playback_rate: u32,
    pub source: String,
    pub sink: String,
    pub duck_external: bool,
    pub duck_gain: f32,
}

impl Default for AudioCfg {
    fn default() -> Self {
        Self {
            capture_rate: 16_000,
            playback_rate: 24_000,
            source: "default".into(),
            sink: "default".into(),
            duck_external: true,
            duck_gain: 0.35,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BargeInCfg {
    pub enabled: bool,
    pub grace_ms: u64,
    pub threshold: f32,
    pub hangover_ms: u64,
    pub aec: bool,
}

impl Default for BargeInCfg {
    fn default() -> Self {
        Self { enabled: true, grace_ms: 500, threshold: 0.08, hangover_ms: 160, aec: true }
    }
}
