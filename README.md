# slopTV

Keyboard-first Slop Cannon player for [Omarchy](https://omarchy.org/) Quattro.

One shortcut starts a fullscreen libmpv player for the feed curated at [jon-slop.grok.me](https://jon-slop.grok.me). Written in Rust, hardware decoding through mpv, no TUI. Vim keys, a poster grid, search, and a credit card per clip. Window title `slopTV`, app-id `slop-tv`.

## Install

```sh
omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable
```

Then add **Super+Shift+Ctrl+T** either way:

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/install-bind.sh
```

Or paste `bindings.hypr.lua.example` into `~/.config/hypr/bindings.lua` yourself.

The install script checks whether that chord is free (Omarchy keybindings print plus your `bindings.lua`). If slopTV already owns it, it does nothing. If something else owns it, it prints the conflict and exits 2. Otherwise it appends the portable `sloptv_open()` helper (`os.getenv("HOME")`, no hardcoded path). Super+Shift+Ctrl+T is unused in Omarchy's default `default/hypr/bindings/*.lua` (only Super+T, Super+Ctrl+T and Super+Ctrl+Alt+T use T).

Needs `mpv` (provides libmpv), `jq`, `hyprctl`, plus `wl-clipboard` and `xdg-utils` for the copy / open keys. `yt-dlp` is only used for records without a direct mp4. First open builds the binary once with `cargo build --release` into `~/.cache/omarchy/sloptv/` (needs `rust` and `pkgconf`; a few minutes). If cargo or libmpv is missing you get a notification saying what to install.

```sh
sudo pacman -S mpv rust pkgconf jq wl-clipboard xdg-utils yt-dlp
```

## Shortcut

**Super+Shift+Ctrl+T**

| Situation | What happens |
| --- | --- |
| Not running | Starts slopTV (builds it first if this is the first run) |
| Running, another window focused | Focuses slopTV |
| slopTV focused | Closes it |

## In the player

Vim keys and arrows. The first keypress un-mutes (audio starts muted).

| key | player / card | grid |
|---|---|---|
| Space | pause | pause |
| l / Right | next clip | focus right |
| h / Left | previous clip | focus left |
| k / Up | volume up | focus up |
| j / Down | volume down | focus down |
| m | mute | mute |
| s | shuffle on/off (default on) | same |
| r | loop clip / advance at end (default loop) | same |
| n | next clip (always) | next clip |
| f | fullscreen | fullscreen |
| i | credit card (right third); again or Esc closes | - |
| I | small bottom-left card for 4 s | - |
| o | open post in browser (xdg-open) | opens focused tile's post |
| y | copy post link (wl-copy) | copies focused tile's link |
| Tab | open grid | close grid |
| / | open grid with search | start search |
| Enter | - | play focused tile (when searching: finish typing) |
| Esc | close help / card | clear search, then close grid |
| ? | help overlay | help overlay |
| q | quit | quit |

Search: press `/`, type creator, handle or caption words (all must match), Enter to stop typing, Esc to clear.

## Data source

Default: `https://jon-slop.grok.me/videos.json` (newest first, ETag, re-checked every 60 min with `If-None-Match`, expect 304).

```
{name, source, count, updated_at, note,
 videos: [{id, url, creator, caption, posted_at, mp4, poster,
           duration_ms, likes, views, phase, unsure}]}
```

| field | type | notes |
|---|---|---|
| `name`, `source`, `note` | string | feed metadata |
| `count` | int | number of videos |
| `updated_at` | string | newest video's post time |
| `videos[].id` | string | post id |
| `videos[].url` | string | X post URL |
| `videos[].creator` | string | handle, shown as `@handle` |
| `videos[].caption` | string | post text |
| `videos[].posted_at` | string | timestamp |
| `videos[].mp4` | string | direct mp4 link (may be empty: yt-dlp is used) |
| `videos[].poster` | string | poster image URL |
| `videos[].duration_ms`, `likes`, `views` | int | |
| `videos[].phase` | string | |
| `videos[].unsure` | bool | |

Fallbacks, in order: the on-disk copy of the last good feed; the embedded `<script id="data">` JSON of `https://jon-slop.grok.me` (short keys). `--source` or `source_url` also takes a local path or `file://` for development. Expired mp4 links are skipped; offline you get one line and only cached clips play. Binary feed formats were evaluated and not adopted: see [docs/FORMAT_EVAL.md](docs/FORMAT_EVAL.md).

## Config

Copy `config.example.toml` to `~/.config/slop-tv/config.toml` (all keys optional): `source_url`, `fallback_url`, `volume`, `shuffle`, `loop_clip`, `fullscreen`, `cache_max_mb`, `poll_minutes`, `font`, `hwdec`, `vo`, `ao`, `gpu_context`.

CLI: `slop-tv [--source URL|PATH] [--config FILE] [--cache-dir DIR] [--windowed] [--geometry WxH] [--check]`. `--check` fetches and parses the feed and exits. `SLOPTV_MPV_LOG=/path` writes a verbose mpv log. `SLOPTV_BIN=/path/to/slop-tv` makes the launcher use that binary instead of building.

| thing | path |
|---|---|
| plugin | `~/.config/omarchy/plugins/io.github.monomyth.sloptv/` |
| binary | `~/.cache/omarchy/sloptv/target/release/slop-tv` |
| build log | `~/.cache/omarchy/sloptv/build.log` |
| launcher log | `~/.local/state/omarchy/sloptv/slop-tv.log` |
| config | `~/.config/slop-tv/config.toml` |
| clip / poster cache | `~/.cache/slop-tv/` (size capped, default 2048 MB) |

Optional: `packaging/slopTV.desktop` is a launcher entry for Walker (copy it to `~/.local/share/applications/` yourself); the plugin does not need it.

## Remove

```sh
omarchy plugin remove io.github.monomyth.sloptv
```

Then delete the bind block from `~/.config/hypr/bindings.lua`, and optionally `~/.cache/omarchy/sloptv/` and `~/.cache/slop-tv/`.

## Development

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```

34 unit tests use the small fixtures in `tests/fixtures/` (20 records of the live feed, and a trimmed page with the embedded data block for 10). CI runs the same three commands on ubuntu-latest with `libmpv-dev`.

## Not yet verified on Hyprland

Built and tested on a Debian box (mpv 0.40, Xvfb, software GL): `cargo build --release`, unit tests, live feed fetch including a real 304, real mp4 and poster download from X, playback, grid, search, cards, help, failed-clip skipping, offline notice.

Not verified: anything on Wayland or Hyprland (app-id, fullscreen, `hyprctl` focus and close in `open.sh`), the Omarchy plugin loader, Quickshell loading `Service.qml` and `BarWidget.qml`, the bind via `o.bind` and `install-bind.sh` against a real `omarchy menu keybindings --print`, hardware decoding, audio, `wl-copy` / `xdg-open`, yt-dlp playback of X page URLs, and the first-run build on Arch (needs mpv client API 2.x).

## Credits

Feed curated by [jon-slop.grok.me](https://jon-slop.grok.me). Plugin layout follows [omarchy-meshpeek](https://github.com/monomyth/omarchy-meshpeek). The videos belong to their creators; they are streamed from X (video.twimg.com) and cached locally for playback only. slopTV does not redistribute them, and this repository contains only a small metadata fixture.

## License

MIT
