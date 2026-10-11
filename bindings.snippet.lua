-- slopTV — minimal string bind (open.sh toggle: start, focus, or close the player).
-- Prefer bindings.hypr.lua.example if you want the HOME-relative helper.
o.bind("SUPER + SHIFT + CTRL + T", "slopTV", "bash $HOME/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh toggle", { release = true })
-- slopTV fullscreen toggle of the player window via Hyprland (works even if keys do not reach the page).
o.bind("SUPER + SHIFT + CTRL + F", "slopTV fullscreen", "bash $HOME/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh fullscreen", { release = true })
