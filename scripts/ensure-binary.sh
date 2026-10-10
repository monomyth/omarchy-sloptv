#!/usr/bin/env bash
# Prints the path of the slop-tv binary on stdout, building it on first use.
# Search order: $SLOPTV_BIN, prebuilt in the cache dir, slop-tv on PATH, then
# `cargo build --release` into ~/.cache/omarchy/sloptv/target (once).
# Writes only under $HOME. Never runs sudo and never installs packages.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/omarchy/sloptv"
TARGET_DIR="$CACHE_DIR/target"
BIN="$TARGET_DIR/release/slop-tv"

say() {
  echo "slopTV: $*" >&2
  if command -v notify-send >/dev/null 2>&1; then
    notify-send -u low "slopTV" "$*" || true
  fi
}

die() {
  echo "slopTV: $*" >&2
  if command -v notify-send >/dev/null 2>&1; then
    notify-send -u critical "slopTV" "$*" || true
  fi
  exit 127
}

if [[ -n "${SLOPTV_BIN:-}" ]]; then
  [[ -x "$SLOPTV_BIN" ]] || die "SLOPTV_BIN is set but not executable: $SLOPTV_BIN"
  printf '%s\n' "$SLOPTV_BIN"
  exit 0
fi

if [[ -x "$BIN" ]]; then
  printf '%s\n' "$BIN"
  exit 0
fi

if command -v slop-tv >/dev/null 2>&1; then
  command -v slop-tv
  exit 0
fi

command -v cargo >/dev/null 2>&1 \
  || die "cargo not found. Install Rust (Arch: sudo pacman -S rust pkgconf mpv), then open slopTV again."
command -v pkg-config >/dev/null 2>&1 \
  || die "pkg-config not found (Arch: sudo pacman -S pkgconf), then open slopTV again."
pkg-config --exists mpv 2>/dev/null \
  || die "libmpv not found (pkg-config 'mpv'). Arch: sudo pacman -S mpv, then open slopTV again."

mkdir -p "$CACHE_DIR"
say "first run: building slop-tv (cargo build --release), this takes a few minutes..."
if ! (cd "$REPO_DIR" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release --locked >"$CACHE_DIR/build.log" 2>&1); then
  die "build failed, see $CACHE_DIR/build.log"
fi
[[ -x "$BIN" ]] || die "build finished but $BIN is missing, see $CACHE_DIR/build.log"
say "build done."
printf '%s\n' "$BIN"
