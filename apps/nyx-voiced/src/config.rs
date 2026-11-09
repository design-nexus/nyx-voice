//! `~/.config/nyx-voice/config.toml`: the speech sections the Assistant's file has
//! (`[audio]`, `[vad]`, `[stt]`, `[tts]`), plus dictation and who may use the microphone.
//! Without a file of its own, Nyx Voice takes the Assistant's speech sections, so the
//! microphone and voice chosen there carry over.

use nyx_voice_engine::config::Settings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    #[serde(flatten)]
    pub speech: Settings,
    pub dictation: DictationCfg,
    pub permissions: PermissionsCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationCfg {
    /// `type` into the focused window, or `clipboard`.
    pub output: String,
    /// When typing fails, put the text on the clipboard instead.
    pub fallback_to_clipboard: bool,
    /// Milliseconds between typed keys.
    pub type_delay_ms: u32,
    /// Pause playing media while recording, and resume it after.
    pub pause_media: bool,
    /// The longest take.
    pub max_seconds: u32,
}

impl Default for DictationCfg {
    fn default() -> Self {
        Self { output: "type".into(), fallback_to_clipboard: true, type_delay_ms: 1, pause_media: true, max_seconds: 60 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsCfg {
    /// Programs (by executable name) allowed to listen besides Nyx's own.
    pub microphone: Vec<String>,
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join("nyx-voice/config.toml")
}

fn assistant_config() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join("nyx-assistant/config.toml")
}

/// The daemon's settings. It never listens while speaking, so it loads no echo canceller.
pub fn load() -> VoiceConfig {
    let read = |path: PathBuf| -> Option<VoiceConfig> {
        let text = std::fs::read_to_string(&path).ok()?;
        match toml::from_str::<VoiceConfig>(&text) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "unreadable settings; using defaults");
                None
            }
        }
    };
    let mut cfg = read(config_path())
        .or_else(|| read(assistant_config()).map(|a| VoiceConfig { speech: a.speech, ..Default::default() }))
        .unwrap_or_default();
    cfg.speech.barge_in.aec = false;
    cfg.dictation.max_seconds = cfg.dictation.max_seconds.clamp(5, 300);
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_assistant_s_speech_sections() {
        let text = "[assistant]\npersona_id = 'x'\n[audio]\nsource = 'alsa_input.usb-mic'\n[stt]\nprovider = 'voxtype'\nvoxtype_bin = 'voxtype'\n[tts]\nvoice = 'af_bella'\n";
        let c: VoiceConfig = toml::from_str(text).unwrap();
        assert_eq!(c.speech.audio.source, "alsa_input.usb-mic");
        assert_eq!(c.speech.tts.voice, "af_bella");
        assert!(c.speech.stt.streaming());
        assert!(c.dictation.pause_media);
    }
}
