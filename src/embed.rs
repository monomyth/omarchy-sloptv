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
        "https://platform.twitter.com/embed/Tweet.html?id={id}&dnt=true"
    ))
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
        };
        c.cur = c.queue.next();
        c
    }

    pub fn notice_pending(&self) -> bool {
        !self.notice_file.exists()
    }

    fn state(&self) -> String {
        let v = self.cur.and_then(|i| self.videos.get(i));
        serde_json::json!({
            "id": v.map(|v| v.id.as_str()).unwrap_or(""),
            "embed": v.and_then(|v| embed_url(&v.id)).unwrap_or_default(),
            "creator": v.map(|v| v.creator.as_str()).unwrap_or(""),
            "caption": v.map(|v| v.caption.as_str()).unwrap_or(""),
            "shuffle": self.queue.shuffle(),
            "notice": self.notice_pending(),
            "pos": self.cur.unwrap_or(0),
            "total": self.videos.len(),
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
            "api/next" | "api/prev" | "api/open" | "api/shuffle" | "api/ack" | "api/quit"
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
pub fn browser_args(profile: &Path, launch_url: &str) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile.display()),
        "--class=slop-tv".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--autoplay-policy=no-user-gesture-required".into(),
        "--window-size=720,900".into(),
        "--new-window".into(),
        format!("--app={launch_url}"),
    ]
}

pub fn profile_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("omarchy/sloptv/chromium-profile")
}

fn launch(url: &str, boot: &Path) -> Result<Option<Child>> {
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
            .args(browser_args(&profile, &boot_url))
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
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let url = format!(
        "http://127.0.0.1:{}/t/{}/",
        listener.local_addr()?.port(),
        ctl.token
    );
    let boot = std::env::temp_dir().join(format!("slop-tv-launch-{}.html", std::process::id()));
    let mut child = launch(&url, &boot)?;
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
        let f = std::env::temp_dir().join(format!("slop-ack-{}-{:p}", std::process::id(), &0));
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
            "https://platform.twitter.com/embed/Tweet.html?id=2108935501126271427&dnt=true"
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
        let a = browser_args(Path::new("/p"), "file:///b.html");
        assert!(a.contains(&"--user-data-dir=/p".to_string()));
        assert!(a.contains(&"--class=slop-tv".to_string()));
        assert!(a.contains(&"--app=file:///b.html".to_string()));
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
            "https://platform.twitter.com/embed/Tweet.html?id=2&dnt=true"
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
    #[test]
    fn page_loads_no_x_scripts_and_no_injection() {
        assert!(
            !PAGE.contains("Tweet.html"),
            "embed URL comes from the server only"
        );
        assert!(!PAGE.contains("<script src"));
        assert!(!PAGE.contains("contentWindow") && !PAGE.contains("contentDocument"));
    }
}
