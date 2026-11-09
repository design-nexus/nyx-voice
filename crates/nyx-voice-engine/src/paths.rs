//! Where the engine keeps models, caches and runtime files. The program using it says
//! whose folders they are ([`set_app`]): the Assistant keeps `nyx-assistant`'s, as it
//! always has; `nyx-voiced` uses `nyx-voice`'s. Models another Nyx program already
//! downloaded are found there too, so nothing downloads twice.

use std::path::PathBuf;
use std::sync::OnceLock;

struct App {
    slug: &'static str,
    label: &'static str,
}

static APP: OnceLock<App> = OnceLock::new();

/// Name the program, once, before the engine starts: `("nyx-assistant", "Assistant")`.
/// The label names its PipeWire streams.
pub fn set_app(slug: &'static str, label: &'static str) {
    let _ = APP.set(App { slug, label });
}

fn app() -> &'static App {
    APP.get_or_init(|| App { slug: "nyx-voice", label: "Nyx Voice" })
}

/// The program's PipeWire name ("Assistant", "Nyx Voice").
pub fn label() -> &'static str {
    app().label
}

/// The short name in PipeWire objects: "assistant" for `nyx-assistant`, "voice" for
/// `nyx-voice` (the Assistant's echo canceller has always been `assistant-echo-cancel`).
pub fn node_slug() -> &'static str {
    short(app().slug)
}

fn short(slug: &str) -> &str {
    slug.strip_prefix("nyx-").unwrap_or(slug)
}

pub fn xdg_runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| cache_dir().join("runtime"))
}

pub fn runtime_dir() -> PathBuf {
    xdg_runtime().join(app().slug)
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".").join(".local/share")).join(app().slug)
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".").join(".cache")).join(app().slug)
}

const PROGRAMS: [&str; 2] = ["nyx-voice", "nyx-assistant"];

/// Model folders to look in: this program's first, then the other Nyx speech programs'.
pub fn model_dirs() -> Vec<PathBuf> {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".").join(".local/share"));
    let mut out = vec![models_dir()];
    out.extend(PROGRAMS.iter().map(|p| base.join(p).join("models")).filter(|d| *d != models_dir()));
    out
}

/// A cache folder (`loqui` for Kokoro, `silero`) that already has files, this program's
/// first; else this program's own, to fill.
pub fn shared_cache(name: &str) -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".").join(".cache"));
    let own = cache_dir().join(name);
    std::iter::once(own.clone())
        .chain(PROGRAMS.iter().map(|p| base.join(p).join(name)))
        .find(|d| std::fs::read_dir(d).is_ok_and(|mut e| e.next().is_some()))
        .unwrap_or(own)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_assistant_keeps_its_pipewire_names() {
        assert_eq!(super::short("nyx-assistant"), "assistant");
        assert_eq!(super::short("nyx-voice"), "voice");
        assert!(super::model_dirs().iter().any(|d| d.ends_with("nyx-assistant/models")));
        assert!(super::model_dirs().iter().any(|d| d.ends_with("nyx-voice/models")));
    }
}
