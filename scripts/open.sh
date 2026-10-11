#!/usr/bin/env bash
# omarchy:summary=slopTV launcher: start the player, or focus/close it if running
# Modes: toggle (default) | start | focus | quit | fullscreen | status
#   toggle: not running -> start; running but not focused -> focus; focused -> close
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/omarchy/sloptv"
CLASS_RE='^(slop-tv)$'
MODE="${1:-toggle}"
mkdir -p "$STATE_DIR"

is_running() {
  pgrep -x slop-tv >/dev/null 2>&1
}

is_focused() {
  local c
  c="$(hyprctl activewindow -j 2>/dev/null | jq -r '.class // empty' 2>/dev/null || true)"
  [[ "$c" == "slop-tv" ]]
}

start() {
  local bin
  bin="$("$SCRIPT_DIR/ensure-binary.sh")" || exit $?
  nohup setsid "$bin" >>"$STATE_DIR/slop-tv.log" 2>&1 &
}

focus() {
  hyprctl dispatch focuswindow "class:$CLASS_RE" >/dev/null 2>&1 || true
}

quit() {
  hyprctl dispatch closewindow "class:$CLASS_RE" >/dev/null 2>&1 || pkill -x slop-tv || true
}

slop_addr() {
  hyprctl clients -j 2>/dev/null | jq -r '[.[] | select(.class=="slop-tv")][0].address // empty' 2>/dev/null || true
}

# Toggle fullscreen of the slop-tv window through Hyprland (independent of page keyboard focus).
fullscreen() {
  local a
  a="$(slop_addr)"
  if [[ -z "$a" ]]; then
    echo "slop-tv window not found (is it running?)" >&2
    return 1
  fi
  hyprctl dispatch focuswindow "address:$a" >/dev/null 2>&1 || true
  hyprctl dispatch fullscreen 0 >/dev/null 2>&1
}

# Print the window's current state (for debugging fullscreen problems).
status() {
  hyprctl clients -j | jq '[.[] | select(.class=="slop-tv") | {address, class, title, fullscreen, fullscreenClient, floating, workspace: .workspace.name, at, size}]'
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
