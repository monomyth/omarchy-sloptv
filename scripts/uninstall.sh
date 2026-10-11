#!/usr/bin/env bash
# Full cleanup. `omarchy plugin remove` only deletes the plugin folder, so this also
# removes the keybind block, the built binary, the cache and your slopTV config.
# Usage: uninstall.sh [--keep-config]
# Never runs sudo. Writes only under $HOME. Backs up bindings.lua first.
set -euo pipefail

PLUGIN_ID='io.github.monomyth.sloptv'
BINDINGS="${HYPR_BINDINGS:-$HOME/.config/hypr/bindings.lua}"
KEEP_CONFIG=0
[[ "${1:-}" == "--keep-config" ]] && KEEP_CONFIG=1

if [[ -f "$BINDINGS" ]] && grep -q 'sloptv_open' "$BINDINGS"; then
  cp "$BINDINGS" "$BINDINGS.sloptv.bak"
  python3 - "$BINDINGS" <<'PY'
import re, sys
p = sys.argv[1]
lines = open(p).read().split("\n")
has_fs = any(ln.startswith("o.bind(") and '"slopTV fullscreen"' in ln for ln in lines)
out, skipping, skip_next = [], False, False
for ln in lines:
    if skip_next:
        skip_next = False
        continue
    if "slopTV fullscreen chord (added by scripts/install-bind.sh)" in ln:
        skip_next = True
        continue
    if not skipping and "slopTV (added by scripts/install-bind.sh)" in ln:
        skipping = True
        continue
    if skipping:
        end = '"slopTV fullscreen"' if has_fs else '"slopTV"'
        if ln.startswith("o.bind(") and end in ln:
            skipping = False
        continue
    out.append(ln)
text = "\n".join(out)
text = re.sub(r"\n{3,}", "\n\n", text)
open(p, "w").write(text)
PY
  echo "slopTV: removed the keybind block (backup: $BINDINGS.sloptv.bak)"
else
  echo "slopTV: no keybind block found in $BINDINGS"
fi

pkill -x slop-tv 2>/dev/null || true
pkill -f "slop-tv-launch-" 2>/dev/null || true
pkill -f "omarchy/sloptv/chromium-profile" 2>/dev/null || true
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/omarchy/sloptv" "${XDG_CACHE_HOME:-$HOME/.cache}/slop-tv" \
       "${XDG_STATE_HOME:-$HOME/.local/state}/omarchy/sloptv" "${XDG_STATE_HOME:-$HOME/.local/state}/slop-tv"
(( KEEP_CONFIG )) || rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/slop-tv"
if (( KEEP_CONFIG )); then echo "slopTV: removed caches (config kept)"; else echo "slopTV: removed caches and config"; fi

# Last step: this deletes the folder this script lives in.
if [[ -z "${SLOPTV_SKIP_PLUGIN_REMOVE:-}" ]] && command -v omarchy >/dev/null 2>&1; then
  exec omarchy plugin remove "$PLUGIN_ID" --yes
fi
