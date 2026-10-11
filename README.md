# slopTV

A keyboard-driven player for the Slop Cannon feed on [Omarchy](https://omarchy.org/). One shortcut opens a window that plays the curated posts from [jon-slop.grok.me](https://jon-slop.grok.me) one after another, video only, and moves on by itself.

![slopTV playing a clip, video only](docs/images/idle.webp)

## Terms of service and your responsibility

Read this before you install.

- slopTV is an unofficial hobby project. It is **not affiliated with, endorsed by or sponsored by X**, and it is **not submitted as an official Omarchy plugin**.
- **slopTV runs its own code inside X's page.** The default player opens `https://x.com/i/videos/tweet/<id>` as the top-level page of a Chromium app window and injects a JavaScript overlay into it through Chromium's remote debugging port (Chrome DevTools Protocol, bound to `127.0.0.1` on a random port). It also rewrites X's video playlist response so the highest quality is used (see below). That is more invasive than wrapping a page in an iframe and is more likely to run against X's terms. `player = "embed"` does none of this.
- The default player loads `https://x.com/i/videos/tweet/<id>`, an **undocumented X page**. X may change it, put it behind a login, or block it at any time. X's terms for third-party apps that display posts are not entirely clear on a use like this, and nobody here has asked X.
- slopTV does **not show all the display elements X's rules require** for displaying Posts (avatar, post text, linked timestamp, action icons). It shows a small attribution pill with the X mark as text, @handle, creator name when the feed has one, and a "View on X" link. See X's Display Requirements.
- The most conservative option is the official embed: set `player = "embed"` in the config. That shows X's own embedded Post in an iframe on our page, with nothing injected. It is the closest to what X documents, but it is still your call whether the way slopTV wraps it fits the rules.
- The hidden raw mode streams video files directly from `video.twimg.com` and may violate X's terms.
- Using this software, and complying with X's [Terms of Service](https://x.com/en/tos), [Developer Agreement](https://docs.x.com/developer-terms/agreement), [Developer Policy](https://docs.x.com/developer-terms/policy), [Display Requirements](https://docs.x.com/developer-terms/display-requirements) and the rights of the creators whose videos you watch, is **entirely your responsibility**. Use it at your own risk. The authors give no warranty (see the MIT license).

## What it does

- Fetches a list of post IDs (with creator, caption, duration, poster) from `jon-slop.grok.me/videos.json`.
- Opens a Chromium app window, with a private profile, that shows X's video page for the current post, filling the window. Nothing else is drawn by default.
- Moves to the next post when the clip's known duration (plus 1 s) has passed, never looping a single clip, and wraps to the first only after the end of the list.
- Shows small previous/next cards and an attribution pill when you move the mouse, pause, or a new clip starts.

| Cards and attribution (mouse moved) | Paused (cards stay visible) |
|---|---|
| ![cards overlay and attribution pill](docs/images/cards-and-attribution.webp) | ![paused, cards visible](docs/images/paused.webp) |

Screenshots were taken in Chromium under Xvfb, not on Hyprland.

## How it works

1. A small Rust program (the controller) loads the feed and keeps the queue (next, previous, shuffle).
2. It starts Chromium as an app window with a private profile under `~/.cache/omarchy/sloptv/chromium-profile/` (`--autoplay-policy=no-user-gesture-required`, fullscreen by default) and `--remote-debugging-port=<random free port>` on `127.0.0.1`. The window's page is X's video page itself, **top-level, not an iframe**.
3. **Our code runs inside X's page.** The controller connects to the debugging port over a websocket and uses `Page.addScriptToEvaluateOnNewDocument` to inject `overlay.js` (the cards, attribution pill, 2 px progress line and all keys). The overlay lives in a shadow DOM and talks to the controller through a `Runtime` binding; the controller moves between clips with `Page.navigate`. Anything else on your machine that can reach that loopback port can also drive the window while slopTV runs; the port is closed when slopTV exits.
4. **Auto-advance follows the real `<video>`.** The overlay reads the page's `<video>` (`currentTime`, `duration`, `paused`, `ended`): the progress line is `currentTime / duration`, so pausing (click, `p` or Space) and scrubbing in X's player move our timer too. It advances when the video ends (or loops back to the start). Only while the page has no usable `<video>` duration does it fall back to the feed's `duration_ms` plus 1 s.
5. **Quality.** X's video page has no quality menu and starts on its lowest variant. With `best_quality = true` (default) the controller intercepts the `video.twimg.com/*.m3u8` master playlist response over the debugging port (`Fetch` domain) and removes every variant except the highest resolution, so X's own player loads that one. Set `best_quality = false` to leave X's playlist alone. A high-resolution variant uses more bandwidth and decoding (1440p on the tested clip).
6. Keys are handled in the page itself: `h`/`l`/`n` prev/next, `p`/Space pause, `s` shuffle, `a` auto-advance, `f` like page, `o` open post, `w`/F11 fullscreen (through Hyprland when available), `d` debug overlay, `q` quit.

### Player modes

| mode | how | notes |
|---|---|---|
| video page (default) | `player = "video"` | X's undocumented video-only page, `?autoplay=1` (also undocumented), as the top-level page with our overlay injected through the debugging port. If navigation to it fails, the official embed page is loaded top-level instead. |
| official embed | `player = "embed"` | X's official embedded Post (`platform.twitter.com/embed/Tweet.html?id=<id>&dnt=true&theme=dark`), unmodified, centered. Shows the full card. Not video-only. |
| raw mpv (hidden) | `--raw-mpv` or `raw_mpv = true` | Streams the mp4 through libmpv. Not shown in `--help`. See Advanced. |

Optional hidden embed settings (`embed_use_widgets`, `embed_scale_to_fit`) and the research behind the design are in [docs/EMBED_NOTES.md](docs/EMBED_NOTES.md).

### Keys

| key | action |
|---|---|
| l / Right / n | next post |
| h / Left | previous post |
| p / Space | pause / resume our auto-advance timer |
| a | auto-advance on/off |
| s | shuffle on/off (default on) |
| f | like: opens `https://x.com/intent/like?tweet_id=<id>` in your default browser; you confirm on X with your own login |
| o | open the post on X in your default browser (`xdg-open`). Public posts open without an account, but X may show a login wall when signed out |
| w or F11 | toggle fullscreen |
| d | debug overlay (top right): current id, feed duration, seconds remaining, paused, auto-advance, position, focused element. Hidden by default |
| q | quit |

Mouse: move to show the cards; click a card to jump; double-click a screen edge strip to toggle fullscreen. Keys only reach our page while it has focus; see Troubleshooting.

Global chords (set up by the install script, work whatever has focus): **Super+Shift+Ctrl+T** starts, focuses or closes the player; **Super+Shift+Ctrl+F** toggles fullscreen of the player window through Hyprland. The bar icon does the same as the first chord.

## Install

```sh
omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable --yes
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/install.sh
```

`--yes` skips Omarchy's left/center/right prompt, so the bar icon lands on the right. Move it later with `omarchy plugin enable io.github.monomyth.sloptv --section left|center|right`.

`install.sh` builds the binary once with `cargo build --release` into `~/.cache/omarchy/sloptv/` (a few minutes), adds the two keybinds if the chords are free (it reports a conflict instead of overwriting), and prints a how-to-run summary. To add the binds by hand, paste `bindings.hypr.lua.example` into `~/.config/hypr/bindings.lua`.

**Requirements:** Chromium (Chrome and Brave also work), `jq`, `hyprctl`, `xdg-utils`, and for building `rust` and `pkgconf`. `mpv` is optional (raw mode only; the build leaves raw mode out if libmpv is missing).

```sh
sudo pacman -S chromium rust pkgconf jq xdg-utils
```

**Uninstall:**

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/uninstall.sh
```

This removes the keybind blocks (backup in `bindings.lua.sloptv.bak`), stops the player and its private Chromium, deletes the binary and caches and `~/.config/slop-tv/` (`--keep-config` keeps it), then runs `omarchy plugin remove io.github.monomyth.sloptv --yes`.

**Reinstall / update:**

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/uninstall.sh --keep-config
omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable --yes
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/install.sh
```

## Config

Copy `config.example.toml` to `~/.config/slop-tv/config.toml`. All keys are optional.

| key | default | meaning |
|---|---|---|
| `player` | `"video"` | `"video"` (X video page) or `"embed"` (official embed) |
| `start_fullscreen` | `true` | open the window fullscreen. It is applied once at startup and never re-forced after you leave fullscreen |
| `auto_advance` | `true` | move on by timer after each post's duration |
| `best_quality` | `true` | video player: keep only the highest-resolution variant in X's HLS master playlist |
| `embed_default_secs` | `0` | seconds to show a post with no known duration (0 = wait for you) |
| `shuffle` | `true` | shuffled order |
| `source_url`, `fallback_url` | jon-slop.grok.me | feed location (URL, path or `file://`) |

CLI: `slop-tv [--source URL|PATH] [--config FILE] [--cache-dir DIR] [--check] [--reset-notice]`. `--check` fetches and parses the feed and exits; `--reset-notice` shows the first-run notice again. `SLOPTV_BIN=/path/to/slop-tv` makes the launcher use that binary.

First run shows a small notice: the default page is unofficial, loads X scripts and media, and X may set cookies in slopTV's private Chromium profile (not your normal one). Nothing starts loading from X until you accept.

| thing | path |
|---|---|
| plugin | `~/.config/omarchy/plugins/io.github.monomyth.sloptv/` |
| binary | `~/.cache/omarchy/sloptv/target/release/slop-tv` |
| Chromium profile | `~/.cache/omarchy/sloptv/chromium-profile/` |
| build log | `~/.cache/omarchy/sloptv/build.log` |
| launcher log | `~/.local/state/omarchy/sloptv/slop-tv.log` |

## Troubleshooting

**Fullscreen does nothing, or you cannot get back into it.** On Hyprland, `w`, F11, a double-click on a screen edge strip and Super+Shift+Ctrl+F all ask Hyprland itself to toggle the slop-tv window (focus it by address, then `fullscreen 0`). Outside Hyprland, `w` uses the browser's Fullscreen API. slopTV only requests fullscreen once at startup. If it still fails, look at the window state:

```sh
hyprctl clients -j | jq '.[] | select(.class|test("slop-tv")) | {address, class, fullscreen, fullscreenClient, floating, workspace: .workspace.name, at, size}'
# or, with the plugin installed:
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh status
```

On Wayland the class looks like `chrome-__tmp_slop-tv-launch-<pid>.html-Default` (pid = the slop-tv controller), not `slop-tv`; slopTV, `open.sh` and the chord match any class containing `slop-tv` and prefer the one with the controller's pid. `fullscreen` greater than 0 (2 = fullscreen) means Hyprland considers it fullscreen; the toggle (`fullscreen 0`) leaves it windowed and a second press returns. Try the toggle by hand with `bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/open.sh fullscreen` and compare the output; the launcher log is `~/.local/state/omarchy/sloptv/slop-tv.log`.

**Keys stop working after I click the video.** Clicking gives X's frame the keyboard. slopTV takes focus back about a second after a click, and also when the mouse enters a screen edge strip (22 px left/right, 14 px top). Clicking in the video does not affect the countdown.

**A play button shows and the video does not start.** The video page autoplays only because of the undocumented `?autoplay=1`. If X stops honouring it, click the video once per clip. `player = "embed"` has the same limit.

**The page does not load.** If Chromium cannot navigate to the video page (offline, DNS, blocked) the official embed page is loaded instead. An HTTP error, a login wall or an "unavailable" message inside a page that did load is not detected. If Chromium exits straight away, another window using the same slopTV profile may still be open (it then ignores the debugging port). Set `player = "embed"` to avoid the page entirely.

**The next video did not start.** Press `d` to see the countdown: if `remaining` runs down and `paused` is false, slopTV will advance when it hits 0; if `paused` is true you pressed `p`/Space (press it again); if `remaining` says `off`, auto-advance is off (`a`) or the post has no feed duration. If it advanced but the new clip shows a play button, X did not honour `?autoplay=1` for that load (we cannot read the page to confirm playback, so the countdown runs regardless; click the video once). An X error or login wall inside the frame is not detectable and also does not stop the countdown. Chromium is started with `--disable-background-timer-throttling`, `--disable-renderer-backgrounding` and `--disable-backgrounding-occluded-windows` so a covered or background window keeps its pace.

**The auto-advance changes too early or late.** In the default player it follows the real `<video>`, so it should match what you see; press `d` to see what it is following. Only when the page exposes no usable video duration does it use the feed's duration plus 1 s. In `player = "embed"` it is a pure timer from the feed duration plus 1 s. `a` turns it off.

## What it talks to

**Video page (default):**
- `jon-slop.grok.me/videos.json` (feed, ETag polling), fetched by the Rust program.
- `x.com/i/videos/tweet/<id>?autoplay=1` as the top-level page of the app window, and whatever that page loads (X scripts, `video.twimg.com`, possibly cookies in slopTV's private Chromium profile). slopTV's overlay script runs in that page and the controller reads/replaces the `video.twimg.com` HLS master playlist response (never cookies or request headers).
- `pbs.twimg.com`: poster thumbnails for the cards, plain lazy `<img>` tags, only after you accept the notice.
- `x.com/intent/like?tweet_id=<id>` and the post URL, only through `xdg-open` when you press `f`, `o` or click "View on X".
- `127.0.0.1` on a random port: Chromium's remote debugging port (default player), or our local page with a random token (`player = "embed"`); neither is reachable from other machines.

**Embed (`player = "embed"`):** the same, but `platform.twitter.com/embed/Tweet.html?id=<id>&dnt=true&theme=dark` instead of the video page (and `platform.twitter.com/widgets.js` if the hidden `embed_use_widgets` is on).

**Raw mode:** `jon-slop.grok.me/videos.json`, `video.twimg.com` mp4s (libmpv) and `pbs.twimg.com` posters. No X page or API.

slopTV calls no X API, sends no cookies or credentials of its own and reads no cookies. With the default player it **does** inject JavaScript and a stylesheet into X's video page and rewrites one playlist response (see above); `player = "embed"` injects nothing. Streaming a raw mp4 is not the same as playing in X's own player, and whether X counts such plays as views is undocumented and untested.

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
| `videos[].mp4` | string | direct mp4 link (raw mode only; may be empty) |
| `videos[].poster` | string | poster image URL |
| `videos[].duration_ms`, `likes`, `views` | int | |
| `videos[].phase` | string | |
| `videos[].unsure` | bool | |

Fallbacks, in order: the on-disk copy of the last good feed; the embedded `<script id="data">` JSON of `https://jon-slop.grok.me` (short keys). `--source` or `source_url` also takes a local path or `file://` for development. Embed mode needs only each record's post `id` (and `url` for `o`). The `mp4` and `poster` fields are used only by the hidden raw mode. Binary feed formats were evaluated and not adopted: see [docs/FORMAT_EVAL.md](docs/FORMAT_EVAL.md).

## Advanced

### Raw mpv mode (unofficial)

Plays the plain `video.twimg.com` mp4 through libmpv instead of X's page, with a poster grid, search and credit cards. **It may violate X's terms of service. Use it at your own risk.** Off by default and not shown in `--help`. Enable it with `--raw-mpv` or `raw_mpv = true`. It needs a build that includes libmpv.

#### Raw mode keys

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
| n | next clip (always) | next clip |
| f | fullscreen | fullscreen |
| i | credit card (right third); again or Esc closes | - |
| I | small bottom-left card for 4 s | - |
| o | open post in browser (xdg-open); also the fallback when a clip cannot be streamed | opens focused tile's post |
| y | copy post link (wl-copy) | copies focused tile's link |
| Tab | open grid | close grid |
| / | open grid with search | start search |
| Enter | - | play focused tile (when searching: finish typing) |
| Esc | close help / card | clear search, then close grid |
| ? | help overlay | help overlay |
| q | quit | quit |

`o` opens the post's page on X in your default browser via `xdg-open`; it is the fallback for a clip that won't play. Public posts open without an X account, but X may show a login wall or limit what is visible when signed out. The video itself needs no account, since it streams as a plain mp4.

Search: press `/`, type creator, handle or caption words (all must match), Enter to stop typing, Esc to clear.


Raw-mode config keys: `volume`, `fullscreen`, `cache_max_mb`, `poll_minutes`, `font`, `hwdec`, `vo`, `ao`, `gpu_context`; CLI: `--windowed`, `--geometry WxH`; `SLOPTV_MPV_LOG=/path` writes a verbose mpv log. Raw mode caches only the feed and small grid tiles under `~/.cache/slop-tv/`.

Playback streams each clip's mp4 URL straight into libmpv (`loadfile https://video.twimg.com/...mp4`), as the jon-slop.grok.me gallery's `<video src=mp4>` does. mpv's demuxer cache does the buffering (64 MiB ahead, 20 s read-ahead). Raw mode never loops a clip: `loop-file` is off, and at end-of-file it advances to the next clip, wrapping to the first only after the end of the whole list (the old `r` loop key and `loop_clip` option are gone). There is no next-clip prefetch while streaming, so `n`/`l` open a fresh connection (in a live Xvfb test the first frame took several seconds on one clip).

The gallery's fallback to X's embedded player is a web iframe and cannot be done in mpv. If a clip fails for good, press `o` to open the post in your browser.

Search: press `/`, type creator, handle or caption words (all must match), Enter to stop typing, Esc to clear.

Raw-mode config keys: `volume`, `fullscreen`, `cache_max_mb`, `poll_minutes`, `font`, `hwdec`, `vo`, `ao`, `gpu_context`; CLI: `--windowed`, `--geometry WxH`; `SLOPTV_MPV_LOG=/path` writes a verbose mpv log. Playback streams each clip's mp4 straight into libmpv (64 MiB / 20 s demuxer cache), never loops a clip (it advances at end-of-file and wraps after the last one), and has no next-clip prefetch. If a clip fails for good, press `o`.

#### Download (hidden, raw mode only)

`download_clips = false` (config) or `--download` (CLI): when true, the next clip is prefetched into `~/.cache/slop-tv/clips/`, clips on disk play from disk, and clips + tiles are capped at `cache_max_mb`, oldest evicted first. Leave it off unless you want it.

## Known limitations and not verified

Tested on a Debian box with Xvfb, google-chrome-stable and xfwm4: unit tests, the controller over HTTP, the page's keys, fullscreen via the web API, F11, focus recovery after a click in X's frame, four consecutive short clips auto-advancing with and without a click in the video, autoplay and timer advance with live X pages (logged out), and the unreachable-page fallback (by blocking `x.com`). The Hyprland fullscreen path was tested only against a fake `hyprctl` that records its calls.

Not verified:
- Anything on a real Hyprland/Wayland desktop: `--start-fullscreen`, the `hyprctl` calls (`focuswindow address:...`, `fullscreen 0`) on Omarchy's Hyprland version, window rules, and the install script's bind handling.
- That X keeps serving the video page and honouring `?autoplay=1`.
- Whether any of this fits X's terms (see the top of this file).
- Behaviour for deleted, protected or age-restricted posts, and for X login walls.
- Playback quality, sound and long sessions on Isengard.

Known limits: no on-screen pause label (the cards staying visible is the cue); pause stops slopTV's countdown; whether the video itself stops depends on the player. In `player = "embed"`, auto-advance is timer-only and our keys need page focus; the attribution pill is not the full set of X display elements; screenshots in this README are from Xvfb.

## Development

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```

Unit tests use the fixtures in `tests/fixtures/`. The timer and page-syntax tests need `node` and are skipped without it. The `raw-mpv` cargo feature (on by default) pulls in libmpv2 and image decoding; `cargo build --no-default-features` builds the controller without libmpv, which `install.sh` falls back to automatically. Binary feed formats were evaluated and not adopted: [docs/FORMAT_EVAL.md](docs/FORMAT_EVAL.md). Research notes: [docs/EMBED_NOTES.md](docs/EMBED_NOTES.md).

## Credits

Feed curated by [jon-slop.grok.me](https://jon-slop.grok.me). Plugin layout follows [omarchy-meshpeek](https://github.com/monomyth/omarchy-meshpeek). The videos belong to their creators and are shown through X's own pages (or, in raw mode, streamed from video.twimg.com and, with `download_clips`, cached locally for playback only). slopTV does not redistribute them; the repository holds only a small metadata fixture. Screenshots show third-party videos from the live feed.

## License

MIT
