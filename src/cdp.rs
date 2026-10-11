//! Default player (`player = "video"`): X's video page is the TOP-LEVEL page of a Chromium app window and our
//! overlay (src/overlay.js) is injected into it through Chromium's remote debugging port (CDP), bound to
//! 127.0.0.1 on a random free port. The overlay follows the real <video> element (currentTime, duration, paused,
//! ended), so pausing or scrubbing in X's player moves our auto-advance timer too.
//!
//! Page -> controller: the Runtime binding `slopTV` (JSON `{"cmd": ...}`). Controller -> page: Runtime.evaluate of
//! `window.__slopSet(<state>)`. Next/prev navigate with Page.navigate. This is code of ours running inside X's page.
use crate::cache::Cache;
use crate::config::Config;
use crate::embed::{self, Action, Ctl};
use crate::fetch;
use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

pub const OVERLAY: &str = include_str!("overlay.js");
pub const BINDING: &str = "slopTV";

/// A free local port (bound and released immediately; Chromium then listens on it).
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Extra Chromium flags: remote debugging on loopback only.
pub fn debug_args(port: u16) -> Vec<String> {
    vec![
        format!("--remote-debugging-port={port}"),
        "--remote-debugging-address=127.0.0.1".into(),
    ]
}

/// Chromium arguments for the CDP app window: the normal app-window flags plus the debugging port; the first page
/// is about:blank (privacy notice) and the controller navigates it.
/// A window that was killed leaves `exit_type: Crashed` in the profile, which makes Chromium offer
/// "Restore pages?" next start. Mark the last exit as clean before launching.
fn mark_clean_exit(profile: &std::path::Path) {
    let p = profile.join("Default/Preferences");
    if let Ok(t) = std::fs::read_to_string(&p) {
        let n = t
            .replace("\"exit_type\":\"Crashed\"", "\"exit_type\":\"Normal\"")
            .replace("\"exited_cleanly\":false", "\"exited_cleanly\":true");
        if n != t {
            let _ = std::fs::write(&p, n);
        }
    }
}

/// `--app=about:blank` makes this Chromium (with the Arch/Omarchy wrapper flags) open a plain New Tab (Google)
/// instead of an app window, so the start page is a tiny black data: page.
pub const BLANK_APP_URL: &str =
    "data:text/html,<title>slopTV</title><body style='background:%23000'>";

pub fn browser_args(profile: &std::path::Path, port: u16, fullscreen: bool) -> Vec<String> {
    let mut v = embed::browser_args(profile, BLANK_APP_URL, fullscreen);
    v.extend(debug_args(port));
    // test hook, e.g. SLOPTV_BROWSER_EXTRA="--headless=new --no-sandbox" (no window)
    if let Ok(x) = std::env::var("SLOPTV_BROWSER_EXTRA") {
        v.extend(x.split_whitespace().map(String::from));
    }
    v
}

/// Plain HTTP GET against the debugging port (loopback only).
fn http_get(port: u16, path: &str) -> Option<String> {
    let mut s =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500))
            .ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    // Chromium's DevTools HTTP server may keep the connection open: stop at Content-Length, not at EOF
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(body) = http_body(&buf) {
            return Some(body);
        }
        if n == 0 {
            return None;
        }
    }
}

/// Body of a complete HTTP response in `buf` (None while headers or body are still incomplete).
pub fn http_body(buf: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(buf);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let want = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().ok())
        })
        .flatten();
    match want {
        Some(n) if body.len() >= n => Some(body[..n].to_string()),
        Some(_) => None,
        None => None,
    }
}

/// The websocket URL of the first normal page target from `/json/list`, only if it points at our loopback port.
pub fn pick_page_ws(list_json: &str, port: u16) -> Option<String> {
    let v: Value = serde_json::from_str(list_json).ok()?;
    let want = format!("ws://127.0.0.1:{port}/devtools/page/");
    v.as_array()?
        .iter()
        .filter(|t| t["type"] == "page")
        .filter_map(|t| t["webSocketDebuggerUrl"].as_str())
        .map(|u| u.replace("ws://localhost:", "ws://127.0.0.1:"))
        .find(|u| u.starts_with(&want))
}

/// Keep only the highest-RESOLUTION `#EXT-X-STREAM-INF` variant of an HLS master playlist (audio / other tags are
/// untouched). None when the text is not a master playlist, has no RESOLUTION, or already has a single variant.
pub fn filter_master(body: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let height = |l: &str| -> u64 {
        l.split("RESOLUTION=")
            .nth(1)
            .and_then(|r| r.split(|c| c == ',' || c == ' ').next())
            .and_then(|r| r.split('x').nth(1))
            .and_then(|h| h.trim().parse().ok())
            .unwrap_or(0)
    };
    let variants: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with("#EXT-X-STREAM-INF"))
        .collect();
    if variants.len() < 2 {
        return None;
    }
    let best = variants
        .iter()
        .copied()
        .max_by_key(|&i| (height(lines[i]), std::cmp::Reverse(i)))?;
    if height(lines[best]) == 0 {
        return None;
    }
    let out: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            !variants.contains(i) && !variants.contains(&i.wrapping_sub(1))
                || *i == best
                || *i == best + 1
        })
        .map(|(_, l)| *l)
        .collect();
    Some(out.join("\n") + "\n")
}

pub fn command(id: u64, method: &str, params: Value) -> String {
    json!({"id": id, "method": method, "params": params}).to_string()
}

/// Payload of a `Runtime.bindingCalled` event for our binding.
pub fn binding_payload(msg: &Value) -> Option<String> {
    (msg["method"] == "Runtime.bindingCalled" && msg["params"]["name"] == BINDING)
        .then(|| msg["params"]["payload"].as_str().map(str::to_string))
        .flatten()
}

/// `{"cmd":"next"}` -> "next"; only whitelisted commands survive.
pub fn parse_cmd(payload: &str) -> Option<&'static str> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let c = v["cmd"].as_str()?;
    [
        "hello",
        "next",
        "prev",
        "shuffle",
        "auto",
        "like",
        "open",
        "ack",
        "quit",
        "fullscreen",
    ]
    .into_iter()
    .find(|k| *k == c)
}

/// JS pushed into the page to hand it the current state.
pub fn set_state_expr(state_json: &str) -> String {
    format!("window.__slopSet && window.__slopSet({state_json})")
}

/// What the controller does for one page command (pure apart from the queue / notice file inside `ctl`).
#[derive(Debug, Default)]
pub struct Step {
    pub navigate: Option<String>,
    pub state: Option<String>,
    pub action: Option<Action>,
}

/// Where the window should be: the notice (about:blank) until acknowledged, else the current clip's video page.
pub fn nav_target(ctl: &Ctl) -> Option<String> {
    if ctl.notice_pending() {
        return Some("about:blank".into());
    }
    let id = ctl.cur.and_then(|i| ctl.videos.get(i))?.id.clone();
    embed::video_url(&id)
}

pub fn on_cmd(ctl: &mut Ctl, cmd: &str) -> Step {
    let path = |p: &str| format!("/t/{}/api/{p}", ctl.token);
    let mut step = Step::default();
    match cmd {
        "hello" => step.state = Some(ctl.state_json()),
        "next" | "prev" | "ack" => {
            let (p, m) = (path(cmd), "POST");
            ctl.handle(m, &p);
            step.navigate = nav_target(ctl);
        }
        "shuffle" | "auto" => {
            let p = path(cmd);
            ctl.handle("POST", &p);
            step.state = Some(ctl.state_json());
        }
        "like" | "open" | "quit" | "fullscreen" => {
            let p = path(cmd);
            let r = ctl.handle("POST", &p);
            step.action = Some(r.action);
        }
        _ => {}
    }
    step
}

struct Cdp {
    ws: WebSocket<TcpStream>,
    next_id: u64,
}

impl Cdp {
    fn connect(url: &str, port: u16) -> Result<Cdp> {
        let s = TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        let (ws, _) = tungstenite::client(url, s).map_err(|e| anyhow!("cdp handshake: {e}"))?;
        ws.get_ref()
            .set_read_timeout(Some(Duration::from_millis(200)))?;
        Ok(Cdp { ws, next_id: 1 })
    }
    fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.ws.send(Message::text(command(id, method, params)))?;
        Ok(id)
    }
    /// One incoming message; Ok(None) on read timeout; Err when the socket is closed.
    fn recv(&mut self) -> Result<Option<Value>> {
        match self.ws.read() {
            Ok(Message::Text(t)) => Ok(serde_json::from_str(t.as_str()).ok()),
            Ok(Message::Close(_)) => Err(anyhow!("closed")),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(anyhow!("cdp: {e}")),
        }
    }
}

fn run_action(a: Action, quit: &mut bool) {
    match a {
        Action::Open(u) => {
            let _ = std::process::Command::new("xdg-open")
                .arg(u)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map(|mut c| std::thread::spawn(move || c.wait()));
        }
        Action::Fullscreen => {
            std::thread::spawn(crate::hypr::toggle);
        }
        Action::Quit => *quit = true,
        Action::None => {}
    }
}

pub fn run(cfg: Config, cache: Cache, reset_notice: bool) -> Result<()> {
    let notice = embed::notice_path().with_file_name("cdp-notice-ack");
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
    ctl.token = "cdp".into();
    ctl.auto_advance = cfg.auto_advance;
    ctl.default_secs = cfg.embed_default_secs;
    ctl.video_player = true;
    ctl.hypr_native = crate::hypr::available();

    let browser = embed::find_browser().ok_or_else(|| {
        anyhow!(
            "no chromium/chrome found; the default player needs one (or set player = \"embed\")"
        )
    })?;
    let profile = embed::profile_dir();
    std::fs::create_dir_all(&profile)?;
    mark_clean_exit(&profile);
    let port = free_port()?;
    let mut child = std::process::Command::new(browser)
        .args(browser_args(&profile, port, cfg.start_fullscreen))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    crate::hypr::BROWSER_PID.store(child.id(), Ordering::SeqCst);
    if cfg.start_fullscreen {
        crate::hypr::ensure_fullscreen_once();
    }
    let res = session(&mut ctl, &mut child, port, cfg.best_quality);
    let _ = child.kill();
    let _ = child.wait();
    res
}

fn session(
    ctl: &mut Ctl,
    child: &mut std::process::Child,
    port: u16,
    best_quality: bool,
) -> Result<()> {
    // wait for the debugging endpoint and the first page target
    let t0 = Instant::now();
    let ws_url = loop {
        if let Some(u) = http_get(port, "/json/list").and_then(|b| pick_page_ws(&b, port)) {
            break u;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Err(anyhow!(
                "Chromium exited before opening its debugging port (is another slopTV window using the same profile still open?)"
            ));
        }
        if t0.elapsed() > Duration::from_secs(25) {
            return Err(anyhow!(
                "timed out waiting for Chromium's debugging port {port}"
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    let mut cdp = Cdp::connect(&ws_url, port)?;
    cdp.send("Page.enable", json!({}))?;
    cdp.send("Runtime.enable", json!({}))?;
    cdp.send("Runtime.addBinding", json!({"name": BINDING}))?;
    cdp.send(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": OVERLAY}),
    )?;
    // the page that is already open (about:blank) never sees the new-document script: run it there too
    cdp.send("Runtime.evaluate", json!({"expression": OVERLAY}))?;
    if best_quality {
        cdp.send(
            "Fetch.enable",
            json!({"patterns": [{"urlPattern": "https://video.twimg.com/*.m3u8*", "requestStage": "Response"}]}),
        )?;
    }
    let mut body_reqs: Vec<(u64, String, Value)> = Vec::new();
    let mut nav_ids: Vec<(u64, String)> = Vec::new();
    let navigate = |cdp: &mut Cdp, url: &str, nav_ids: &mut Vec<(u64, String)>| -> Result<()> {
        let id = cdp.send("Page.navigate", json!({"url": url}))?;
        nav_ids.push((id, url.to_string()));
        Ok(())
    };
    if let Some(u) = nav_target(ctl).filter(|u| u != "about:blank") {
        navigate(&mut cdp, &u, &mut nav_ids)?;
    }
    let mut quit = false;
    while !quit {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        let Some(msg) = (match cdp.recv() {
            Ok(m) => m,
            Err(_) => break, // window closed
        }) else {
            continue;
        };
        if msg["method"] == "Fetch.requestPaused" {
            let p = &msg["params"];
            let rid = p["requestId"].as_str().unwrap_or_default().to_string();
            if p["responseStatusCode"] == 200 {
                let id = cdp.send("Fetch.getResponseBody", json!({"requestId": rid}))?;
                body_reqs.push((id, rid, p["responseHeaders"].clone()));
            } else {
                cdp.send("Fetch.continueRequest", json!({"requestId": rid}))?;
            }
            continue;
        }
        if let Some(id) = msg["id"].as_u64() {
            if let Some(p) = body_reqs.iter().position(|(i, _, _)| *i == id) {
                let (_, rid, headers) = body_reqs.remove(p);
                let raw = msg["result"]["body"].as_str().unwrap_or_default();
                let text = if msg["result"]["base64Encoded"] == true {
                    base64::engine::general_purpose::STANDARD
                        .decode(raw)
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                } else {
                    Some(raw.to_string())
                };
                match text.as_deref().and_then(filter_master) {
                    Some(out) => {
                        let body = base64::engine::general_purpose::STANDARD.encode(out);
                        cdp.send(
                            "Fetch.fulfillRequest",
                            json!({"requestId": rid, "responseCode": 200, "responseHeaders": headers, "body": body}),
                        )?;
                    }
                    None => {
                        cdp.send("Fetch.continueRequest", json!({"requestId": rid}))?;
                    }
                }
                continue;
            }
        }
        // Page.navigate result: unreachable video page -> official embed page, as top-level page
        if let Some(id) = msg["id"].as_u64() {
            if let Some(p) = nav_ids.iter().position(|(i, _)| *i == id) {
                let (_, url) = nav_ids.remove(p);
                if msg["result"]["errorText"].is_string() && url.contains("/i/videos/tweet/") {
                    if let Some(e) = ctl
                        .cur
                        .and_then(|i| ctl.videos.get(i))
                        .and_then(|v| embed::embed_url(&v.id))
                    {
                        navigate(&mut cdp, &e, &mut nav_ids)?;
                    }
                }
            }
            continue;
        }
        let Some(cmd) = binding_payload(&msg).and_then(|p| parse_cmd(&p).map(str::to_string))
        else {
            continue;
        };
        let step = on_cmd(ctl, &cmd);
        if let Some(u) = step.navigate {
            navigate(&mut cdp, &u, &mut nav_ids)?;
        }
        if let Some(s) = step.state {
            cdp.send(
                "Runtime.evaluate",
                json!({"expression": set_state_expr(&s)}),
            )?;
        }
        if let Some(a) = step.action {
            run_action(a, &mut quit);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Video;
    use std::path::{Path, PathBuf};

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
            duration_ms: Some(5000),
            likes: None,
            views: None,
        }
    }
    fn ctl(ack: bool) -> Ctl {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let f = std::env::temp_dir().join(format!("slop-cdp-ack-{}-{n}", std::process::id()));
        let _ = std::fs::remove_file(&f);
        if ack {
            std::fs::write(&f, "ok").unwrap();
        }
        let mut c = Ctl::new(vec![vid("1"), vid("2"), vid("3")], false, 1, f);
        c.token = "cdp".into();
        c
    }

    #[test]
    fn debug_port_is_loopback_and_random_port_is_free() {
        let a = browser_args(Path::new("/p"), 4242, false);
        assert!(a.contains(&"--remote-debugging-port=4242".to_string()));
        assert!(a.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
        assert!(a.contains(&format!("--app={BLANK_APP_URL}")));
        assert!(a.contains(&"--user-data-dir=/p".to_string()));
        assert!(!a.iter().any(|x| x.contains("remote-allow-origins")));
        let p = free_port().unwrap();
        assert!(p > 0);
        assert!(TcpListener::bind(("127.0.0.1", p)).is_ok());
    }

    #[test]
    fn http_body_waits_for_content_length_and_ignores_eof() {
        let r = b"HTTP/1.1 200 OK\r\nContent-Type: x\r\nContent-Length: 5\r\n\r\nhel";
        assert!(http_body(r).is_none());
        let r = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello";
        assert_eq!(http_body(r).unwrap(), "hello");
        assert!(http_body(b"HTTP/1.1 200 OK\r\n").is_none());
    }

    #[test]
    fn picks_only_loopback_page_targets() {
        let list = r#"[{"type":"service_worker","webSocketDebuggerUrl":"ws://127.0.0.1:9/devtools/page/S"},
          {"type":"page","webSocketDebuggerUrl":"ws://evil.example:9/devtools/page/E"},
          {"type":"page","webSocketDebuggerUrl":"ws://127.0.0.1:9/devtools/page/ABC"}]"#;
        assert_eq!(
            pick_page_ws(list, 9).unwrap(),
            "ws://127.0.0.1:9/devtools/page/ABC"
        );
        assert!(pick_page_ws(list, 10).is_none());
        assert!(pick_page_ws("nonsense", 9).is_none());
    }

    #[test]
    fn binding_events_and_command_whitelist() {
        let m: Value = serde_json::from_str(
            r#"{"method":"Runtime.bindingCalled","params":{"name":"slopTV","payload":"{\"cmd\":\"next\"}"}}"#,
        )
        .unwrap();
        let p = binding_payload(&m).unwrap();
        assert_eq!(parse_cmd(&p), Some("next"));
        assert_eq!(parse_cmd(r#"{"cmd":"rm -rf"}"#), None);
        assert_eq!(parse_cmd("junk"), None);
        let other: Value = serde_json::from_str(
            r#"{"method":"Runtime.bindingCalled","params":{"name":"other","payload":"{}"}}"#,
        )
        .unwrap();
        assert!(binding_payload(&other).is_none());
        let c: Value = serde_json::from_str(&command(
            7,
            "Page.navigate",
            json!({"url": "https://x.com/"}),
        ))
        .unwrap();
        assert_eq!(c["id"], 7);
        assert_eq!(c["params"]["url"], "https://x.com/");
    }

    #[test]
    fn next_prev_navigate_to_the_video_page_and_notice_stays_blank() {
        let mut c = ctl(true);
        assert_eq!(
            nav_target(&c).unwrap(),
            "https://x.com/i/videos/tweet/1?autoplay=1"
        );
        let s = on_cmd(&mut c, "next");
        assert_eq!(
            s.navigate.unwrap(),
            "https://x.com/i/videos/tweet/2?autoplay=1"
        );
        let s = on_cmd(&mut c, "prev");
        assert_eq!(
            s.navigate.unwrap(),
            "https://x.com/i/videos/tweet/1?autoplay=1"
        );
        let mut n = ctl(false);
        assert_eq!(nav_target(&n).unwrap(), "about:blank");
        let s = on_cmd(&mut n, "ack");
        assert_eq!(
            s.navigate.unwrap(),
            "https://x.com/i/videos/tweet/1?autoplay=1"
        );
        let _ = std::fs::remove_file(&n.notice_file);
        let _ = std::fs::remove_file(&c.notice_file);
    }

    #[test]
    fn hello_shuffle_auto_push_state_and_actions_pass_through() {
        let mut c = ctl(true);
        let s = on_cmd(&mut c, "hello");
        let st: Value = serde_json::from_str(&s.state.unwrap()).unwrap();
        assert_eq!(st["id"], "1");
        assert_eq!(st["duration_secs"], 5);
        assert!(st["hypr_native"].is_boolean());
        let s = on_cmd(&mut c, "auto");
        let st: Value = serde_json::from_str(&s.state.unwrap()).unwrap();
        assert_eq!(st["auto"], false);
        assert!(s.navigate.is_none());
        assert!(matches!(on_cmd(&mut c, "quit").action, Some(Action::Quit)));
        assert!(
            matches!(on_cmd(&mut c, "open").action, Some(Action::Open(u)) if u.ends_with("/status/1"))
        );
        assert!(
            matches!(on_cmd(&mut c, "like").action, Some(Action::Open(u)) if u.contains("intent/like"))
        );
        let e = set_state_expr("{\"a\":1}");
        assert!(e.contains("__slopSet({\"a\":1})"));
        let _ = std::fs::remove_file(&c.notice_file);
    }

    /// Run the overlay's pure SYNC section (follower + bestQuality) under node. None (skipped) without node.
    fn run_sync_js(test: &str) -> Option<String> {
        let a = OVERLAY.find("// SYNC-BEGIN")?;
        let b = OVERLAY.find("// SYNC-END")?;
        let js = format!("{}\n{}", &OVERLAY[a..b], test);
        let out = std::process::Command::new("node")
            .arg("-e")
            .arg(js)
            .output()
            .ok()?;
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    #[test]
    fn timer_follows_pause_scrub_end_and_loop_of_the_real_video() {
        let r = run_sync_js(
            r#"
            const o = [];
            const f = makeFollower(1000); f.start(10, true, 0);
            // playing at 3s of 20s: frac follows the real video, not the feed duration
            o.push(f.step({t:3,d:20,paused:false,ended:false}, 100), f.frac.toFixed(2));
            // user clicks to pause for a long time: wall clock moves, video does not -> never due
            o.push(f.step({t:3,d:20,paused:true,ended:false}, 999999), f.frac.toFixed(2));
            // user scrubs back to 1s, then to 12s: progress follows immediately
            f.step({t:1,d:20,paused:false,ended:false}, 1e6); o.push(f.frac.toFixed(2));
            f.step({t:12,d:20,paused:false,ended:false}, 1e6+1); o.push(f.frac.toFixed(2));
            // reaching the end fires once; ended flag alone also fires (new clip)
            o.push(f.step({t:19.99,d:20,paused:false,ended:false}, 1e6+2), f.step({t:20,d:20,paused:true,ended:true}, 1e6+3));
            const g = makeFollower(1000); g.start(10, true, 0);
            o.push(g.step({t:5,d:10,paused:true,ended:true}, 1));
            // looping video: time wraps back to 0 near the end
            const l = makeFollower(1000); l.start(10, true, 0);
            l.step({t:9.8,d:10,paused:false,ended:false}, 1);
            o.push(l.step({t:0.2,d:10,paused:false,ended:false}, 2));
            // auto off never fires, shows via frac only
            const n = makeFollower(1000); n.start(10, false, 0);
            o.push(n.step({t:10,d:10,paused:false,ended:true}, 1));
            console.log(o.join(' '));
            "#,
        );
        if let Some(r) = r {
            assert_eq!(
                r,
                "false 0.15 false 0.15 0.05 0.60 true false true true false"
            );
        }
    }

    #[test]
    fn feed_duration_is_only_a_fallback_when_no_video_duration() {
        let r = run_sync_js(
            r#"
            const o = [];
            const f = makeFollower(1000); f.start(10, true, 0);
            o.push(f.step(null, 10999), f.step(null, 11000), f.step(null, 12000), f.src);
            // pausing a video whose duration is unreadable holds the fallback clock
            const g = makeFollower(1000); g.start(10, true, 0);
            g.step({t:0,d:NaN,paused:false,ended:false}, 1000);
            g.step({t:0,d:NaN,paused:true,ended:false}, 50000);
            o.push(g.step({t:0,d:NaN,paused:false,ended:false}, 50500));
            // once a real duration was seen, the feed fallback is never used again
            const h = makeFollower(1000); h.start(2, true, 0);
            h.step({t:1,d:30,paused:false,ended:false}, 100);
            o.push(h.step(null, 99999), h.src);
            console.log(o.join(' '));
            "#,
        );
        if let Some(r) = r {
            assert_eq!(r, "false true false feed false false video");
        }
    }

    #[test]
    fn master_playlist_keeps_only_the_highest_resolution() {
        let m = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",URI=\"/au.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=480x270,AUDIO=\"a\"\n/270.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2,RESOLUTION=2560x1440,AUDIO=\"a\"\n/1440.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3,RESOLUTION=1280x720,AUDIO=\"a\"\n/720.m3u8\n";
        let o = filter_master(m).unwrap();
        assert!(o.contains("/1440.m3u8") && o.contains("RESOLUTION=2560x1440"));
        assert!(!o.contains("/270.m3u8") && !o.contains("/720.m3u8"));
        assert!(o.contains("#EXT-X-MEDIA:TYPE=AUDIO") && o.starts_with("#EXTM3U"));
        // media playlists, single variants and resolution-less masters are left alone
        assert!(filter_master("#EXTM3U\n#EXTINF:3,\nseg.m4s\n").is_none());
        assert!(filter_master(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,RESOLUTION=1280x720\n/a.m3u8\n"
        )
        .is_none());
        assert!(filter_master(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n/a\n#EXT-X-STREAM-INF:BANDWIDTH=2\n/b\n"
        )
        .is_none());
        assert!(Config::default().best_quality);
    }

    #[test]
    fn overlay_is_valid_js_and_has_the_keys_and_no_cookie_access() {
        let f = std::env::temp_dir().join(format!("slop-tv-overlay-{}.js", std::process::id()));
        std::fs::write(&f, OVERLAY).unwrap();
        if let Ok(o) = std::process::Command::new("node")
            .arg("--check")
            .arg(&f)
            .output()
        {
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
        let _ = std::fs::remove_file(&f);
        for k in [
            "k === 'w' || k === 'F11'",
            "k === 'h'",
            "k === 'l'",
            "k === 'n'",
            "k === 's'",
            "k === 'a'",
            "k === 'f'",
            "k === 'd'",
            "k === 'p'",
        ] {
            assert!(OVERLAY.contains(k), "{k}");
        }
        assert!(OVERLAY.contains("height:2px") && OVERLAY.contains("id=\"attr\""));
        assert!(OVERLAY.contains("addEventListener('ended'") && OVERLAY.contains("currentTime"));
        assert!(!OVERLAY.contains("document.cookie") && !OVERLAY.contains("localStorage"));
        assert!(OVERLAY.contains("window.top !== window"), "top frame only");
        let _ = PathBuf::new();
    }
}
