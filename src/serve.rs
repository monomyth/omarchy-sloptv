//! Hidden, unofficial LAN web player (`--serve`): a tiny HTTP server that hands the slopTV feed to a
//! browser on the same network (e.g. Safari on iPad/iPhone). Same X TOS gray area as raw-mpv mode:
//! it plays the feed's direct video.twimg.com mp4s outside X's player, proxied through this server.
//! Only ids present in the feed are proxied (no open proxy). The live feed no longer carries `mp4`, so
//! a clip's mp4 is resolved from X's public embed syndication JSON (cdn.syndication.twimg.com, the
//! endpoint X's own embed widget uses; no key) and the author's display name is taken from it too.
use crate::cache::Cache;
use crate::config::Config;
use crate::fetch;
use crate::model::Video;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("web.html");
const APP_JS: &str = include_str!("web.js");
/// Max simultaneous connections; extra ones get a 503 and are closed.
const MAX_CONN: usize = 32;
/// Max bytes of request line + headers.
const MAX_HEAD: u64 = 8 * 1024;
const SECURITY_HEADERS: &str = "X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src 'self' https://pbs.twimg.com; media-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n";

/// The feed, refreshed lazily: nothing runs while idle. A request that finds it older than
/// `max_age` refetches it (single-flight: one fetch, concurrent callers wait and reuse it). An
/// unknown clip id triggers at most one refetch per `miss_gap`.
pub struct LazyFeed {
    videos: RwLock<Arc<Vec<Video>>>,
    fetched: Mutex<Instant>,
    last_miss: Mutex<Option<Instant>>,
    refresh: Mutex<()>,
    max_age: Duration,
    miss_gap: Duration,
    loader: Box<dyn Fn() -> Vec<Video> + Send + Sync>,
}

fn lk<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl LazyFeed {
    pub fn new(
        initial: Vec<Video>,
        max_age: Duration,
        miss_gap: Duration,
        loader: Box<dyn Fn() -> Vec<Video> + Send + Sync>,
    ) -> LazyFeed {
        LazyFeed {
            videos: RwLock::new(Arc::new(initial)),
            fetched: Mutex::new(Instant::now()),
            last_miss: Mutex::new(None),
            refresh: Mutex::new(()),
            max_age,
            miss_gap,
            loader,
        }
    }
    fn current(&self) -> Arc<Vec<Video>> {
        self.videos
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn reload(&self) {
        let v = (self.loader)();
        if !v.is_empty() {
            *self.videos.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(v);
        }
        *lk(&self.fetched) = Instant::now(); // also on failure, so a dead source isn't hammered
    }
    /// The feed, refetched first if it is older than max_age.
    pub fn fresh(&self) -> Arc<Vec<Video>> {
        if lk(&self.fetched).elapsed() >= self.max_age {
            let _g = lk(&self.refresh);
            if lk(&self.fetched).elapsed() >= self.max_age {
                self.reload();
            }
        }
        self.current()
    }
    /// Feed record for a (validated) id; an unknown id refetches at most once per miss_gap.
    pub fn find(&self, id: &str) -> Option<Video> {
        let hit = |f: &[Video]| f.iter().find(|x| x.id == id).cloned();
        if let Some(v) = hit(&self.fresh()) {
            return Some(v);
        }
        let _g = lk(&self.refresh);
        {
            let mut lm = lk(&self.last_miss);
            if lm.is_some_and(|t| t.elapsed() < self.miss_gap) {
                return hit(&self.current());
            }
            *lm = Some(Instant::now());
        }
        if let Some(v) = hit(&self.current()) {
            return Some(v); // another request refreshed meanwhile
        }
        self.reload();
        hit(&self.current())
    }
}

type Feed = Arc<LazyFeed>;

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub mp4: String,
    pub name: Option<String>,
}
type Resolver = Arc<Mutex<HashMap<String, Resolved>>>;

/// Pick the highest-bitrate mp4 variant and the author's display name from syndication JSON.
pub fn parse_syndication(j: &serde_json::Value) -> Option<Resolved> {
    let mut best: Option<(u64, String)> = None;
    for m in j["mediaDetails"].as_array()? {
        for v in m["video_info"]["variants"].as_array().into_iter().flatten() {
            if v["content_type"] != "video/mp4" {
                continue;
            }
            let (b, u) = (
                v["bitrate"].as_u64().unwrap_or(0),
                v["url"].as_str().unwrap_or(""),
            );
            if u.starts_with("https://video.twimg.com/") && best.as_ref().map_or(true, |x| b > x.0)
            {
                best = Some((b, u.to_string()));
            }
        }
    }
    Some(Resolved {
        mp4: best?.1,
        name: j["user"]["name"].as_str().map(|s| s.to_string()),
    })
}

fn resolve(res: &Resolver, v: &Video) -> Result<Resolved> {
    if let Some(r) = res.lock().unwrap_or_else(|e| e.into_inner()).get(&v.id) {
        return Ok(r.clone());
    }
    let r = match v.mp4.clone() {
        Some(m) if m.starts_with("https://video.twimg.com/") => Resolved {
            mp4: m,
            name: v.name.clone(),
        },
        _ => {
            let u = format!(
                "https://cdn.syndication.twimg.com/tweet-result?id={}&token=a",
                v.id
            );
            let resp = fetch::agent().get(&u).call().map_err(|e| anyhow!("{e}"))?;
            if resp.status().as_u16() != 200 {
                return Err(anyhow!("syndication HTTP {} for {}", resp.status(), v.id));
            }
            let j: serde_json::Value = serde_json::from_reader(resp.into_body().into_reader())?;
            parse_syndication(&j).ok_or_else(|| anyhow!("no mp4 for {}", v.id))?
        }
    };
    res.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(v.id.clone(), r.clone());
    Ok(r)
}

pub fn run(cfg: Config, cache: Cache, bind: &str) -> Result<()> {
    let r = fetch::load_feed(&cfg, &cache);
    if r.videos.is_empty() {
        return Err(anyhow!("no videos (source {})", cfg.source_url));
    }
    eprintln!(
        "slop-tv serve: {} videos (online: {})",
        r.videos.len(),
        r.online
    );
    let max_age = Duration::from_secs(cfg.poll_minutes.max(1) * 60);
    let loader = Box::new(move || fetch::load_feed(&cfg, &cache).videos);
    let feed: Feed = Arc::new(LazyFeed::new(
        r.videos,
        max_age,
        Duration::from_secs(60),
        loader,
    ));
    let bind = if bind.is_empty() {
        default_bind()
    } else {
        bind.to_string()
    };
    let bind = bind.as_str();
    let res: Resolver = Arc::new(Mutex::new(HashMap::new()));
    let l = TcpListener::bind(bind)?;
    if bind.starts_with("0.0.0.0") {
        eprintln!("slop-tv serve: WARNING listening on ALL interfaces ({bind}); keep it behind the LAN firewall");
    }
    eprintln!("slop-tv serve: listening on http://{bind}/ (LAN only; unofficial, see README)");
    let active = Arc::new(AtomicUsize::new(0));
    for mut s in l.incoming().flatten() {
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONN {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
            let _ = send(
                &mut s,
                "503 Service Unavailable",
                "text/plain",
                b"busy",
                false,
            );
            continue;
        }
        let (feed, res, active) = (feed.clone(), res.clone(), active.clone());
        std::thread::spawn(move || {
            if let Err(e) = handle(s, &feed, &res) {
                eprintln!("slop-tv serve: {e}");
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

pub(crate) struct Req {
    method: String,
    path: String,
    range: Option<String>,
}

fn read_req(s: &TcpStream) -> Result<Req> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    parse_req(BufReader::new(s.take(MAX_HEAD)))
}

/// Parse a request head from a reader already capped at MAX_HEAD bytes.
pub(crate) fn parse_req<R: BufRead>(mut r: R) -> Result<Req> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("/").to_string();
    let mut range = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Err(anyhow!("request head too large or truncated"));
        }
        if h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = sanitize_range(v);
            }
        }
    }
    if !line.ends_with('\n') {
        return Err(anyhow!("request head too large or truncated"));
    }
    Ok(Req {
        method,
        path,
        range,
    })
}

fn send(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8], head: bool) -> Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{SECURITY_HEADERS}Connection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        s.write_all(body)?;
    }
    Ok(())
}

pub fn queue_json(v: &[Video]) -> String {
    let items: Vec<_> = v
        .iter()
        .map(|x| {
            serde_json::json!({
                "id": x.id, "creator": x.creator, "name": x.name, "title": x.title(),
                "handle": x.handle(), "caption": x.caption, "url": safe_post_url(&x.url, &x.id), "posted_at": x.posted_at,
                "duration_ms": x.duration_ms, "poster": x.poster, "src": format!("/v/{}.mp4", x.id), "meta": format!("/meta/{}.json", x.id),
            })
        })
        .collect();
    serde_json::json!({ "videos": items }).to_string()
}

/// Post ids are 1-25 ASCII digits; anything else is rejected before any lookup or fetch.
pub fn valid_id(id: &str) -> bool {
    (1..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Accept only a single, sane byte range (`bytes=A-B`, `bytes=A-`, `bytes=-N`); anything else is
/// dropped so the request becomes a plain 200. Returns the normalized header value.
pub fn sanitize_range(r: &str) -> Option<String> {
    let spec = r.trim().strip_prefix("bytes=")?;
    if spec.len() > 41 || spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let num = |x: &str| -> Option<Option<u64>> {
        if x.is_empty() {
            Some(None)
        } else if x.bytes().all(|c| c.is_ascii_digit()) {
            x.parse().ok().map(Some)
        } else {
            None
        }
    };
    match (num(a)?, num(b)?) {
        (Some(a), Some(b)) if a <= b => Some(format!("bytes={a}-{b}")),
        (Some(a), None) => Some(format!("bytes={a}-")),
        (None, Some(n)) if n > 0 => Some(format!("bytes=-{n}")),
        _ => None,
    }
}

/// Feed post links shown to the browser must be https x.com/twitter.com URLs.
pub fn safe_post_url(u: &str, id: &str) -> String {
    if u.starts_with("https://x.com/") || u.starts_with("https://twitter.com/") {
        u.to_string()
    } else {
        format!("https://x.com/i/status/{id}")
    }
}

/// Default bind: this host's LAN address (the interface with the default route), port 8787.
/// Falls back to 0.0.0.0 only when no LAN address is found, and says so.
pub fn default_bind() -> String {
    let ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s)) // no packet is sent
        .and_then(|s| s.local_addr())
        .map(|a| a.ip());
    match ip {
        Ok(ip) if is_lan(&ip) => format!("{ip}:8787"),
        _ => {
            eprintln!("slop-tv serve: no private LAN address found; binding 0.0.0.0:8787 (all interfaces)");
            "0.0.0.0:8787".into()
        }
    }
}

pub fn is_lan(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => v.is_private(),
        _ => false,
    }
}

/// The validated id from `/v/<id>.mp4` or `/meta/<id>.json`.
pub fn clip_id(path: &str) -> Option<&str> {
    let id = path
        .strip_prefix("/v/")
        .and_then(|p| p.strip_suffix(".mp4"))
        .or_else(|| {
            path.strip_prefix("/meta/")
                .and_then(|p| p.strip_suffix(".json"))
        })?;
    if !valid_id(id) {
        return None;
    }
    Some(id)
}

fn handle(mut s: TcpStream, feed: &Feed, res: &Resolver) -> Result<()> {
    let req = read_req(&s)?;
    let head = req.method == "HEAD";
    if req.method != "GET" && !head {
        return send(&mut s, "405 Method Not Allowed", "text/plain", b"", false);
    }
    let path = req.path.split('?').next().unwrap_or("/").to_string();
    match path.as_str() {
        "/" | "/index.html" => send(
            &mut s,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
            head,
        ),
        "/app.js" => send(
            &mut s,
            "200 OK",
            "text/javascript; charset=utf-8",
            APP_JS.as_bytes(),
            head,
        ),
        "/queue.json" => {
            let j = queue_json(&feed.fresh());
            send(&mut s, "200 OK", "application/json", j.as_bytes(), head)
        }
        p if p.starts_with("/v/") || p.starts_with("/meta/") => {
            let Some(v) = clip_id(p).and_then(|id| feed.find(id)) else {
                return send(&mut s, "404 Not Found", "text/plain", b"unknown clip", head);
            };
            match resolve(res, &v) {
                Ok(r) if p.starts_with("/meta/") => {
                    let j = serde_json::json!({"id": v.id, "name": r.name}).to_string();
                    send(&mut s, "200 OK", "application/json", j.as_bytes(), head)
                }
                Ok(r) => proxy(&mut s, &r.mp4, req.range.as_deref(), head),
                Err(e) => {
                    eprintln!("slop-tv serve: {e}");
                    send(
                        &mut s,
                        "502 Bad Gateway",
                        "text/plain",
                        b"cannot resolve clip",
                        head,
                    )
                }
            }
        }
        _ => send(&mut s, "404 Not Found", "text/plain", b"not found", head),
    }
}

/// Stream the upstream mp4, passing Range through so Safari gets 206 + Content-Range.
fn proxy(s: &mut TcpStream, url: &str, range: Option<&str>, head: bool) -> Result<()> {
    if !url.starts_with("https://video.twimg.com/") {
        return send(
            s,
            "502 Bad Gateway",
            "text/plain",
            b"refusing non-twimg url",
            head,
        );
    }
    let mut req = fetch::agent().get(url);
    if let Some(r) = range {
        req = req.header("Range", r);
    }
    let resp = req.call().map_err(|e| anyhow!("{e}"))?;
    let st = resp.status().as_u16();
    if !matches!(st, 200 | 206 | 416) {
        return send(s, "502 Bad Gateway", "text/plain", b"upstream error", head);
    }
    let h = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string())
    };
    let reason = match st {
        200 => "OK",
        206 => "Partial Content",
        _ => "Range Not Satisfiable",
    };
    let mut out = format!(
        "HTTP/1.1 {st} {reason}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\n{SECURITY_HEADERS}Connection: close\r\n"
    );
    if let Some(l) = h("content-length") {
        out += &format!("Content-Length: {l}\r\n");
    }
    if let Some(c) = h("content-range") {
        out += &format!("Content-Range: {c}\r\n");
    }
    out += "\r\n";
    s.write_all(out.as_bytes())?;
    if head {
        return Ok(());
    }
    let mut body = resp.into_body().into_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if s.write_all(&buf[..n]).is_err() {
            break;
        } // client seeked / closed
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    fn lookup(f: &[Video], p: &str) -> Option<Video> {
        let id = clip_id(p)?;
        f.iter().find(|x| x.id == id).cloned()
    }
    fn counting(
        ids: &'static [&'static str],
        n: Arc<AtomicUsize>,
        delay_ms: u64,
    ) -> Box<dyn Fn() -> Vec<Video> + Send + Sync> {
        Box::new(move || {
            n.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(delay_ms));
            ids.iter().map(|i| v(i, None)).collect()
        })
    }
    #[test]
    fn lazy_feed_idle_fresh_and_single_flight() {
        let n = Arc::new(AtomicUsize::new(0));
        // fresh feed: no fetch at all
        let f = LazyFeed::new(
            vec![v("1", None)],
            Duration::from_secs(3600),
            Duration::from_secs(60),
            counting(&["1", "2"], n.clone(), 0),
        );
        assert_eq!(f.fresh().len(), 1);
        assert_eq!(n.load(Ordering::SeqCst), 0);
        // stale feed: 8 concurrent requests -> exactly one fetch
        let f2 = Arc::new(LazyFeed::new(
            vec![v("1", None)],
            Duration::from_secs(3600),
            Duration::from_secs(60),
            counting(&["1", "2"], n.clone(), 200),
        ));
        *lk(&f2.fetched) -= Duration::from_secs(7200);
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let f = f2.clone();
                std::thread::spawn(move || f.fresh().len())
            })
            .collect();
        for h in hs {
            assert_eq!(h.join().unwrap(), 2);
        }
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn lazy_feed_miss_refreshes_at_most_once_per_gap() {
        let n = Arc::new(AtomicUsize::new(0));
        let f = LazyFeed::new(
            vec![v("1", None)],
            Duration::from_secs(3600),
            Duration::from_secs(60),
            counting(&["1", "2"], n.clone(), 0),
        );
        assert!(f.find("1").is_some());
        assert_eq!(n.load(Ordering::SeqCst), 0);
        assert!(f.find("2").is_some()); // miss -> one refetch finds it
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(f.find("999").is_none());
        assert!(f.find("998").is_none()); // within the gap of the last miss: no more fetches
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
    fn v(id: &str, mp4: Option<&str>) -> Video {
        Video {
            id: id.into(),
            url: format!("https://x.com/a/status/{id}"),
            creator: "a".into(),
            name: None,
            caption: "c".into(),
            posted_at: String::new(),
            mp4: mp4.map(|s| s.into()),
            poster: None,
            duration_ms: None,
            likes: None,
            views: None,
        }
    }
    #[test]
    fn lookup_only_feed_ids() {
        let f = vec![v("1", Some("https://video.twimg.com/x.mp4")), v("2", None)];
        assert_eq!(lookup(&f, "/v/1.mp4").unwrap().id, "1");
        assert_eq!(lookup(&f, "/meta/2.json").unwrap().id, "2");
        assert!(lookup(&f, "/v/3.mp4").is_none());
        assert!(lookup(&f, "/v/https://evil.mp4").is_none());
        assert!(lookup(&f, "/v/1").is_none());
    }
    #[test]
    fn syndication_picks_best_mp4_and_name() {
        let j = serde_json::json!({"user": {"name": "Ann"}, "mediaDetails": [{"video_info": {"variants": [
            {"content_type": "application/x-mpegURL", "url": "https://video.twimg.com/a.m3u8"},
            {"content_type": "video/mp4", "bitrate": 432000, "url": "https://video.twimg.com/lo.mp4"},
            {"content_type": "video/mp4", "bitrate": 8768000, "url": "https://video.twimg.com/hi.mp4"},
            {"content_type": "video/mp4", "bitrate": 9999999, "url": "https://evil.example/x.mp4"}]}}]});
        let r = parse_syndication(&j).unwrap();
        assert_eq!(r.mp4, "https://video.twimg.com/hi.mp4");
        assert_eq!(r.name.as_deref(), Some("Ann"));
        assert!(parse_syndication(&serde_json::json!({"mediaDetails": []})).is_none());
    }
    #[test]
    fn queue_has_attribution() {
        let j: serde_json::Value =
            serde_json::from_str(&queue_json(&[v("1", Some("u")), v("2", None)])).unwrap();
        let a = j["videos"].as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0]["handle"], "@a");
        assert_eq!(a[0]["src"], "/v/1.mp4");
        assert_eq!(a[0]["url"], "https://x.com/a/status/1");
    }
    #[test]
    fn ids_must_be_digits() {
        assert!(valid_id("2110117733329502549"));
        for bad in [
            "",
            "../etc/passwd",
            "1/../2",
            "1%2e",
            "12a",
            "1 2",
            &"9".repeat(26),
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
        let f = vec![v("1", None)];
        assert!(lookup(&f, "/v/../1.mp4").is_none());
        assert!(lookup(&f, "/v/1.mp4/../x").is_none());
    }
    #[test]
    fn ranges_are_sanitized() {
        assert_eq!(
            sanitize_range("bytes=0-1023").as_deref(),
            Some("bytes=0-1023")
        );
        assert_eq!(sanitize_range(" bytes=5- ").as_deref(), Some("bytes=5-"));
        assert_eq!(sanitize_range("bytes=-500").as_deref(), Some("bytes=-500"));
        for bad in [
            "bytes=5-1",
            "bytes=-0",
            "bytes=-",
            "bytes=0-1,5-9",
            "bytes=-1-2",
            "bytes=+1-2",
            "bytes=99999999999999999999999-",
            "items=0-1",
            "bytes=0-1\r\nX: y",
            "bytes=a-b",
        ] {
            assert!(sanitize_range(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn post_urls_are_https_x_only() {
        assert_eq!(
            safe_post_url("javascript:alert(1)", "7"),
            "https://x.com/i/status/7"
        );
        assert_eq!(
            safe_post_url("https://x.com.evil/a", "7"),
            "https://x.com/i/status/7"
        );
        assert_eq!(
            safe_post_url("https://x.com/a/status/7", "7"),
            "https://x.com/a/status/7"
        );
    }
    #[test]
    fn request_head_is_bounded_and_parsed() {
        let r = parse_req(std::io::Cursor::new(
            b"GET /v/1.mp4 HTTP/1.1\r\nRange: bytes=0-9\r\n\r\n".to_vec(),
        ))
        .unwrap();
        assert_eq!(
            (r.method.as_str(), r.path.as_str(), r.range.as_deref()),
            ("GET", "/v/1.mp4", Some("bytes=0-9"))
        );
        let huge = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(20_000));
        assert!(parse_req(BufReader::new(
            std::io::Cursor::new(huge.into_bytes()).take(MAX_HEAD)
        ))
        .is_err());
        assert!(parse_req(std::io::Cursor::new(b"\xff\xfe garbage".to_vec())).is_err());
        assert!(parse_req(std::io::Cursor::new(Vec::new())).is_err());
    }
    #[test]
    fn lan_detection() {
        assert!(is_lan(&"192.168.1.183".parse().unwrap()));
        assert!(!is_lan(&"8.8.8.8".parse().unwrap()));
        assert!(!is_lan(&"0.0.0.0".parse().unwrap()));
    }
    #[test]
    fn page_has_no_inline_script_or_innerhtml() {
        assert!(!PAGE.contains("<script>") && PAGE.contains("src=\"/app.js\""));
        assert!(!APP_JS.contains("innerHTML") && !APP_JS.contains("document.write"));
        assert!(
            SECURITY_HEADERS.contains("frame-ancestors 'none'")
                && SECURITY_HEADERS.contains("nosniff")
        );
    }
}
