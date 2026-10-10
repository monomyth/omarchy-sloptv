#!/usr/bin/env bash
# omarchy:summary=slopTV launcher: start the player, or focus/close it if running
# Modes: toggle (default) | start | focus | quit
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
  *)
    echo "usage: open.sh [toggle|start|focus|quit]" >&2
    exit 2
    ;;
esac
