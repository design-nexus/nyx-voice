//! nyx-voice: Nyx Voice from the command line and key bindings.

use anyhow::{bail, Result};
use nyx_voice_client::proto::{DictateAction, Event, Mode, Op, Reply, Until};
use nyx_voice_client::{Client, Item};

const USAGE: &str = "nyx-voice — Nyx Voice (dictation, listening, speaking)

  dictate start|stop|toggle   dictation into the focused window (push to talk: start on
                              key down, stop on key up)
  listen [--assistant] [--hold] [--partials]
                              one take, its text printed (--hold: until `stop`)
  stop                        end the current take now
  say TEXT…                   speak the text
  quiet                       stop speaking
  status [--json]             what Nyx Voice is doing
  watch                       state changes as they happen (JSON lines)
  devices                     microphones and outputs
  models fetch asr            download the streaming speech model (about 650 MB)
";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("nyx-voice: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let voice = Client::new("nyx-voice");
    match a.as_slice() {
        ["dictate", action] => {
            let action = match *action {
                "start" => DictateAction::Start,
                "stop" => DictateAction::Stop,
                "toggle" => DictateAction::Toggle,
                _ => bail!("dictate start|stop|toggle"),
            };
            if let Some(text) = voice.dictate(action).await? {
                if !text.is_empty() {
                    println!("{text}");
                }
            }
        }
        ["listen", flags @ ..] => {
            let mode = if flags.contains(&"--assistant") { Mode::Assistant } else { Mode::Dictation };
            let until = if flags.contains(&"--hold") { Until::Stop } else { Until::Silence };
            let mut s = voice.listen(mode, until, flags.contains(&"--partials")).await?;
            while let Some(item) = s.next().await {
                match item? {
                    Item::Event(Event::Partial { text }) => eprintln!("… {text}"),
                    Item::Done(Reply::Transcript { text }) => println!("{text}"),
                    _ => {}
                }
            }
        }
        ["stop"] => voice.stop_listening().await?,
        ["say", words @ ..] if !words.is_empty() => voice.speak(&words.join(" ")).await?,
        ["quiet"] => voice.stop_speaking().await?,
        ["status", rest @ ..] => {
            let s = voice.status().await?;
            if rest.contains(&"--json") {
                println!("{}", serde_json::to_string(&s)?);
            } else {
                println!("{}{}", s.activity, s.mode.map(|m| format!(" ({m:?})").to_lowercase()).unwrap_or_default());
                if let Some(why) = s.recognizer_unavailable {
                    println!("speech recognition: {why}");
                }
                if let Some(why) = s.synthesizer_unavailable {
                    println!("speech synthesis: {why}");
                }
            }
        }
        ["watch"] => {
            let mut s = voice.watch().await?;
            while let Some(item) = s.next().await {
                if let Item::Event(Event::State(state)) = item? {
                    println!("{}", serde_json::to_string(&state)?);
                }
            }
        }
        ["devices"] => {
            if let Reply::Devices { inputs, outputs } = voice.call(Op::Devices {}).await? {
                for (title, list) in [("Microphones", inputs), ("Outputs", outputs)] {
                    println!("{title}:");
                    for d in list {
                        println!("  {} {}  ({})", if d.default { "*" } else { " " }, d.name, d.id);
                    }
                }
            }
        }
        ["models", "fetch", "asr"] => {
            nyx_voice_engine::paths::set_app("nyx-voice", "Nyx Voice");
            if let Some(found) = nyx_voice_engine::asr::installed_model() {
                println!("Already installed: {}", found.display());
            } else {
                println!("Downloading the streaming speech model (about 650 MB; NVIDIA Open Model License)…");
                let path = tokio::task::spawn_blocking(nyx_voice_engine::asr::fetch_model).await??;
                println!("Installed {}", path.display());
            }
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}
