//! nyx-voiced: Nyx Voice, the desktop's speech service. Dictation into any window,
//! listening and speaking for Nyx apps, one microphone owner, nothing sent off the
//! computer. Protocol: `nyx_voice_proto::voice`.
//!
//! Raw audio is never written to disk, and transcripts are never logged.

mod config;
mod desktop;
mod server;
mod voice;

use std::time::Duration;

/// The socket systemd opened for us (`nyx-voiced.socket`), if it did.
fn activated() -> Option<tokio::net::UnixListener> {
    use std::os::fd::FromRawFd;
    let ours = std::env::var("LISTEN_PID").ok()?.parse::<u32>().ok()? == std::process::id();
    if !ours || std::env::var("LISTEN_FDS").ok()? != "1" {
        return None;
    }
    // SAFETY: systemd passes the listening socket as fd 3 (SD_LISTEN_FDS_START) and it is ours alone.
    let std = unsafe { std::os::unix::net::UnixListener::from_raw_fd(3) };
    std.set_nonblocking(true).ok()?;
    tokio::net::UnixListener::from_std(std).ok()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();
    nyx_voice_engine::paths::set_app("nyx-voice", "Nyx Voice");
    let cfg = config::load();
    let path = nyx_voice_client::socket_path();
    let listener = match activated().map(Ok).unwrap_or_else(|| server::bind(&path)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nyx-voiced: {e:#}");
            std::process::exit(1);
        }
    };
    let own_dir = voice::own_dir().unwrap_or_default();
    let voice = voice::Voice::new(cfg);
    tracing::info!(socket = %path.display(), "Nyx Voice ready");
    let idle = voice.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            idle.release_idle().await;
        }
    });
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
    tokio::select! {
        _ = server::serve(voice.clone(), listener, own_dir) => {}
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    let _ = std::fs::write(nyx_voice_engine::paths::runtime_dir().join("state"), "idle");
}
