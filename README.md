# slopTV

Keyboard-first Slop Cannon player for [Omarchy](https://omarchy.org/) Quattro.

One shortcut opens a Chromium app window (window class `slop-tv`) that plays each post in **X's own official embedded player**, unmodified, for the feed curated at [jon-slop.grok.me](https://jon-slop.grok.me). A small Rust program is the controller: it reads the feed and keeps the queue (next, previous, shuffle, like, open on X). The window shows nothing but the embed unless you move the mouse. The approach mirrors omarchy-meshpeek: `chromium --app` on a local launcher page.

The first time you run it, slopTV shows a one-time privacy notice: the embed is X's own player, loads X scripts and may set X cookies (in slopTV's own private Chromium profile, not your normal one). The embed URL uses `dnt=true&theme=dark`. Nothing is injected into the embed and slopTV never touches cookies.

## Install

```sh
omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable --yes
```

`--yes` skips Omarchy's interactive left/center/right prompt, so the bar icon lands in the manifest's default section (right). To move it later: `omarchy plugin enable io.github.monomyth.sloptv --section left|center|right`. Clicking the bar icon starts, focuses or closes the player, like the shortcut.

Then run the setup script, which builds the app and adds the **Super+Shift+Ctrl+T** keybind:

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/install.sh
```

When it finishes, `install.sh` prints a short how-to-run summary: the keybind, the bar icon (right side, if the widget is enabled), the in-app keys (`l`/`h` next/previous, `p` pause, `f` like, `o` open on X, `w` fullscreen, `q` quit) and the path to `uninstall.sh`.

Or skip the keybind step and paste `bindings.hypr.lua.example` into `~/.config/hypr/bindings.lua` yourself.

The install script checks whether that chord is free (Omarchy keybindings print plus your `bindings.lua`). If slopTV already owns it, it does nothing. If something else owns it, it prints the conflict and exits 2. Otherwise it appends the portable `sloptv_open()` helper (`os.getenv("HOME")`, no hardcoded path). Super+Shift+Ctrl+T is unused in Omarchy's default `default/hypr/bindings/*.lua` (only Super+T, Super+Ctrl+T and Super+Ctrl+Alt+T use T).

Needs Chromium (Omarchy ships it; Chrome or Brave also work, `xdg-open` is the fallback and opens your default browser as a normal tab), `jq`, `hyprctl`, plus `xdg-utils` for the `o` key. `scripts/install.sh` builds the binary once with `cargo build --release` into `~/.cache/omarchy/sloptv/` (needs `rust` and `pkgconf`; a few minutes). No webkit2gtk or GTK is needed. `mpv` is optional: it is only used by the hidden raw mode (see Advanced). If libmpv is missing, the build automatically leaves that mode out.

```sh
sudo pacman -S chromium rust pkgconf jq xdg-utils   # + mpv for the optional raw mode
```

## Shortcut

**Super+Shift+Ctrl+T**

| Situation | What happens |
| --- | --- |
| Not running | Starts slopTV (if you skipped `install.sh`, it builds on the first run) |
| Running, another window focused | Focuses slopTV |
| slopTV focused | Closes it |

## In the player

The window is just X's embed, centered on a dark background (`theme=dark` is an official embed parameter). Nothing else is drawn by default except a 2 px progress line along the bottom edge.

| key | action |
|---|---|
| l / Right / n | next post |
| h / Left | previous post |
| p / Space | pause / resume (freezes the auto-advance countdown) |
| a | auto-advance on/off (timer-based, see below) |
| s | shuffle on/off (default on) |
| f | like: opens `https://x.com/intent/like?tweet_id=<id>` via `xdg-open`; you confirm on X with your own login |
| o | open the post on X in your default browser (`xdg-open`); also your fallback for a post that won't play |
| w | toggle window fullscreen (`f` is Like, not fullscreen) |
| q | quit |

Keys work while our page has focus. Once you click inside X's player the player keeps the keyboard; click the dark background beside it to get the keys back (that also resumes a pause caused by the click). Small toasts confirm shuffle / auto-advance / like / open; there is no help panel.

**Previous / next cards.** Two small semi-transparent cards (thumbnail, @creator, short caption) sit in the bottom corners. They are visible only while the mouse moves, while paused, and for a few seconds when a new post appears, then fade out. Click one to jump. At the end of the list the next card says "loops to start". The thumbnails are plain `<img>` tags loaded from `pbs.twimg.com` (lazily, only after you accepted the privacy notice).

**Privacy notice.** The first time, a small box asks you to accept that this is X's own player (X scripts, possible X cookies in slopTV's private Chromium profile, `dnt=true`) before anything from X is loaded. After OK it never shows again (`--reset-notice` brings it back).

`o` opens the post's page on X; public posts open without an X account, but X may show a login wall or limit what is visible when signed out. The embedded player itself needs no account. slopTV starts Chromium with `--autoplay-policy=no-user-gesture-required`, but if a video does not start, click it once. The embed is X's own layout, so it can show its own scrollbar for long posts. There is no search or poster grid in this mode.

**Autoplay next, no looping.** When a video is over, slopTV moves to the next one and only wraps to the first after the end of the whole list (in shuffle mode, after every post of the current shuffle round). This is **timer-based in embed mode**, because X's player runs in a cross-origin iframe whose events (such as "ended") are not accessible to us, and slopTV does not inject into or modify the embed. Every record in the live `videos.json` carries `duration_ms` (checked: 594 of 594, from 4 s to 17 min), and the countdown for each clip is that clip's own duration, rounded up, plus 1 s for loading. It starts when the embed has finished loading in the window, and it restarts on next, previous, a card click and toggling `a`. A record without a duration waits for you (press next), or uses `embed_default_secs` if you set it. Because it is a timer, it can be off by a few seconds when the embed starts slowly, autoplay needs a click, or you pause or scrub the video itself.

**Pause.** `p` / Space freezes the countdown; resume continues from the remaining time, not from the full duration. While paused a small "paused" pill shows and the cards stay visible. Clicking inside X's player also pauses our countdown, because that is the only sign we get that you are interacting with X's player. **Limit:** we cannot observe X's own pause, seek or end events, so pausing with X's own controls does not pause our timer; click inside the player (which does) or press `p`.

**Like.** `f` opens X's like page for the current post in your default browser. slopTV uses no API keys, reads no cookies and automates nothing on X; you confirm the like there. Numeric post ids only. The mpv-only keys are listed under Advanced.

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

## Config

Copy `config.example.toml` to `~/.config/slop-tv/config.toml` (all keys optional). Embed mode uses `source_url`, `fallback_url`, `shuffle`, `auto_advance` (default true) and `embed_default_secs` (default 0: posts without a known duration wait for you); the other keys belong to the raw mode.

CLI: `slop-tv [--source URL|PATH] [--config FILE] [--cache-dir DIR] [--check] [--reset-notice]`. `--check` fetches and parses the feed and exits. `--reset-notice` shows the privacy notice again. `SLOPTV_BIN=/path/to/slop-tv` makes the launcher use that binary instead of building.

| thing | path |
|---|---|
| plugin | `~/.config/omarchy/plugins/io.github.monomyth.sloptv/` |
| binary | `~/.cache/omarchy/sloptv/target/release/slop-tv` |
| Chromium profile (private to slopTV) | `~/.cache/omarchy/sloptv/chromium-profile/` |
| build log | `~/.cache/omarchy/sloptv/build.log` |
| launcher log | `~/.local/state/omarchy/sloptv/slop-tv.log` |
| privacy notice acknowledged | `~/.local/state/slop-tv/embed-notice-ack` |
| config | `~/.config/slop-tv/config.toml` |
| feed cache | `~/.cache/slop-tv/` (feed + ETag only in embed mode) |

Optional: `packaging/slopTV.desktop` is a launcher entry for Walker (copy it to `~/.local/share/applications/` yourself); the plugin does not need it.

## What it talks to

**Embed mode (default):**

- `jon-slop.grok.me/videos.json` (feed, ETag polling; plus the page itself as a last-resort fallback), fetched by the Rust program.
- `platform.twitter.com/embed/Tweet.html?id=<id>&dnt=true&theme=dark` and whatever X's player then loads (x.com, X's scripts, `video.twimg.com`, `pbs.twimg.com`, possibly cookies in slopTV's private Chromium profile), loaded by Chromium inside the official embed. This is X's own player and X decides what it fetches.
- `pbs.twimg.com`: poster thumbnails for the previous/next cards, two `<img>` tags loaded lazily by Chromium on our page (not by the Rust program), only after the privacy notice is accepted. Posters come from the feed's `poster` field and are only used if they are on `pbs.twimg.com`.
- `x.com/intent/like?tweet_id=<id>`, only when you press `f` or the heart: opened in your default browser with `xdg-open`, never fetched by slopTV.
- `127.0.0.1` on a random port, with a random token in the URL, serves slopTV's own local page. It is not reachable from other machines.

slopTV loads no X API and no X post pages itself, injects no CSS or JS into the embed, and sends no cookies or auth of its own. The page is ours; the iframe inside it is X's, unmodified.

**Raw mode (hidden, unofficial):** `jon-slop.grok.me/videos.json`, `video.twimg.com` (mp4s, streamed by libmpv) and `pbs.twimg.com` (grid posters). It contacts no X page or API and does not use cookies. Streaming a raw mp4 is not the same as playing in X's own player. Whether X counts such plays as views is undocumented and untested.

## Advanced

### Raw mpv mode (unofficial)

An unofficial alternative that plays the plain `video.twimg.com` mp4 through libmpv instead of X's embed, with a poster grid, search and credit cards. **It may violate X's terms of service. Use it at your own risk.** It is off by default and not shown in `--help`. Enable it with `--raw-mpv` or `raw_mpv = true` in the config. Requires a build that includes libmpv (the default when mpv is installed at build time; see Development).

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


#### Download (hidden, raw mode only)

`download_clips = false` (config) or `--download` (CLI): when true, the next clip is prefetched into `~/.cache/slop-tv/clips/`, clips on disk play from disk, and clips + tiles are capped at `cache_max_mb`, oldest evicted first. Leave it off unless you want it.

## Remove

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/uninstall.sh
```

`omarchy plugin remove` only deletes the plugin folder. `uninstall.sh` also removes the keybind block from `~/.config/hypr/bindings.lua` (backup saved next to it as `bindings.lua.sloptv.bak`), stops a running player and its private Chromium, deletes the built binary and caches, and removes `~/.config/slop-tv/` (pass `--keep-config` to keep it). It finishes by running `omarchy plugin remove io.github.monomyth.sloptv --yes`.

## Development

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```

56 unit tests use the small fixtures in `tests/fixtures/` (20 records of the live feed, and a trimmed page with the embedded data block for 10). The timer tests run the page's pure timer code under `node` with a fake clock and are skipped when node is not installed. CI runs the same three commands on ubuntu-latest with `libmpv-dev`. The `raw-mpv` cargo feature (on by default) pulls in libmpv2 and image decoding; `cargo build --no-default-features` builds the embed-only controller without libmpv (`install.sh` falls back to that automatically when libmpv is missing). I chose a feature over a separate crate so the default binary and `install.sh` stay a single build.

## Not yet verified on Hyprland

Built and tested on a Debian box (Xvfb, google-chrome-stable): `cargo build --release`, unit tests (with and without the `raw-mpv` feature), the local controller served over HTTP (state, next, ack, quit, token gate), Chromium launching with its private profile and class `slop-tv`, and the process exiting cleanly (Chromium killed, launch file removed). The raw mode was smoke-tested earlier with libmpv under Xvfb.

Not verified: that X's embed actually renders and plays inside the window (screenshots under Xvfb show the embed loading its tweet card with the video poster and a play button; actual video playback was not observed), whether autoplay works or needs a click, embed behaviour for deleted or protected posts, anything on Wayland or Hyprland (app-id from `--class`, floating/tiling, `hyprctl` focus and close in `open.sh`), the Omarchy plugin loader, Quickshell loading `Service.qml` and `BarWidget.qml` (including the bar icon click and its default right placement), the bind via `o.bind` and `install-bind.sh` against a real `omarchy menu keybindings --print`, `xdg-open`, and the first-run build on Arch.

## Credits

Feed curated by [jon-slop.grok.me](https://jon-slop.grok.me). Plugin layout follows [omarchy-meshpeek](https://github.com/monomyth/omarchy-meshpeek). The videos belong to their creators; they are shown through X's own embedded player (or, in the hidden raw mode, streamed from video.twimg.com and, with `download_clips`, cached locally for playback only). slopTV does not redistribute them, and this repository contains only a small metadata fixture.

## License

MIT
