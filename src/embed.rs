//! Default mode: the Rust binary is a small controller (feed, queue, shuffle, open-on-X) that
//! serves our own local page on 127.0.0.1 and shows it in a Chromium app window. The page holds
//! X's official, unmodified embed in an iframe (platform.twitter.com/embed/Tweet.html?id=..&dnt=true)
//! plus our own prev/next buttons and keys. Nothing is injected into the embed and no cookies are
//! touched by slopTV; Chromium runs with its own dedicated profile directory.
use crate::cache::Cache;
use crate::config::Config;
use crate::fetch;
use crate::model::Video;
use crate::queue::Queue;
use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("embed.html");
const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave",
];

/// Official embed URL for a post id. Only plain numeric ids are accepted.
pub fn embed_url(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 25 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "https://platform.twitter.com/embed/Tweet.html?id={id}&dnt=true&theme=dark"
    ))
}

/// X's own like-intent page for a post; the user confirms there with their own login.
pub fn like_url(id: &str) -> Option<String> {
    embed_url(id)?; // same numeric-id guard
    Some(format!("https://x.com/intent/like?tweet_id={id}"))
}

/// `o` only ever opens an X / Twitter post URL.
pub fn is_x_post_url(u: &str) -> bool {
    [
        "https://x.com/",
        "https://twitter.com/",
        "https://www.x.com/",
    ]
    .iter()
    .any(|p| u.starts_with(p))
        && !u.contains(char::is_whitespace)
}

/// Poster thumbnails are only ever loaded from X's image CDN, as a plain <img>.
pub fn safe_poster(u: &str) -> Option<&str> {
    (u.starts_with("https://pbs.twimg.com/") && !u.contains(['"', '\'', '<', '>', ' ', '\\']))
        .then_some(u)
}

/// How long to wait before auto-advancing, from the feed's known duration (seconds).
pub fn advance_secs(duration_ms: Option<u64>, default_secs: u64) -> Option<u64> {
    match duration_ms {
        Some(ms) if ms > 0 => Some(ms.div_ceil(1000)),
        _ if default_secs > 0 => Some(default_secs),
        _ => None,
    }
}

fn snippet(s: &str, max: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        let mut t: String = one.chars().take(max).collect();
        t.push('\u{2026}');
        t
    }
}

pub fn notice_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("slop-tv/embed-notice-ack")
}

pub enum Action {
    None,
    Open(String),
    Quit,
}

pub struct Resp {
    pub status: u16,
    pub ctype: &'static str,
    pub body: String,
    pub action: Action,
}

fn json(body: String) -> Resp {
    Resp {
        status: 200,
        ctype: "application/json",
        body,
        action: Action::None,
    }
}
fn plain(status: u16, body: &str) -> Resp {
    Resp {
        status,
        ctype: "text/plain",
        body: body.into(),
        action: Action::None,
    }
}

pub struct Ctl {
    pub videos: Vec<Video>,
    pub queue: Queue,
    pub cur: Option<usize>,
    pub notice_file: PathBuf,
    pub token: String,
    pub last_ping: Option<Instant>,
    pub auto_advance: bool,
    pub default_secs: u64,
    pub scale_to_fit: bool,
    pub use_widgets: bool,
}

impl Ctl {
    pub fn new(videos: Vec<Video>, shuffle: bool, seed: u64, notice_file: PathBuf) -> Ctl {
        let mut queue = Queue::new(videos.len(), shuffle, seed);
        let pool = videos
            .iter()
            .enumerate()
            .filter(|(_, v)| embed_url(&v.id).is_some())
            .map(|(i, _)| i)
            .collect();
        queue.set_pool(pool);
        let mut c = Ctl {
            videos,
            queue,
            cur: None,
            notice_file,
            token: String::new(),
            last_ping: None,
            auto_advance: true,
            default_secs: 0,
            scale_to_fit: false,
            use_widgets: false,
        };
        c.cur = c.queue.next();
        c
    }

    pub fn notice_pending(&self) -> bool {
        !self.notice_file.exists()
    }

    fn card(&self, i: Option<usize>, wraps: bool, notice: bool) -> serde_json::Value {
        let Some(v) = i.and_then(|i| self.videos.get(i)) else {
            return serde_json::Value::Null;
        };
        let poster = if notice {
            None
        } else {
            v.poster.as_deref().and_then(safe_poster)
        };
        serde_json::json!({
            "creator": v.creator,
            "caption": snippet(&v.caption, 70),
            "poster": poster,
            "wraps": wraps,
        })
    }

    fn state(&mut self) -> String {
        let notice = self.notice_pending();
        let next = self.queue.peek_next();
        let prev = self.queue.peek_prev();
        let wraps = self.queue.next_wraps();
        let prev_card = self.card(prev, false, notice);
        let next_card = self.card(next, wraps, notice);
        let v = self.cur.and_then(|i| self.videos.get(i));
        serde_json::json!({
            "id": v.map(|v| v.id.as_str()).unwrap_or(""),
            "embed": v.and_then(|v| embed_url(&v.id)).unwrap_or_default(),
            "creator": v.map(|v| v.creator.as_str()).unwrap_or(""),
            "caption": v.map(|v| v.caption.as_str()).unwrap_or(""),
            "shuffle": self.queue.shuffle(),
            "notice": notice,
            "pos": self.cur.unwrap_or(0),
            "total": self.videos.len(),
            "auto": self.auto_advance,
            "scale_fit": self.scale_to_fit,
            "widgets": self.use_widgets,
            "duration_secs": v.and_then(|v| advance_secs(v.duration_ms, self.default_secs)),
            "prev": prev_card,
            "next": next_card,
        })
        .to_string()
    }

    /// Route one request. `path` must start with `/t/<token>/`; anything else is a 404.
    pub fn handle(&mut self, method: &str, path: &str) -> Resp {
        let prefix = format!("/t/{}/", self.token);
        let Some(rest) = path.split('?').next().and_then(|p| p.strip_prefix(&prefix)) else {
            return plain(404, "not found");
        };
        let mutating = matches!(
            rest,
            "api/next"
                | "api/prev"
                | "api/open"
                | "api/shuffle"
                | "api/auto"
                | "api/like"
                | "api/ack"
                | "api/quit"
        );
        if mutating && method != "POST" {
            return plain(405, "POST only");
        }
        match rest {
            "" | "index.html" => Resp {
                status: 200,
                ctype: "text/html; charset=utf-8",
                body: PAGE.into(),
                action: Action::None,
            },
            "api/state" => json(self.state()),
            "api/ping" => {
                self.last_ping = Some(Instant::now());
                plain(204, "")
            }
            "api/next" => {
                if let Some(i) = self.queue.next() {
                    self.cur = Some(i);
                }
                json(self.state())
            }
            "api/prev" => {
                if let Some(i) = self.queue.prev() {
                    self.cur = Some(i);
                }
                json(self.state())
            }
            "api/shuffle" => {
                let on = !self.queue.shuffle();
                self.queue.set_shuffle(on);
                json(self.state())
            }
            "api/auto" => {
                self.auto_advance = !self.auto_advance;
                json(self.state())
            }
            "api/ack" => {
                if let Some(d) = self.notice_file.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&self.notice_file, "acknowledged\n");
                json(self.state())
            }
            "api/open" => {
                let url = self
                    .cur
                    .and_then(|i| self.videos.get(i))
                    .map(|v| v.url.clone())
                    .filter(|u| is_x_post_url(u));
                let mut r = json(self.state());
                if let Some(u) = url {
                    r.action = Action::Open(u);
                }
                r
            }
            "api/like" => {
                let url = self
                    .cur
                    .and_then(|i| self.videos.get(i))
                    .and_then(|v| like_url(&v.id));
                let mut r = json(self.state());
                if let Some(u) = url {
                    r.action = Action::Open(u);
                }
                r
            }
            "api/quit" => {
                let mut r = plain(204, "");
                r.action = Action::Quit;
                r
            }
            _ => plain(404, "not found"),
        }
    }
}

fn token() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    b.iter().fold(String::new(), |mut a, x| {
        use std::fmt::Write;
        let _ = write!(a, "{x:02x}");
        a
    })
}

fn serve_conn(mut s: TcpStream, ctl: &Mutex<Ctl>, quit: &Mutex<bool>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut buf = [0u8; 2048];
    let n = s.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let mut it = req.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    if method.is_empty() {
        return;
    }
    let r = ctl.lock().unwrap().handle(method, path);
    let reason = match r.status {
        200 => "OK",
        204 => "No Content",
        405 => "Method Not Allowed",
        _ => "Not Found",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        r.status,
        r.ctype,
        r.body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(r.body.as_bytes());
    match r.action {
        Action::Open(u) => {
            let _ = Command::new("xdg-open")
                .arg(u)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|mut c| std::thread::spawn(move || c.wait()));
        }
        Action::Quit => *quit.lock().unwrap() = true,
        Action::None => {}
    }
}

pub fn find_browser() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    BROWSERS
        .iter()
        .find(|b| std::env::split_paths(&path).any(|d| d.join(b).is_file()))
        .map(|b| b.to_string())
}

/// Chromium arguments for the app window (pure, for tests).
pub fn browser_args(profile: &Path, launch_url: &str, fullscreen: bool) -> Vec<String> {
    let mut v = vec![
        format!("--user-data-dir={}", profile.display()),
        "--class=slop-tv".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--autoplay-policy=no-user-gesture-required".into(),
        "--window-size=720,900".into(),
        "--new-window".into(),
        format!("--app={launch_url}"),
    ];
    if fullscreen {
        v.push("--start-fullscreen".into());
    }
    v
}

pub fn profile_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("omarchy/sloptv/chromium-profile")
}

fn launch(url: &str, boot: &Path, fullscreen: bool) -> Result<Option<Child>> {
    // bootstrap file redirects to the local URL, so the token stays out of the browser's command line
    std::fs::write(
        boot,
        format!("<!doctype html><meta http-equiv=\"refresh\" content=\"0;url={url}\"><title>slopTV</title>"),
    )?;
    let boot_url = format!("file://{}", boot.display());
    if let Some(b) = find_browser() {
        let profile = profile_dir();
        std::fs::create_dir_all(&profile)?;
        let c = Command::new(b)
            .args(browser_args(&profile, &boot_url, fullscreen))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        return Ok(Some(c));
    }
    eprintln!(
        "slop-tv: no chromium/chrome found, using xdg-open (default browser, not an app window)"
    );
    Command::new("xdg-open")
        .arg(&boot_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("cannot open a browser: {e} (install chromium)"))?;
    Ok(None)
}

/// On Hyprland, make sure the app window really is fullscreen: --start-fullscreen is only a request
/// and a compositor may ignore it. Polls briefly for the `slop-tv` window, then dispatches
/// `fullscreen 0` once if it is not already fullscreen. Does nothing outside Hyprland.
fn hyprland_fullscreen() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    std::thread::spawn(|| {
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(500));
            let Ok(out) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
                return;
            };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
                continue;
            };
            let Some(w) = v
                .as_array()
                .and_then(|a| a.iter().find(|c| c["class"] == "slop-tv"))
            else {
                continue;
            };
            let fs = w["fullscreen"].as_i64().unwrap_or(0) > 0 || w["fullscreen"] == true;
            if !fs {
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "fullscreen", "0"])
                    .output();
            }
            return;
        }
    });
}

pub fn run(cfg: Config, cache: Cache, reset_notice: bool) -> Result<()> {
    let notice = notice_path();
    if reset_notice {
        let _ = std::fs::remove_file(&notice);
    }
    let fr = fetch::load_feed(&cfg, &cache);
    if fr.videos.is_empty() {
        return Err(anyhow!("no videos (can't reach {})", cfg.host()));
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ ((std::process::id() as u64) << 32);
    let mut ctl = Ctl::new(fr.videos, cfg.shuffle, seed, notice);
    ctl.token = token();
    ctl.auto_advance = cfg.auto_advance;
    ctl.default_secs = cfg.embed_default_secs;
    ctl.scale_to_fit = cfg.embed_scale_to_fit;
    ctl.use_widgets = cfg.embed_use_widgets;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let url = format!(
        "http://127.0.0.1:{}/t/{}/",
        listener.local_addr()?.port(),
        ctl.token
    );
    let boot = std::env::temp_dir().join(format!("slop-tv-launch-{}.html", std::process::id()));
    let mut child = launch(&url, &boot, cfg.start_fullscreen)?;
    if cfg.start_fullscreen {
        hyprland_fullscreen();
    }
    let ctl = Arc::new(Mutex::new(ctl));
    let quit = Arc::new(Mutex::new(false));
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((s, _)) => {
                let (c, q) = (ctl.clone(), quit.clone());
                std::thread::spawn(move || serve_conn(s, &c, &q));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
        if *quit.lock().unwrap() {
            break;
        }
        let ping = ctl.lock().unwrap().last_ping;
        let gone = match ping {
            Some(t) => t.elapsed() > Duration::from_secs(8),
            None => started.elapsed() > Duration::from_secs(90),
        };
        let exited = child
            .as_mut()
            .map(|c| {
                matches!(c.try_wait(), Ok(Some(_)))
                    && ping
                        .map(|t| t.elapsed() > Duration::from_secs(5))
                        .unwrap_or(true)
            })
            .unwrap_or(false);
        if gone || exited {
            break;
        }
    }
    if let Some(mut c) = child {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_file(&boot);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vid(id: &str) -> Video {
        Video {
            id: id.into(),
            url: format!("https://x.com/a/status/{id}"),
            creator: "a".into(),
            name: None,
            caption: "c".into(),
            posted_at: String::new(),
            mp4: None,
            poster: None,
            duration_ms: None,
            likes: None,
            views: None,
        }
    }
    fn ctl() -> Ctl {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let f = std::env::temp_dir().join(format!("slop-ack-{}-{n}", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let mut c = Ctl::new(
            vec![vid("1"), vid("2"), vid("bad id"), vid("3")],
            false,
            1,
            f,
        );
        c.token = "tok".into();
        c
    }
    #[test]
    fn embed_url_is_official_and_dnt() {
        assert_eq!(
            embed_url("2108935501126271427").unwrap(),
            "https://platform.twitter.com/embed/Tweet.html?id=2108935501126271427&dnt=true&theme=dark"
        );
        assert!(embed_url("").is_none());
        assert!(embed_url("12&x=1").is_none());
        assert!(embed_url("../1").is_none());
    }
    #[test]
    fn open_only_x_urls() {
        assert!(is_x_post_url("https://x.com/a/status/1"));
        assert!(!is_x_post_url("https://evil.example/x.com/"));
        assert!(!is_x_post_url("file:///etc/passwd"));
    }
    #[test]
    fn browser_args_use_private_profile_and_class() {
        let a = browser_args(Path::new("/p"), "file:///b.html", false);
        assert!(a.contains(&"--user-data-dir=/p".to_string()));
        assert!(a.contains(&"--class=slop-tv".to_string()));
        assert!(a.contains(&"--app=file:///b.html".to_string()));
        assert!(!a.contains(&"--start-fullscreen".to_string()));
        let f = browser_args(Path::new("/p"), "file:///b.html", true);
        assert!(
            f.contains(&"--start-fullscreen".to_string()),
            "start_fullscreen=true adds the flag"
        );
        assert!(profile_dir().ends_with("omarchy/sloptv/chromium-profile"));
    }
    #[test]
    fn token_gate_and_post_only() {
        let mut c = ctl();
        assert_eq!(c.handle("GET", "/").status, 404);
        assert_eq!(c.handle("GET", "/t/wrong/api/state").status, 404);
        assert_eq!(c.handle("GET", "/t/tok/").status, 200);
        assert_eq!(c.handle("GET", "/t/tok/api/next").status, 405);
        assert_eq!(c.handle("POST", "/t/tok/api/next").status, 200);
    }
    #[test]
    fn queue_next_prev_skips_bad_ids_and_open_is_x_url() {
        let mut c = ctl();
        let first = c.cur.unwrap();
        assert_eq!(c.videos[first].id, "1");
        c.handle("POST", "/t/tok/api/next");
        assert_eq!(c.videos[c.cur.unwrap()].id, "2");
        c.handle("POST", "/t/tok/api/next");
        assert_eq!(c.videos[c.cur.unwrap()].id, "3", "non-numeric id skipped");
        c.handle("POST", "/t/tok/api/prev");
        assert_eq!(c.videos[c.cur.unwrap()].id, "2");
        let r = c.handle("POST", "/t/tok/api/open");
        assert!(matches!(r.action, Action::Open(u) if u == "https://x.com/a/status/2"));
        let st: serde_json::Value =
            serde_json::from_str(&c.handle("GET", "/t/tok/api/state").body).unwrap();
        assert_eq!(
            st["embed"],
            "https://platform.twitter.com/embed/Tweet.html?id=2&dnt=true&theme=dark"
        );
    }
    #[test]
    fn notice_shown_once_then_remembered() {
        let mut c = ctl();
        assert!(c.notice_pending());
        let st: serde_json::Value =
            serde_json::from_str(&c.handle("GET", "/t/tok/api/state").body).unwrap();
        assert_eq!(st["notice"], true);
        c.handle("POST", "/t/tok/api/ack");
        assert!(!c.notice_pending());
        let _ = std::fs::remove_file(&c.notice_file);
    }
    fn state(c: &mut Ctl) -> serde_json::Value {
        serde_json::from_str(&c.handle("GET", "/t/tok/api/state").body).unwrap()
    }
    #[test]
    fn advance_secs_from_feed_duration_or_default() {
        assert_eq!(advance_secs(Some(163_640), 0), Some(164));
        assert_eq!(advance_secs(Some(8_000), 30), Some(8));
        assert_eq!(
            advance_secs(None, 0),
            None,
            "unknown + no default: manual only"
        );
        assert_eq!(advance_secs(Some(0), 0), None);
        assert_eq!(advance_secs(None, 20), Some(20));
    }
    #[test]
    fn state_carries_duration_and_auto_flag() {
        let mut c = ctl();
        c.videos[0].duration_ms = Some(12_500);
        c.default_secs = 0;
        let st = state(&mut c);
        assert_eq!(st["duration_secs"], 13);
        assert_eq!(st["auto"], true);
        c.handle("POST", "/t/tok/api/auto");
        assert_eq!(state(&mut c)["auto"], false);
        assert_eq!(c.handle("GET", "/t/tok/api/auto").status, 405);
        c.handle("POST", "/t/tok/api/next");
        assert!(state(&mut c)["duration_secs"].is_null(), "unknown duration");
    }
    #[test]
    fn preview_cards_prev_next_and_wrap_marker() {
        let mut c = ctl(); // ids 1, 2, (bad), 3 in order
        c.videos[1].poster = Some("https://pbs.twimg.com/amplify_video_thumb/1/img/a.jpg".into());
        c.videos[3].poster = Some("https://evil.example/x.jpg".into());
        c.handle("POST", "/t/tok/api/ack");
        let st = state(&mut c); // on id 1
        assert!(st["prev"].is_null());
        assert_eq!(
            st["next"]["poster"],
            "https://pbs.twimg.com/amplify_video_thumb/1/img/a.jpg"
        );
        assert_eq!(st["next"]["wraps"], false);
        c.handle("POST", "/t/tok/api/next"); // id 2
        let st = state(&mut c);
        assert_eq!(st["prev"]["creator"], "a");
        assert!(st["next"]["poster"].is_null(), "non-pbs poster dropped");
        c.handle("POST", "/t/tok/api/next"); // id 3, last
        let st = state(&mut c);
        assert_eq!(st["next"]["wraps"], true, "next loops to start");
        c.handle("POST", "/t/tok/api/next");
        assert_eq!(
            state(&mut c)["id"],
            "1",
            "wraps to the first only after the end"
        );
        let _ = std::fs::remove_file(&c.notice_file);
    }
    #[test]
    fn no_poster_urls_before_notice_is_acknowledged() {
        let mut c = ctl();
        c.videos[1].poster = Some("https://pbs.twimg.com/a.jpg".into());
        assert!(state(&mut c)["next"]["poster"].is_null());
    }
    #[test]
    fn snippet_and_poster_validation() {
        assert_eq!(snippet("a  b\nc", 10), "a b c");
        assert!(snippet(&"x".repeat(100), 70).ends_with('\u{2026}'));
        assert!(safe_poster("https://pbs.twimg.com/a.jpg").is_some());
        assert!(safe_poster("https://pbs.twimg.com/a\".jpg").is_none());
        assert!(safe_poster("http://pbs.twimg.com/a.jpg").is_none());
    }
    #[test]
    fn like_opens_intent_url_for_numeric_ids_only() {
        assert_eq!(
            like_url("123").unwrap(),
            "https://x.com/intent/like?tweet_id=123"
        );
        assert!(like_url("1&x=2").is_none() && like_url("").is_none());
        let mut c = ctl();
        assert_eq!(c.handle("GET", "/t/tok/api/like").status, 405);
        let r = c.handle("POST", "/t/tok/api/like");
        assert!(matches!(r.action, Action::Open(u) if u == "https://x.com/intent/like?tweet_id=1"));
        let _ = std::fs::remove_file(&c.notice_file);
    }
    /// Run the page's pure timer logic (between TIMER-BEGIN/END) under node with a fake clock.
    /// Returns None (test skipped) when node is not installed.
    fn run_timer_js(test: &str) -> Option<String> {
        let a = PAGE.find("// TIMER-BEGIN")?;
        let b = PAGE.find("// TIMER-END")?;
        let js = format!("{}\n{}", &PAGE[a..b], test);
        let out = Command::new("node").arg("-e").arg(js).output().ok()?;
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    #[test]
    fn timer_fires_after_each_clips_own_duration_plus_one_second() {
        let r = run_timer_js(
            r#"
            const out = [];
            for (const d of [4, 13, 164, 1025]) {
              const t = makeTimer(1000);
              t.start(d, true, 5000);                 // clip shown at t=5000
              const early = t.due(5000 + d * 1000 + 999);
              const on = t.due(5000 + d * 1000 + 1000);
              const again = t.due(5000 + d * 1000 + 5000);
              out.push([d, early, on, again].join(':'));
            }
            const n = makeTimer(1000); n.start(0, true, 0);      // no duration: never fires
            const o = makeTimer(1000); o.start(10, false, 0);    // auto-advance off: never fires
            out.push(n.due(1e9), o.due(1e9));
            // next/prev resets: a new start() gives the new clip its own full countdown
            const t = makeTimer(1000); t.start(10, true, 0); t.start(3, true, 8000);
            out.push(t.due(8000 + 3999), t.due(8000 + 4000));
            console.log(out.join(' '));
            "#,
        );
        if let Some(r) = r {
            assert_eq!(
                r,
                "4:false:true:false 13:false:true:false 164:false:true:false 1025:false:true:false false false false true"
            );
        }
    }
    #[test]
    fn pause_freezes_and_resume_continues_from_remaining_time() {
        let r = run_timer_js(
            r#"
            const out = [];
            const t = makeTimer(1000);
            t.start(10, true, 0);                       // total 11000 ms
            t.pause(4000);                              // 7000 ms left
            out.push(t.left(4000), t.left(90000), t.due(90000));   // frozen, never due
            t.resume(50000);                            // continues: 7000 ms left, not 11000
            out.push(t.left(50000), t.due(56999), t.due(57000));
            // pausing twice or resuming when not paused changes nothing
            const u = makeTimer(1000); u.start(10, true, 0);
            u.resume(500); u.pause(2000); u.pause(9000); u.resume(10000);
            out.push(u.left(10000));
            // pause on a timer that is not running (no duration) is a no-op for the timer
            const v = makeTimer(1000); v.start(0, true, 0); v.pause(5); out.push(v.paused, v.due(1e9));
            console.log(out.join(' '));
            "#,
        );
        if let Some(r) = r {
            assert_eq!(r, "7000 7000 false 7000 false true 9000 false false");
        }
    }
    #[test]
    fn page_keys_w_fullscreen_f_like_and_no_chrome() {
        assert!(PAGE.contains("k === 'w') fullscreen()"));
        assert!(PAGE.contains("k === 'f') { go('like')"), "f is Like");
        assert!(!PAGE.contains("F11") && !PAGE.contains("k === 'f') fullscreen"));
        assert!(PAGE.contains("theme=dark") || embed_url("1").unwrap().contains("theme=dark"));
        // no visible controls or help panel in the page: only the two cards and the notice buttons
        assert_eq!(PAGE.matches("<button").count(), 4);
        assert!(!PAGE.contains("id=\"help\"") && !PAGE.contains("id=\"bar\""));
        assert!(PAGE.contains("height:2px"), "thin progress line");
    }
    #[test]
    fn iframe_allows_fullscreen_and_timer_survives_it() {
        assert!(PAGE.contains(
            "allow=\"fullscreen; autoplay; encrypted-media; picture-in-picture\" allowfullscreen"
        ));
        assert!(
            PAGE.contains("fullscreenchange")
                && PAGE.contains("document.fullscreenElement === frame()")
        );
        assert!(
            PAGE.contains("!inFs && document.activeElement"),
            "the fullscreen click must not pause"
        );
        // one iframe element for the whole session: only its src changes
        assert_eq!(PAGE.matches("<iframe").count(), 1);
        assert!(PAGE.contains("$('f').src = s.embed"));
        // no browser flag that would block (or force) iframe fullscreen
        let a = browser_args(Path::new("/p"), "file:///b.html", false).join(" ");
        assert!(
            !a.contains("fullscreen") && !a.contains("disable-features") && !a.contains("kiosk")
        );
    }
    #[test]
    fn scale_to_fit_is_off_by_default_and_page_only_scales_the_iframe_element() {
        let mut c = ctl();
        assert_eq!(state(&mut c)["scale_fit"], false);
        c.scale_to_fit = true;
        assert_eq!(state(&mut c)["scale_fit"], true);
        // uniform transform on our own iframe element only; never reaches into the embed
        assert!(PAGE.contains("transform = 'scale("));
        assert!(!PAGE.contains("zoom") && !PAGE.contains("contentWindow"));
    }
    #[test]
    fn page_loads_no_x_scripts_and_no_injection() {
        assert!(
            !PAGE.contains("Tweet.html"),
            "embed URL comes from the server only"
        );
        assert!(!PAGE.contains("<script src"));
        assert!(!PAGE.contains("contentWindow") && !PAGE.contains("contentDocument"));
    }

    #[test]
    fn widgets_mode_is_off_by_default_and_uses_only_documented_calls() {
        let mut ctl = Ctl::new(vec![], false, 1, PathBuf::from("/nonexistent/notice"));
        assert!(!ctl.use_widgets);
        assert!(ctl.state().contains("\"widgets\":false"));
        // widgets.js is loaded lazily from X's documented URL, only via createTweet with documented options
        assert!(PAGE.contains("https://platform.twitter.com/widgets.js"));
        assert!(PAGE.contains(
            "twttr.widgets.createTweet(id, box, {theme: 'dark', align: 'center', dnt: true})"
        ));
        assert!(!PAGE.contains("<script src"));
        assert!(!PAGE.contains("contentWindow") && !PAGE.contains("contentDocument"));
        // never touches X's iframe or cookies
        assert!(!PAGE.contains("document.cookie"));
        // the iframe X builds is only measured/focused, never styled by us in widgets mode
        assert!(PAGE.contains("#stage.w #f{display:none}"));
    }
}
