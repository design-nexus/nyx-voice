//! Dictation's output and the desktop around it: typing into the focused window (or the
//! clipboard), pausing media while recording, and the audio devices there are.

use crate::config::DictationCfg;
use anyhow::{bail, Context, Result};
use nyx_voice_proto::voice::Device;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Type `text` into the focused window, or put it on the clipboard.
pub async fn deliver(cfg: &DictationCfg, text: &str) -> Result<()> {
    if cfg.output != "clipboard" {
        let typed = Command::new("wtype")
            .args(["-d", &cfg.type_delay_ms.to_string(), "--", text])
            .stdin(Stdio::null())
            .status()
            .await;
        match typed {
            Ok(s) if s.success() => return Ok(()),
            _ if !cfg.fallback_to_clipboard => bail!("couldn't type the text (wtype)"),
            _ => {}
        }
    }
    let mut child = Command::new("wl-copy").stdin(Stdio::piped()).spawn().context("wl-copy")?;
    child.stdin.take().context("wl-copy input")?.write_all(text.as_bytes()).await?;
    if !child.wait().await?.success() {
        bail!("couldn't put the text on the clipboard");
    }
    Ok(())
}

/// Pause every player that's playing; returns them, to resume.
pub async fn pause_media() -> Vec<String> {
    let Ok(out) = Command::new("playerctl").arg("-l").output().await else { return Vec::new() };
    let mut paused = Vec::new();
    for player in String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|p| !p.is_empty()) {
        let status = Command::new("playerctl").args(["-p", player, "status"]).output().await;
        if status.is_ok_and(|s| String::from_utf8_lossy(&s.stdout).trim() == "Playing")
            && Command::new("playerctl").args(["-p", player, "pause"]).status().await.is_ok_and(|s| s.success())
        {
            paused.push(player.to_string());
        }
    }
    paused
}

pub async fn resume_media(players: &[String]) {
    for player in players {
        let _ = Command::new("playerctl").args(["-p", player, "play"]).status().await;
    }
}

/// Microphones and outputs as PipeWire's Pulse server lists them (no monitors).
pub async fn devices() -> Result<(Vec<Device>, Vec<Device>)> {
    async fn list(kind: &str, default_cmd: &str) -> Result<Vec<Device>> {
        let out = Command::new("pactl").args(["--format=json", "list", kind]).output().await.context("pactl")?;
        if !out.status.success() {
            bail!("pactl couldn't list {kind}");
        }
        let default = Command::new("pactl").arg(default_cmd).output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let items: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)?;
        Ok(items
            .iter()
            .filter_map(|d| {
                let id = d["name"].as_str()?.to_string();
                if id.ends_with(".monitor") {
                    return None;
                }
                let name = d["description"].as_str().unwrap_or(&id).to_string();
                Some(Device { default: id == default, id, name })
            })
            .collect())
    }
    Ok((list("sources", "get-default-source").await?, list("sinks", "get-default-sink").await?))
}
