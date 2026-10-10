//! Unofficial raw-mpv mode (hidden: `--raw-mpv` or `raw_mpv = true`): libmpv streams the mp4
//! straight from video.twimg.com, with ASS/bitmap overlays for the UI. Feature `raw-mpv`.
use crate::args::Args;
use crate::cache::{self, Cache};
use crate::config::Config;
use crate::{fetch, model, queue, ui};
use anyhow::Result;
use libmpv2::{events::Event, mpv_end_file_reason, Mpv};
use model::Video;
use queue::Queue;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use ui::{Mode, View};

const CHROME_HOLD: f64 = 2.0;
const CHROME_FADE: f64 = 0.4;
const CARD_SECS: f64 = 4.0;
const MAX_FAILS: u32 = 8;

enum Msg {
    Feed(fetch::FeedResult),
    #[allow(dead_code)]
    ClipReady(String),
    ClipFailed(String),
    TileReady,
}

struct TileJob {
    id: String,
    poster: String,
    w: u32,
    h: u32,
}

enum Ev {
    None,
    Key(String),
    End(u32),
    Shutdown,
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ ((std::process::id() as u64) << 32)
}

fn spawn_quiet(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
        .is_ok()
}

fn copy_to_clipboard(text: &str) -> bool {
    match Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut c) => {
            if let Some(mut i) = c.stdin.take() {
                let _ = i.write_all(text.as_bytes());
            }
            std::thread::spawn(move || {
                let _ = c.wait();
            });
            true
        }
        Err(_) => false,
    }
}

struct App {
    mpv: Mpv,
    cfg: Config,
    cache: Cache,
    videos: Vec<Video>,
    queue: Queue,
    cur: Option<usize>,
    mode: Mode,
    help: bool,
    query: String,
    searching: bool,
    focus: usize,
    last_input: Instant,
    card_until: Instant,
    flash: Option<(String, Instant)>,
    offline: bool,
    heard_key: bool,
    fails: u32,
    retry_at: Option<Instant>,
    loop_clip: bool,
    quit: bool,
    last_ass: String,
    overlays: HashMap<i32, (PathBuf, i32, i32)>,
    tile_asked: HashSet<PathBuf>,
    clip_tx: Sender<(String, String)>,
    tile_tx: Sender<TileJob>,
    dims: (f64, f64),
}

const KEYS: &[&str] = &[
    "SPACE",
    "ENTER",
    "ESC",
    "TAB",
    "BS",
    "LEFT",
    "RIGHT",
    "UP",
    "DOWN",
    "/",
    "?",
    ".",
    ",",
    "-",
    "_",
    "@",
    "CLOSE_WIN",
];

impl App {
    fn new(cfg: Config, cache: Cache, args: &Args) -> Result<(App, Receiver<Msg>)> {
        let (clip_tx, clip_rx) = channel::<(String, String)>();
        let (tile_tx, tile_rx) = channel::<TileJob>();
        let (msg_tx, msg_rx) = channel::<Msg>();

        let fullscreen = cfg.fullscreen && !args.windowed;
        let mpv = Mpv::with_initializer(|i| {
            // mpv's libmpv profile defaults to vo=libmpv (render API); we want its own window.
            i.set_property(
                "vo",
                if cfg.vo.is_empty() {
                    "gpu"
                } else {
                    cfg.vo.as_str()
                },
            )?;
            if !cfg.gpu_context.is_empty() {
                i.set_property("gpu-context", cfg.gpu_context.as_str())?;
            }
            if !cfg.ao.is_empty() {
                i.set_property("ao", cfg.ao.as_str())?;
            }
            i.set_property("input-default-bindings", false)?;
            i.set_property("input-builtin-bindings", false)?;
            i.set_property("input-vo-keyboard", true)?;
            i.set_property("osc", false)?;
            i.set_property("osd-level", 0i64)?;
            i.set_property("force-window", "yes")?;
            i.set_property("idle", "yes")?;
            i.set_property("title", "slopTV")?;
            i.set_property("fullscreen", fullscreen)?;
            i.set_property("keep-open", "no")?;
            i.set_property("hwdec", cfg.hwdec.as_str())?;
            i.set_property("mute", true)?;
            i.set_property("volume", cfg.volume as f64)?;
            i.set_property("loop-file", if cfg.loop_clip { "inf" } else { "no" })?;
            i.set_property("cursor-autohide", 2000i64)?;
            i.set_property("osd-font", cfg.font.as_str())?;
            i.set_property("network-timeout", 15i64)?;
            // Never let mpv shell out to yt-dlp: that would load X post pages. Plain mp4 URLs only.
            i.set_property("ytdl", false)?;
            // Streaming: let mpv's demuxer cache do the buffering. The back buffer lets
            // loop-file=inf seek back inside the cache instead of re-downloading short clips.
            i.set_property("cache", "yes")?;
            i.set_property("demuxer-max-bytes", "64MiB")?;
            i.set_property("demuxer-max-back-bytes", "64MiB")?;
            i.set_property("demuxer-readahead-secs", 20i64)?;
            let _ = i.set_property("background", "color");
            let _ = i.set_property("background-color", "#1a1b1c");
            let _ = i.set_property("wayland-app-id", "slop-tv");
            let _ = i.set_property("x11-name", "slop-tv");
            let _ = i.set_property("terminal", false);
            if let Ok(f) = std::env::var("SLOPTV_MPV_LOG") {
                let _ = i.set_property("log-file", f.as_str());
                let _ = i.set_property("msg-level", "all=v");
            }
            if let Some(g) = &args.geometry {
                let _ = i.set_property("geometry", g.as_str());
            }
            Ok(())
        })
        .map_err(|e| anyhow::anyhow!("cannot start libmpv: {e}"))?;

        for k in KEYS {
            let name = if *k == "CLOSE_WIN" { "CLOSE_WIN" } else { k };
            mpv.command("keybind", &[name, &format!("script-message slop-key {k}")])
                .ok();
        }
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            let k = c.to_string();
            mpv.command("keybind", &[&k, &format!("script-message slop-key {k}")])
                .ok();
        }

        // workers
        let cache2 = cache.clone();
        let tx = msg_tx.clone();
        let cap = cfg.cache_max_mb * 1024 * 1024;
        std::thread::spawn(move || {
            while let Ok((id, url)) = clip_rx.recv() {
                let dest = cache2.clip(&id);
                let r = if cache2.has_clip(&id) {
                    Ok(0)
                } else {
                    fetch::download(&url, &dest)
                };
                match r {
                    Ok(_) => {
                        cache2.enforce_cap(cap, &HashSet::new());
                        let _ = tx.send(Msg::ClipReady(id));
                    }
                    Err(e) => {
                        eprintln!("slop-tv: prefetch {id} failed: {e}");
                        let _ = tx.send(Msg::ClipFailed(id));
                    }
                }
            }
        });
        let cache3 = cache.clone();
        let tx = msg_tx.clone();
        std::thread::spawn(move || {
            while let Ok(j) = tile_rx.recv() {
                let out = cache3.tile(&j.id, j.w, j.h);
                if out.exists() {
                    continue;
                }
                let post = cache3.poster(&j.id);
                if !post.exists() && fetch::download(&j.poster, &post).is_err() {
                    continue;
                }
                // keep only the small BGRA tile; the source poster jpg is not needed afterwards
                let ok = fetch::make_tile(&post, &out, j.w, j.h).is_ok();
                let _ = std::fs::remove_file(&post);
                if ok {
                    let _ = tx.send(Msg::TileReady);
                }
            }
        });
        // feed poller: immediately, then every poll_minutes (every minute while offline)
        let cfg2 = cfg.clone();
        let cache4 = cache.clone();
        let tx = msg_tx.clone();
        std::thread::spawn(move || loop {
            let r = fetch::load_feed(&cfg2, &cache4);
            let online = r.online;
            if tx.send(Msg::Feed(r)).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(if online {
                cfg2.poll_minutes.max(1) * 60
            } else {
                60
            }));
        });

        let videos = fetch::read_cached(&cache);
        let now = Instant::now();
        let app = App {
            queue: Queue::new(videos.len(), cfg.shuffle, seed()),
            videos,
            mpv,
            cache,
            cur: None,
            mode: Mode::Player,
            help: false,
            query: String::new(),
            searching: false,
            focus: 0,
            last_input: now - Duration::from_secs(60),
            card_until: now,
            flash: None,
            offline: false,
            heard_key: false,
            fails: 0,
            retry_at: None,
            loop_clip: cfg.loop_clip,
            quit: false,
            last_ass: String::new(),
            overlays: HashMap::new(),
            tile_asked: HashSet::new(),
            clip_tx,
            tile_tx,
            dims: (1920.0, 1080.0),
            cfg,
        };
        Ok((app, msg_rx))
    }

    fn playable(&self, v: &Video) -> bool {
        v.mp4.is_some()
    }

    fn apply_pool(&mut self) {
        let pool: Vec<usize> = self
            .videos
            .iter()
            .enumerate()
            .filter(|(_, v)| self.playable(v) && (!self.offline || self.cache.has_clip(&v.id)))
            .map(|(i, _)| i)
            .collect();
        self.queue.set_pool(pool);
    }

    fn source_for(&self, v: &Video) -> String {
        let src = cache::playback_source(&self.cache, &v.id, v.mp4.as_deref().unwrap_or(""));
        if self.cache.has_clip(&v.id) {
            Cache::touch(&self.cache.clip(&v.id));
        }
        src
    }

    fn play(&mut self, idx: usize) {
        let src = self.source_for(&self.videos[idx]);
        self.cur = Some(idx);
        self.card_until = Instant::now() + Duration::from_secs_f64(CARD_SECS);
        let _ = self.mpv.set_property("pause", false);
        if let Err(e) = self.mpv.command("loadfile", &[&src, "replace"]) {
            eprintln!("slop-tv: loadfile failed: {e}");
        }
        self.prefetch();
    }

    fn prefetch(&mut self) {
        if !self.cfg.download_clips {
            return;
        }
        if let Some(n) = self.queue.peek_next() {
            let v = &self.videos[n];
            if let (Some(url), false) = (&v.mp4, self.cache.has_clip(&v.id)) {
                let _ = self.clip_tx.send((v.id.clone(), url.clone()));
            }
        }
    }

    fn advance(&mut self) {
        match self.queue.next() {
            Some(i) => self.play(i),
            None => self.nothing_to_play(),
        }
    }

    fn nothing_to_play(&mut self) {
        // everything marked bad (expired links or offline with an empty cache): try again later
        self.retry_at = Some(Instant::now() + Duration::from_secs(15));
        let _ = self.mpv.command("stop", &[]);
        self.cur = None;
    }

    fn retreat(&mut self) {
        if let Some(i) = self.queue.prev() {
            self.play(i);
        }
    }

    fn current_video(&self) -> Option<&Video> {
        self.cur.and_then(|i| self.videos.get(i))
    }

    fn apply_feed(&mut self, fr: fetch::FeedResult) {
        self.offline = !fr.online;
        let same = fr.videos.len() == self.videos.len()
            && fr
                .videos
                .iter()
                .zip(&self.videos)
                .all(|(a, b)| a.id == b.id);
        if !same && !fr.videos.is_empty() {
            let cur_id = self.current_video().map(|v| v.id.clone());
            self.videos = fr.videos;
            self.queue = Queue::new(self.videos.len(), self.queue.shuffle(), seed());
            self.cur = None;
            self.apply_pool();
            if let Some(id) = cur_id {
                if let Some(i) = self.videos.iter().position(|v| v.id == id) {
                    self.queue.jump(i);
                    self.cur = Some(i);
                    self.prefetch();
                }
            }
            self.focus = 0;
        } else {
            self.queue.clear_bad();
            self.apply_pool();
        }
        if self.cur.is_none() && self.queue.pool_len() > 0 {
            self.advance();
        }
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Feed(fr) => self.apply_feed(fr),
            Msg::ClipReady(_) => {}
            Msg::ClipFailed(id) => {
                // expired / unreachable link: skip it, pick another next clip, retry later (bad list resets on refresh)
                if let Some(i) = self.videos.iter().position(|v| v.id == id) {
                    self.queue.mark_bad(i);
                    if self.cur != Some(i) {
                        self.prefetch();
                    }
                }
            }
            Msg::TileReady => {}
        }
    }

    fn on_end(&mut self, reason: u32) {
        if reason == mpv_end_file_reason::Error {
            self.fails += 1;
            if let Some(i) = self.cur {
                let id = self.videos[i].id.clone();
                if self.cache.has_clip(&id) {
                    let _ = std::fs::remove_file(self.cache.clip(&id));
                }
                self.queue.mark_bad(i);
            }
            if self.fails >= MAX_FAILS {
                self.fails = 0;
                self.nothing_to_play();
            } else {
                self.advance();
            }
        } else if reason == mpv_end_file_reason::Eof {
            self.fails = 0;
            self.advance();
        }
    }

    fn flash(&mut self, s: &str) {
        self.flash = Some((s.to_string(), Instant::now() + Duration::from_millis(1800)));
    }

    fn set_volume(&mut self, d: f64) {
        let v: f64 = self
            .mpv
            .get_property("volume")
            .unwrap_or(self.cfg.volume as f64);
        let nv = (v + d).clamp(0.0, 130.0);
        let _ = self.mpv.set_property("volume", nv);
        let _ = self.mpv.set_property("mute", false);
    }

    fn filtered(&self) -> Vec<usize> {
        model::filter(&self.videos, &self.query)
    }

    fn open_grid(&mut self) {
        self.mode = Mode::Grid;
        self.searching = false;
        let f = self.filtered();
        self.focus = self
            .cur
            .and_then(|c| f.iter().position(|&i| i == c))
            .unwrap_or(0);
    }

    fn close_grid(&mut self) {
        self.mode = Mode::Player;
        self.searching = false;
        self.query.clear();
        self.clear_overlays();
    }

    fn clear_overlays(&mut self) {
        let ids: Vec<i32> = self.overlays.keys().copied().collect();
        for id in ids {
            let _ = self.mpv.command("overlay-remove", &[&id.to_string()]);
        }
        self.overlays.clear();
    }

    fn key(&mut self, k: &str) {
        self.last_input = Instant::now();
        if !self.heard_key {
            self.heard_key = true;
            let _ = self.mpv.set_property("mute", false);
            if k == "m" {
                return;
            }
        }
        if k == "CLOSE_WIN" {
            self.quit = true;
            return;
        }
        if self.mode == Mode::Grid && self.searching {
            match k {
                "ESC" => {
                    self.query.clear();
                    self.searching = false;
                    self.focus = 0;
                }
                "ENTER" => self.searching = false,
                "BS" => {
                    self.query.pop();
                    self.focus = 0;
                }
                "TAB" => self.close_grid(),
                "SPACE" => {
                    if !self.query.is_empty() && !self.query.ends_with(' ') {
                        self.query.push(' ');
                    }
                }
                s if s.chars().count() == 1 => {
                    self.query.push_str(s);
                    self.focus = 0;
                }
                _ => {}
            }
            return;
        }
        let in_grid = self.mode == Mode::Grid;
        match k {
            "q" => self.quit = true,
            "?" => self.help = !self.help,
            "ESC" => {
                if self.help {
                    self.help = false;
                } else if in_grid {
                    if self.query.is_empty() {
                        self.close_grid();
                    } else {
                        self.query.clear();
                        self.focus = 0;
                    }
                } else if self.mode == Mode::Card {
                    self.mode = Mode::Player;
                }
            }
            "SPACE" => {
                let p: bool = self.mpv.get_property("pause").unwrap_or(false);
                let _ = self.mpv.set_property("pause", !p);
            }
            "m" => {
                let m: bool = self.mpv.get_property("mute").unwrap_or(false);
                let _ = self.mpv.set_property("mute", !m);
            }
            "s" => {
                let on = !self.queue.shuffle();
                self.queue.set_shuffle(on);
                self.prefetch();
            }
            "r" => {
                self.loop_clip = !self.loop_clip;
                let _ = self
                    .mpv
                    .set_property("loop-file", if self.loop_clip { "inf" } else { "no" });
            }
            "n" => self.advance(),
            "f" => {
                let f: bool = self.mpv.get_property("fullscreen").unwrap_or(false);
                let _ = self.mpv.set_property("fullscreen", !f);
            }
            "o" => {
                if let Some(u) = self.target_video().map(|v| v.url.clone()) {
                    let ok = spawn_quiet("xdg-open", &[&u]);
                    self.flash(if ok {
                        "opened in browser"
                    } else {
                        "xdg-open not found"
                    });
                }
            }
            "y" => {
                if let Some(u) = self.target_video().map(|v| v.url.clone()) {
                    let ok = copy_to_clipboard(&u);
                    self.flash(if ok {
                        "link copied"
                    } else {
                        "wl-copy not found"
                    });
                }
            }
            "TAB" => {
                if in_grid {
                    self.close_grid()
                } else {
                    self.open_grid()
                }
            }
            "/" => {
                if !in_grid {
                    self.open_grid();
                }
                self.searching = true;
                self.query.clear();
                self.focus = 0;
            }
            "ENTER" if in_grid => {
                let f = self.filtered();
                if let Some(&i) = f.get(self.focus) {
                    self.close_grid();
                    self.queue.jump(i);
                    self.play(i);
                }
            }
            "i" => {
                if in_grid {
                    return;
                }
                self.mode = if self.mode == Mode::Card {
                    Mode::Player
                } else {
                    Mode::Card
                };
            }
            "I" => {
                if !in_grid {
                    self.card_until = Instant::now() + Duration::from_secs_f64(CARD_SECS);
                }
            }
            "l" | "RIGHT" => {
                if in_grid {
                    self.focus_move(1)
                } else {
                    self.advance()
                }
            }
            "h" | "LEFT" => {
                if in_grid {
                    self.focus_move(-1)
                } else {
                    self.retreat()
                }
            }
            "j" | "DOWN" => {
                if in_grid {
                    self.focus_move(ui::COLS as i64)
                } else {
                    self.set_volume(-5.0)
                }
            }
            "k" | "UP" => {
                if in_grid {
                    self.focus_move(-(ui::COLS as i64))
                } else {
                    self.set_volume(5.0)
                }
            }
            _ => {}
        }
    }

    fn target_video(&self) -> Option<&Video> {
        if self.mode == Mode::Grid {
            let f = self.filtered();
            f.get(self.focus).map(|&i| &self.videos[i])
        } else {
            self.current_video()
        }
    }

    fn focus_move(&mut self, d: i64) {
        let n = self.filtered().len() as i64;
        if n > 0 {
            self.focus = (self.focus as i64 + d).clamp(0, n - 1) as usize;
        }
    }

    fn status(&self) -> String {
        let mut p = vec![];
        if self.mpv.get_property::<bool>("pause").unwrap_or(false) {
            p.push("paused".to_string());
        }
        if self.mpv.get_property::<bool>("mute").unwrap_or(false) {
            p.push("muted".into());
        } else {
            p.push(format!(
                "vol {}",
                self.mpv
                    .get_property::<f64>("volume")
                    .unwrap_or(0.0)
                    .round() as i64
            ));
        }
        p.push(
            if self.queue.shuffle() {
                "shuffle"
            } else {
                "in order"
            }
            .into(),
        );
        p.push(if self.loop_clip { "loop" } else { "advance" }.into());
        if let Some((s, until)) = &self.flash {
            if Instant::now() < *until {
                p.insert(0, s.clone());
            }
        }
        p.join("   ")
    }

    fn render(&mut self) {
        let w: i64 = self.mpv.get_property("osd-width").unwrap_or(0);
        let h: i64 = self.mpv.get_property("osd-height").unwrap_or(0);
        if w > 0 && h > 0 {
            self.dims = (w as f64, h as f64);
        }
        let (w, h) = self.dims;
        let since = self.last_input.elapsed().as_secs_f64();
        let chrome = ui::fade(since, CHROME_HOLD, CHROME_FADE);
        let until = self
            .card_until
            .saturating_duration_since(Instant::now())
            .as_secs_f64();
        let small = if until > 0.0 {
            (until / CHROME_FADE).min(1.0)
        } else {
            0.0
        };
        let pos: f64 = self.mpv.get_property("time-pos").unwrap_or(0.0);
        let dur: f64 = self.mpv.get_property("duration").unwrap_or(0.0);
        let progress = if dur > 0.0 { pos / dur } else { 0.0 };
        let notice = if self.offline {
            Some(format!("Can't reach {}", self.cfg.host()))
        } else {
            None
        };
        let status = if chrome > 0.0 {
            self.status()
        } else {
            String::new()
        };

        let filtered = self.filtered();
        let items: Vec<&Video> = filtered.iter().map(|&i| &self.videos[i]).collect();
        let cache = self.cache.clone();
        let g = ui::grid_geom(w, h);
        let (tw, th) = (g.tile_w as u32, g.tile_h as u32);
        let ready = move |v: &Video| cache.tile(&v.id, tw, th).exists();
        let grid = if self.mode == Mode::Grid {
            Some(ui::GridView {
                items: &items,
                focus: self.focus,
                query: &self.query,
                searching: self.searching,
                tile_ready: &ready,
            })
        } else {
            None
        };
        let view = View {
            w,
            h,
            font: &self.cfg.font,
            mode: self.mode,
            chrome,
            small_card: small,
            video: self.cur.and_then(|i| self.videos.get(i)),
            progress,
            status,
            notice,
            grid,
            help: self.help,
        };
        let ass = ui::render(&view);
        drop(view);
        if ass != self.last_ass {
            let _ = self.mpv.command(
                "osd-overlay",
                &[
                    "1",
                    "ass-events",
                    &ass,
                    &(w as i64).to_string(),
                    &(h as i64).to_string(),
                ],
            );
            self.last_ass = ass;
        }
        if self.mode == Mode::Grid {
            self.sync_tiles(&filtered, w, h);
        }
    }

    fn sync_tiles(&mut self, filtered: &[usize], w: f64, h: f64) {
        let tiles = ui::visible_tiles(w, h, self.focus, filtered.len());
        let mut want: HashMap<i32, (PathBuf, i32, i32, u32, u32)> = HashMap::new();
        for (slot, (idx, t)) in tiles.iter().enumerate() {
            let v = &self.videos[filtered[*idx]];
            let (tw, th) = (t.w as u32, t.h as u32);
            let path = self.cache.tile(&v.id, tw, th);
            if path.exists() {
                want.insert(slot as i32, (path, t.x as i32, t.y as i32, tw, th));
            } else if let Some(p) = &v.poster {
                if self.tile_asked.insert(path) {
                    let _ = self.tile_tx.send(TileJob {
                        id: v.id.clone(),
                        poster: p.clone(),
                        w: tw,
                        h: th,
                    });
                }
            }
        }
        // small margin: poster jobs for the next row after the visible page (not drawn yet)
        let after = tiles.last().map(|(i, _)| i + 1).unwrap_or(0);
        if let Some((_, t0)) = tiles.first() {
            let (tw, th) = (t0.w as u32, t0.h as u32);
            for &vi in filtered.iter().skip(after).take(ui::COLS) {
                let v = &self.videos[vi];
                let path = self.cache.tile(&v.id, tw, th);
                if let (false, Some(p)) = (path.exists(), &v.poster) {
                    if self.tile_asked.insert(path) {
                        let _ = self.tile_tx.send(TileJob {
                            id: v.id.clone(),
                            poster: p.clone(),
                            w: tw,
                            h: th,
                        });
                    }
                }
            }
        }
        let stale: Vec<i32> = self
            .overlays
            .keys()
            .filter(|k| !want.contains_key(k))
            .copied()
            .collect();
        for k in stale {
            let _ = self.mpv.command("overlay-remove", &[&k.to_string()]);
            self.overlays.remove(&k);
        }
        for (slot, (path, x, y, tw, th)) in want {
            if self.overlays.get(&slot) != Some(&(path.clone(), x, y)) {
                let r = self.mpv.command(
                    "overlay-add",
                    &[
                        &slot.to_string(),
                        &x.to_string(),
                        &y.to_string(),
                        &path.to_string_lossy(),
                        "0",
                        "bgra",
                        &tw.to_string(),
                        &th.to_string(),
                        &(tw * 4).to_string(),
                    ],
                );
                if r.is_ok() {
                    self.overlays.insert(slot, (path, x, y));
                }
            }
        }
    }

    fn poll(&mut self) -> Ev {
        match self.mpv.wait_event(0.03) {
            Some(Ok(Event::ClientMessage(a))) if a.len() >= 2 && a[0] == "slop-key" => {
                Ev::Key(a[1].to_string())
            }
            Some(Ok(Event::EndFile(r))) => Ev::End(r),
            Some(Ok(Event::Shutdown)) => Ev::Shutdown,
            _ => Ev::None,
        }
    }

    fn run(&mut self, rx: Receiver<Msg>) {
        let _ = self
            .cache
            .enforce_cap(self.cfg.cache_max_mb * 1024 * 1024, &HashSet::new());
        if self.queue.len() > 0 {
            self.apply_pool();
            self.advance();
        }
        while !self.quit {
            match self.poll() {
                Ev::Key(k) => self.key(&k),
                Ev::End(r) => self.on_end(r),
                Ev::Shutdown => break,
                Ev::None => {}
            }
            while let Ok(m) = rx.try_recv() {
                self.handle(m);
            }
            if let Some(t) = self.retry_at {
                if Instant::now() >= t {
                    self.retry_at = None;
                    self.queue.clear_bad();
                    self.fails = 0;
                    self.advance();
                }
            }
            self.render();
        }
        let _ = self.mpv.command("quit", &[]);
    }
}

pub fn run(cfg: Config, cache: Cache, args: &Args) -> Result<()> {
    let (mut app, rx) = App::new(cfg, cache, args)?;
    app.run(rx);
    Ok(())
}
