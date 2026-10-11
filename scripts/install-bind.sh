#!/usr/bin/env bash
# Opt-in: add the Super+Shift+Ctrl+T slopTV bind to ~/.config/hypr/bindings.lua if the chord is free.
# Exit codes: 0 added or already present, 1 plugin not installed, 2 chord owned by something else.
set -euo pipefail

BINDINGS="${HYPR_BINDINGS:-$HOME/.config/hypr/bindings.lua}"
PLUGIN_ID='io.github.monomyth.sloptv'
SCRIPT_PATH="$HOME/.config/omarchy/plugins/$PLUGIN_ID/scripts/open.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE="$SCRIPT_DIR/../bindings.hypr.lua.example"

normalize_chord() {
  # Uppercase, drop pluses/spaces, sort tokens so SUPER SHIFT CTRL T == SHIFT CTRL SUPER T
  echo "$1" | tr '[:lower:]' '[:upper:]' | tr -cs 'A-Z0-9' ' ' | xargs -n1 | sort | xargs
}

TARGET_NORM="$(normalize_chord "SUPER SHIFT CTRL T")"
FS_NORM="$(normalize_chord "SUPER SHIFT CTRL F")"

chord_taken_by_other() {
  # Echo "conflict:<line>" and return 0 if another binding owns the chord,
  # echo "already:sloptv" and return 2 if slopTV owns it, return 1 if free/unknown.
  local print line left right norm
  print="$(omarchy menu keybindings --print 2>/dev/null || true)"
  [[ -z "$print" ]] && return 1
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    left="${line%%→*}"
    right="${line#*→}"
    left="$(echo "$left" | sed 's/+/ /g')"
    norm="$(normalize_chord "$left")"
    if [[ "$norm" == "$TARGET_NORM" ]]; then
      if echo "$right" | grep -qi 'slopTV'; then
        echo "already:sloptv"
        return 2
      fi
      echo "conflict:$line"
      return 0
    fi
  done <<< "$print"
  return 1
}

already_in_file() {
  [[ -f "$BINDINGS" ]] || return 1
  grep -q 'sloptv_open' "$BINDINGS" 2>/dev/null && return 0
  grep -qi 'slopTV' "$BINDINGS" 2>/dev/null && return 0
  grep -Eq 'SUPER \+ SHIFT \+ CTRL \+ T"|SUPER \+ CTRL \+ SHIFT \+ T"' "$BINDINGS" 2>/dev/null && return 0
  return 1
}


# Second chord: Super+Shift+Ctrl+F toggles fullscreen of the player window via Hyprland.
add_fullscreen_bind() {
  [[ -f "$BINDINGS" ]] || return 0
  grep -q 'sloptv_fullscreen\|open.sh fullscreen' "$BINDINGS" 2>/dev/null && return 0
  local print line left norm
  print="$(omarchy menu keybindings --print 2>/dev/null || true)"
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    left="$(echo "${line%%→*}" | sed 's/+/ /g')"
    norm="$(normalize_chord "$left")"
    if [[ "$norm" == "$FS_NORM" ]]; then
      echo "Super+Shift+Ctrl+F is already bound ($line); skipping the slopTV fullscreen chord." >&2
      return 0
    fi
  done <<< "$print"
  {
    echo ""
    echo "-- slopTV fullscreen chord (added by scripts/install-bind.sh)"
    echo 'o.bind("SUPER + SHIFT + CTRL + F", "slopTV fullscreen", function() hl.dispatch(hl.dsp.exec_cmd("bash " .. (os.getenv("HOME") or "") .. "/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh fullscreen")) end, { release = true })'
  } >> "$BINDINGS"
  echo "Added slopTV fullscreen chord (Super+Shift+Ctrl+F) to $BINDINGS"
}

if [[ ! -f "$SCRIPT_PATH" ]]; then
  echo "slopTV open.sh not found at:" >&2
  echo "  $SCRIPT_PATH" >&2
  echo "Install the plugin first:" >&2
  echo "  omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable" >&2
  exit 1
fi

if [[ ! -f "$EXAMPLE" ]]; then
  echo "bindings.hypr.lua.example not found next to scripts/." >&2
  exit 1
fi

if already_in_file; then
  echo "slopTV bind already present in $BINDINGS."
  add_fullscreen_bind
  exit 0
fi

status=0
result="$(chord_taken_by_other)" || status=$?
if [[ "$status" -eq 2 ]] || [[ "$result" == already:sloptv ]]; then
  echo "Super+Shift+Ctrl+T is already slopTV — nothing to do."
  exit 0
fi
if [[ "$status" -eq 0 ]]; then
  echo "Super+Shift+Ctrl+T is already bound:" >&2
  echo "  ${result#conflict:}" >&2
  echo "Pick another chord, or add manually from bindings.hypr.lua.example" >&2
  exit 2
fi

mkdir -p "$(dirname "$BINDINGS")"
if [[ ! -f "$BINDINGS" ]]; then
  echo "-- Keep only your personal keybinding overrides here." > "$BINDINGS"
fi

{
  echo ""
  echo "-- slopTV (added by scripts/install-bind.sh)"
  cat "$EXAMPLE"
} >> "$BINDINGS"

echo "Added slopTV bind to $BINDINGS"
add_fullscreen_bind
echo "Reload Hyprland config if the chord doesn't take effect (omarchy often hot-reloads)."
