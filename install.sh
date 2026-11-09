#!/usr/bin/env bash
# Install Nyx Voice: nyx-voiced (the speech service), the nyx-voice command, the
# speech recognizer worker, and the user units that start it on first use.
#
#   ./install.sh          build this checkout and install it
#
# Nothing is downloaded at install: speech models come with `nyx-voice models fetch asr`,
# or are found where the Assistant already keeps them. NYX_VOICE_NO_ENABLE=1 installs
# without enabling the socket.
set -euo pipefail

REPO="design-nexus/nyx-voice"
say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m==>\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m==>\033[0m %s\n' "$*" >&2; exit 1; }

case ${1:-voice} in
  voice | all | --*) ;;
  -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) die "This repository installs only voice (Nyx Voice)." ;;
esac

here=""
[[ -n ${BASH_SOURCE[0]:-} && -f ${BASH_SOURCE[0]} ]] && here=$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [[ -n $here && -f $here/Cargo.toml && -d $here/crates ]]; then
  src=$here
else
  say "Fetching the source"
  git clone -q --depth 1 "https://github.com/$REPO.git" "$work/src"
  src="$work/src"
fi

if command -v pacman >/dev/null; then
  need=()
  # playerctl is optional: without it dictation doesn't pause media.
  for p in pipewire libpulse wtype wl-clipboard; do pacman -Qq "$p" >/dev/null 2>&1 || need+=("$p"); done
  if ((${#need[@]})); then
    say "Installing ${need[*]}"
    sudo pacman -S --needed --noconfirm "${need[@]}"
  fi
fi

target=${CARGO_TARGET_DIR:-$src/target}
command -v cargo >/dev/null || die "Rust (cargo) is needed to build Nyx Voice."
say "Building Nyx Voice"
(cd "$src" && cargo build --release --locked -p nyx-voiced -p nyx-voice -p nyx-voice-asr)

bin="$HOME/.local/bin"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$bin" "$units"
say "Installing nyx-voiced, nyx-voice and nyx-voice-asr"
for b in nyx-voiced nyx-voice nyx-voice-asr; do
  install -m 755 "$target/release/$b" "$bin/$b"
done
install -m 644 "$src/data/nyx-voiced.service" "$src/data/nyx-voiced.socket" "$units/"
systemctl --user daemon-reload 2>/dev/null || true
if [[ ${NYX_VOICE_NO_ENABLE:-0} != 1 ]]; then
  systemctl --user enable --now nyx-voiced.socket >/dev/null 2>&1 || warn "Couldn't enable nyx-voiced.socket; run: systemctl --user enable --now nyx-voiced.socket"
fi
# A running daemon keeps the old build; it starts again on the next request.
if systemctl --user is-active -q nyx-voiced.service 2>/dev/null; then
  systemctl --user stop nyx-voiced.service || true
fi
say "Done. Try: nyx-voice status"
