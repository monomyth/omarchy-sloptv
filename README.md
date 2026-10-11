# slopTV

Keyboard-first Slop Cannon player for [Omarchy](https://omarchy.org/) Quattro.

One shortcut opens a Chromium app window (window class `slop-tv`) that plays each post in **X's own video-only player page** (`https://x.com/i/videos/tweet/<id>?autoplay=1`) filling the window, for the feed curated at [jon-slop.grok.me](https://jon-slop.grok.me). A small Rust program is the controller: it reads the feed and keeps the queue (next, previous, shuffle, like, open on X). The window shows nothing but the video unless you move the mouse. `player = "embed"` in the config switches to X's official embedded Post instead. The approach mirrors omarchy-meshpeek: `chromium --app` on a local launcher page.

The first time you run it, slopTV shows a one-time privacy notice: the default video page is an **unofficial, undocumented X page** that may change or be blocked, loads X scripts and media and may set X cookies (in slopTV's own private Chromium profile, not your normal one). The notice says so, and says slopTV falls back to the official embed if the page is unreachable. Nothing is injected into X's page or the embed, and slopTV never touches cookies.

## The video player page (default) and its limits

`x.com/i/videos/tweet/<id>` is the video-only player X's own embeds use. It is **not documented anywhere in docs.x.com**, so treat it as an unofficial surface: X may change it, add a login wall, or block it, and whether it fits X's terms is untested and at your own risk (the official embed remains available with `player = "embed"`). What we tested in Chromium under Xvfb, logged out, with the live feed:

- The page loads without any X account and shows only X's player (video, X's own small time/views label). No card, no header.
- Without a parameter it shows a poster and a big play button that needs a click on every clip. The page accepts an **undocumented** `?autoplay=1`; with it, and `allow="autoplay; fullscreen"` on our iframe plus Chromium's `--autoplay-policy=no-user-gesture-required`, clips started on their own, unmuted, including after auto-advance. If X stops honouring it, you will see the play button and must click it.
- Auto-advance is still timer-based (feed `duration_ms` + 1 s), because the iframe's events are not readable. Pause (`p`/Space) freezes only **our** timer; X's player keeps playing, and a click inside the video also pauses our timer.
- Clicking inside the video gives that frame the keyboard focus, so our keys stop working until you move the mouse to a screen edge or click outside it. Our edge zones (22 px left/right, 14 px top) are transparent strips on our page that show the cards and attribution on mouse move, since mouse events inside X's frame are invisible to us.
- Attribution: a small pill in the top-left corner (X mark as text, @handle, name when the feed has it, and a **View on X** link, which opens the post URL via `xdg-open`) fades in on mouse move, pause and clip change. It does not cover the video centre. It is our own element; it is not the full set of Display Requirements elements (no avatar, text, timestamp or action icons), which is one more reason this page is a gray area.
- Fallback: when the first clip loads, slopTV makes one opaque `no-cors` request to the same URL (response never read, no credentials). If it fails (DNS, connection, TLS, 8 s timeout), the session switches to the official embed. Verified by blocking `x.com` in Chromium. Not detectable without inspecting X's content: HTTP errors, a login wall, or an X "unavailable" message inside a page that did load. Only the first clip is probed.

## Install

```sh
omarchy plugin add https://github.com/monomyth/omarchy-sloptv.git --enable --yes
```

`--yes` skips Omarchy's interactive left/center/right prompt, so the bar icon lands in the manifest's default section (right). To move it later: `omarchy plugin enable io.github.monomyth.sloptv --section left|center|right`. Clicking the bar icon starts, focuses or closes the player, like the shortcut.

Then run the setup script, which builds the app and adds the **Super+Shift+Ctrl+T** keybind:

```sh
bash ~/.config/omarchy/plugins/io.github.monomyth.sloptv/scripts/install.sh
```

When it finishes, `install.sh` prints a short how-to-run summary: the keybind, the bar icon (right side, if the widget is enabled), the in-app keys (`l`/`h` next/previous, `p` pause, `f` like, `o` open on X, `w` fullscreen toggle, `q` quit) and the path to `uninstall.sh`.

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

The window is just X's video page, filling the window (with `player = "embed"`: X's embed, centered on a dark background, `theme=dark` is an official embed parameter). Nothing else is drawn by default except a 2 px progress line along the bottom edge.

| key | action |
|---|---|
| l / Right / n | next post |
| h / Left | previous post |
| p / Space | pause / resume (freezes the auto-advance countdown) |
| a | auto-advance on/off (timer-based, see below) |
| s | shuffle on/off (default on) |
| f | like: opens `https://x.com/intent/like?tweet_id=<id>` via `xdg-open`; you confirm on X with your own login |
| o | open the post on X in your default browser (`xdg-open`); also your fallback for a post that won't play |
| w | toggle fullscreen (the window starts fullscreen; `f` is Like, not fullscreen) |
| q | quit |

Keys work while our page has focus. Once you click inside X's player the player keeps the keyboard; click the dark background beside it to get the keys back (that also resumes a pause caused by the click). Small toasts confirm shuffle / auto-advance / like / open; there is no help panel.

**Fullscreen: what works (tested in Chromium).** The window starts fullscreen (`start_fullscreen = true`; set it to `false` to open windowed). slopTV passes `--start-fullscreen` to Chromium and, on Hyprland, checks that the `slop-tv` window really is fullscreen and sends one `hyprctl dispatch fullscreen 0` if it is not. `w` toggles the window's fullscreen on and off (`f` is Like). This page-level fullscreen is the one that survives auto-advance: in a Chromium test, a timer-driven advance (no keypress) changed the iframe `src` and the window stayed fullscreen, and our keys (`l`, `p`, `w`) still reached the page afterwards.

**Video-only view, and why it does not survive auto-advance.** Click the fullscreen button inside X's player to see just the video (the iframe allows it: `allow="fullscreen; autoplay; encrypted-media; picture-in-picture"` plus `allowfullscreen`, and the Chromium flags do not block it). In the same test, when X's player was fullscreen: our page received no keys (the player holds the keyboard; press Esc to leave), and when the timer advanced to the next post the fullscreen **ended** (fullscreen element went back to none). The countdown itself keeps running during X's fullscreen (clicking that button does not pause it). Entering fullscreen again needs a real user click on X's own button for every clip; a page cannot do that for you without a user gesture, and slopTV does not script or click inside X's embed. The alternatives we tried: making our own iframe element the fullscreen element does survive `src` changes, but then the embed is shown at its native 550 px, left-aligned on X's white page background, not scaled, and it is not video-only; fullscreening our page (`w`) is the same as the window-level fullscreen above. So the honest summary: hands-free auto-advance works in a fullscreen window showing the centered post card with its video; video-only view needs a click on X's fullscreen button per clip.

**Embed look: what X documents, and what we tested.** The documented options for an embedded post are `theme`, `dnt`, `lang`, `width` (250-550), `align`, `conversation=none` / `hide_thread`, `cards=hidden` / `hide_media`, and `chrome=noheader nofooter noborders noscrollbar transparent` (the last one is documented for timelines only). We loaded real embeds with each of them directly in Chromium (screenshots `param-*.png`, contact sheet `param-contact-sheet.png`). Result: on the plain `Tweet.html` iframe URL, `theme=dark` and `dnt=true` work; `width`, `align`, `conversation`, `cards` and `chrome` are ignored (they are `widgets.js` / oEmbed settings, not iframe query parameters); the only thing that visibly changed the card was the internal name `hideCard=true`, which hides the video, so it is useless here. The card is capped at 550 px wide, and no documented option makes the video fill the window. We do not load `widgets.js` into our page to get at those options.

**Display rules, quoted** (docs.x.com "Display requirements: Posts"): "Display real, unmodified posts from real accounts." The author's "profile picture, @username, and display name must always be displayed", the timestamp "must be displayed", "All post Action icons (including reply, repost, and like) must always be visible", and "The official X logo must always be reasonably visible and displayed on the upper-right corner of an individual post". So hiding or cropping the header, footer or actions to get a "video-only" look is not allowed; that is why slopTV never does it. X's own player fullscreen button is the sanctioned way to a video-only view, and it ends when the next post loads (see above).

**Documented widgets.js mode (hidden, off by default).** `embed_use_widgets = true` builds the embed the way X documents it for websites: our controller page loads `https://platform.twitter.com/widgets.js` (X's script, running in our page, never injected into X's iframe) and calls `twttr.widgets.createTweet(id, el, {theme:'dark', align:'center', dnt:true})`, which creates X's official iframe. X then sizes the iframe to the card, so there is no empty white area or inner scrollbar below short posts. It is not video-only (the documented `cards:'hidden'` removes the video, `conversation:'none'` only hides the parent post) and it needs `platform.twitter.com/widgets.js` in addition to the embed itself. Auto-advance, pause, cards, fullscreen and keys work as in the default mode.

**Other options we looked at, and why they are not built in.** (a) X API v2 media variants (`media.fields=variants`) played in our own player: no "must use embed for video" wording was found in the Developer Policy, Agreement or Display Requirements, but each user would need their own paid API key (about $0.005 per Post read plus users, pay per use), we would have to show the full required post elements ourselves (avatar, name, @handle, text, linked timestamp, X logo, actions), keep content current and remove deleted Posts. Not built, and no key ships with this repo. (b) `https://x.com/i/videos/tweet/<id>` is the video-only player page X's own embed uses. It plays X's player at full window with no card, but it is not documented anywhere in docs.x.com, so we do not use it.

**Scaling our iframe (hidden, off by default).** The rules say nothing about rendering a post larger, but they also say "unmodified" and the logo must stay "reasonably visible", so we treat uniform scaling as a gray area, not as clearly allowed. `embed_scale_to_fit = true` in the config scales our iframe element uniformly with a CSS transform (capped at 2.5x, about 1.4x on 1080p), like a larger window scale factor: the whole card, header, actions and logo included, just bigger, nothing hidden, cropped or scripted inside the embed. It does not give a video-only look and the iframe's own white page background below short posts also grows. Leave it off unless you accept that.

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

Copy `config.example.toml` to `~/.config/slop-tv/config.toml` (all keys optional). Embed mode uses `player` (`"video"` default, or `"embed"`), `source_url`, `fallback_url`, `shuffle`, `auto_advance` (default true), `embed_default_secs` (default 0: posts without a known duration wait for you) and `start_fullscreen` (default true) and the hidden `embed_scale_to_fit` and `embed_use_widgets` (both default false); the other keys belong to the raw mode.

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

**Video player (default):**

- `jon-slop.grok.me/videos.json` (feed), fetched by the Rust program.
- `x.com/i/videos/tweet/<id>?autoplay=1` (X's undocumented player page) inside our iframe, plus whatever it loads (X scripts, `video.twimg.com`, possibly cookies in slopTV's private Chromium profile). One extra opaque `no-cors` request to the first clip's page serves as the reachability probe.
- `pbs.twimg.com` poster thumbnails for the cards (lazy `<img>` after the notice is accepted), and `x.com/intent/like` / the post URL only via `xdg-open` on `f` / `o` / View on X.

**Embed mode (`player = "embed"`):**

- `jon-slop.grok.me/videos.json` (feed, ETag polling; plus the page itself as a last-resort fallback), fetched by the Rust program.
- `platform.twitter.com/embed/Tweet.html?id=<id>&dnt=true&theme=dark` and whatever X's player then loads (x.com, X's scripts, `video.twimg.com`, `pbs.twimg.com`, possibly cookies in slopTV's private Chromium profile), loaded by Chromium inside the official embed. This is X's own player and X decides what it fetches.
- `pbs.twimg.com`: poster thumbnails for the previous/next cards, two `<img>` tags loaded lazily by Chromium on our page (not by the Rust program), only after the privacy notice is accepted. Posters come from the feed's `poster` field and are only used if they are on `pbs.twimg.com`.
- With the hidden `embed_use_widgets = true`: also `platform.twitter.com/widgets.js` (X's script, loaded into our page after the notice is accepted), which then builds the same official iframe.
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

58 unit tests use the small fixtures in `tests/fixtures/` (20 records of the live feed, and a trimmed page with the embedded data block for 10). The timer tests run the page's pure timer code under `node` with a fake clock and are skipped when node is not installed. CI runs the same three commands on ubuntu-latest with `libmpv-dev`. The `raw-mpv` cargo feature (on by default) pulls in libmpv2 and image decoding; `cargo build --no-default-features` builds the embed-only controller without libmpv (`install.sh` falls back to that automatically when libmpv is missing). I chose a feature over a separate crate so the default binary and `install.sh` stay a single build.

## Not yet verified on Hyprland

Built and tested on a Debian box (Xvfb, google-chrome-stable): `cargo build --release`, unit tests (with and without the `raw-mpv` feature), the local controller served over HTTP (state, next, ack, quit, token gate), Chromium launching with its private profile and class `slop-tv`, and the process exiting cleanly (Chromium killed, launch file removed). The raw mode was smoke-tested earlier with libmpv under Xvfb.

Not verified: that `--start-fullscreen` and the Hyprland `hyprctl dispatch fullscreen 0` fallback make the window fullscreen on a real Hyprland desktop (checked only with Chrome under Xvfb + xfwm4, where the window did become fullscreen; the `hyprctl` syntax for Omarchy's Hyprland version is untested), that X's embed actually renders and plays inside the window (screenshots under Xvfb show the embed loading its tweet card with the video poster and a play button; actual video playback was not observed), whether autoplay works or needs a click, embed behaviour for deleted or protected posts, anything on Wayland or Hyprland (app-id from `--class`, floating/tiling, `hyprctl` focus and close in `open.sh`), the Omarchy plugin loader, Quickshell loading `Service.qml` and `BarWidget.qml` (including the bar icon click and its default right placement), the bind via `o.bind` and `install-bind.sh` against a real `omarchy menu keybindings --print`, `xdg-open`, and the first-run build on Arch.

## Credits

Feed curated by [jon-slop.grok.me](https://jon-slop.grok.me). Plugin layout follows [omarchy-meshpeek](https://github.com/monomyth/omarchy-meshpeek). The videos belong to their creators; they are shown through X's own embedded player (or, in the hidden raw mode, streamed from video.twimg.com and, with `download_clips`, cached locally for playback only). slopTV does not redistribute them, and this repository contains only a small metadata fixture.

## License

MIT
