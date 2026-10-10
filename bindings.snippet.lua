-- slopTV — minimal string bind (open.sh toggle: start, focus, or close the player).
-- Prefer bindings.hypr.lua.example if you want the HOME-relative helper.
o.bind("SUPER + SHIFT + CTRL + T", "slopTV", "bash $HOME/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh toggle", { release = true })
