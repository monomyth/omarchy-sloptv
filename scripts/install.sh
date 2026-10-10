#!/usr/bin/env bash
# One-step setup after `omarchy plugin add`: build the binary now (so the first
# keypress never waits on a compile), then add the Super+Shift+Ctrl+T bind if free.
# Never runs sudo and never installs packages; it prints what is missing instead.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "slopTV: building (first time takes a few minutes)..."
BIN="$(bash "$SCRIPT_DIR/ensure-binary.sh")"
echo "slopTV: binary ready at $BIN"

status=0
bash "$SCRIPT_DIR/install-bind.sh" || status=$?
if [[ "$status" -eq 2 ]]; then
  echo "slopTV: built, but the chord is taken. Pick another from bindings.hypr.lua.example." >&2
  exit 2
elif [[ "$status" -ne 0 ]]; then
  exit "$status"
fi
cat <<EOF
slopTV: done. How to run:
  Keybind:   Super+Shift+Ctrl+T starts, focuses or closes the player
  Bar icon:  on the right of the bar if the widget is enabled (click = same toggle)
  In app:    ? help, l/h next/prev, o open on X, q quit (click the bottom bar first so it has the keyboard)
  First run: a one-time privacy notice explains that X's own embedded player is used
  Remove:    bash "$SCRIPT_DIR/uninstall.sh"
EOF
