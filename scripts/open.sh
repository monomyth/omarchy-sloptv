#!/usr/bin/env bash
# omarchy:summary=slopTV launcher: start the player, or focus/close it if running
# Modes: toggle (default) | start | focus | quit | fullscreen | status
#   toggle: not running -> start; running but not focused -> focus; focused -> close
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/omarchy/sloptv"
MODE="${1:-toggle}"
mkdir -p "$STATE_DIR"

is_running() {
  pgrep -x slop-tv >/dev/null 2>&1
}

is_focused() {
  local c
  c="$(hyprctl activewindow -j 2>/dev/null | jq -r '.class // empty' 2>/dev/null || true)"
  [[ "$c" == *slop-tv* ]]
}

start() {
  local bin
  bin="$("$SCRIPT_DIR/ensure-binary.sh")" || exit $?
  nohup setsid "$bin" >>"$STATE_DIR/slop-tv.log" 2>&1 &
}

# On Wayland Chromium derives the app-id from the --app URL/profile, so --class=slop-tv is ignored and the
# class is e.g. chrome-__tmp_slop-tv-launch-<controller pid>.html-Default. Match any class containing
# "slop-tv", preferring the one that carries the running controller's pid.
slop_addr() {
  local pid
  pid="$(pgrep -nx slop-tv 2>/dev/null || true)"
  hyprctl clients -j 2>/dev/null | jq -r --arg p "slop-tv-launch-${pid}." \
    '[.[] | select(.class | test("slop-tv"))] | ((map(select(.class | contains($p)))[0]) // .[0]) | .address // empty' 2>/dev/null || true
}

# Hyprland with a Lua config (0.55+) rejects the old `hyprctl dispatch <name> <args>` text form, so try
# `hyprctl eval "hl.dispatch(...)"` first and fall back to the classic dispatcher.
hl_try() { # hl_try '<lua expr>' <classic dispatch args...>
  local lua="$1"; shift
  [[ "$(hyprctl eval "hl.dispatch($lua)" 2>/dev/null)" == "ok" ]] && return 0
  hyprctl dispatch "$@" >/dev/null 2>&1
}

focus() {
  local a
  a="$(slop_addr)"
  [[ -n "$a" ]] && hl_try "hl.dsp.focus({ window = 'address:$a' })" focuswindow "address:$a" || true
}

quit() {
  local a
  a="$(slop_addr)"
  if [[ -n "$a" ]] && hl_try "hl.dsp.window.close({ window = 'address:$a' })" closewindow "address:$a"; then :; else pkill -x slop-tv || true; fi
}


# Toggle fullscreen of the slop-tv window through Hyprland (independent of page keyboard focus).
fullscreen() {
  local a fs n
  a="$(slop_addr)"
  if [[ -z "$a" ]]; then
    echo "slop-tv window not found (is it running?)" >&2
    return 1
  fi
  fs="$(hyprctl clients -j | jq -r --arg a "$a" '.[] | select(.address == $a) | .fullscreen')"
  n=2; [[ "${fs:-0}" != "0" ]] && n=0
  hl_try "hl.dsp.focus({ window = 'address:$a' })" focuswindow "address:$a" || true
  hl_try "hl.dsp.window.fullscreen_state({ window = 'address:$a', action = 'set', internal = $n, client = $n })" fullscreen 0
}

# Print the window's current state (for debugging fullscreen problems).
status() {
  hyprctl clients -j | jq '[.[] | select(.class | test("slop-tv")) | {address, class, title, fullscreen, fullscreenClient, floating, workspace: .workspace.name, at, size}]'
}

case "$MODE" in
  toggle|"")
    if ! is_running; then
      start
    elif is_focused; then
      quit
    else
      focus
    fi
    ;;
  start)
    if is_running; then focus; else start; fi
    ;;
  focus)
    if is_running; then focus; else start; fi
    ;;
  quit)
    quit
    ;;
  fullscreen)
    if ! is_running; then start; else fullscreen; fi
    ;;
  status)
    status
    ;;
  *)
    echo "usage: open.sh [toggle|start|focus|quit|fullscreen|status]" >&2
    exit 2
    ;;
esac
