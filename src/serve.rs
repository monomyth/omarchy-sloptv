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
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const PAGE: &str = include_str!("web.html");

type Feed = Arc<RwLock<Vec<Video>>>;

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
            let (b, u) = (v["bitrate"].as_u64().unwrap_or(0), v["url"].as_str().unwrap_or(""));
            if u.starts_with("https://video.twimg.com/") && best.as_ref().map_or(true, |x| b > x.0) {
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
    if let Some(r) = res.lock().unwrap().get(&v.id) {
        return Ok(r.clone());
    }
    let r = match v.mp4.clone() {
        Some(m) => Resolved { mp4: m, name: v.name.clone() },
        None => {
            let u = format!("https://cdn.syndication.twimg.com/tweet-result?id={}&token=a", v.id);
            let resp = fetch::agent().get(&u).call().map_err(|e| anyhow!("{e}"))?;
            if resp.status().as_u16() != 200 {
                return Err(anyhow!("syndication HTTP {} for {}", resp.status(), v.id));
            }
            let j: serde_json::Value = serde_json::from_reader(resp.into_body().into_reader())?;
            parse_syndication(&j).ok_or_else(|| anyhow!("no mp4 for {}", v.id))?
        }
    };
    res.lock().unwrap().insert(v.id.clone(), r.clone());
    Ok(r)
}

pub fn run(cfg: Config, cache: Cache, bind: &str) -> Result<()> {
    let r = fetch::load_feed(&cfg, &cache);
    if r.videos.is_empty() {
        return Err(anyhow!("no videos (source {})", cfg.source_url));
    }
    eprintln!("slop-tv serve: {} videos (online: {})", r.videos.len(), r.online);
    let feed: Feed = Arc::new(RwLock::new(r.videos));
    {
        let feed = feed.clone();
        let poll = cfg.poll_minutes.max(5) as u64;
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(poll * 60));
            let r = fetch::load_feed(&cfg, &cache);
            if !r.videos.is_empty() {
                *feed.write().unwrap() = r.videos;
            }
        });
    }
    let res: Resolver = Arc::new(Mutex::new(HashMap::new()));
    let l = TcpListener::bind(bind)?;
    eprintln!("slop-tv serve: listening on http://{bind}/");
    for s in l.incoming().flatten() {
        let feed = feed.clone();
        let res = res.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(s, &feed, &res) {
                eprintln!("slop-tv serve: {e}");
            }
        });
    }
    Ok(())
}

struct Req {
    method: String,
    path: String,
    range: Option<String>,
}

fn read_req(s: &TcpStream) -> Result<Req> {
    s.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("/").to_string();
    let mut range = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = Some(v.trim().to_string());
            }
        }
    }
    Ok(Req { method, path, range })
}

fn send(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8], head: bool) -> Result<()> {
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
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
                "handle": x.handle(), "caption": x.caption, "url": x.url, "posted_at": x.posted_at,
                "duration_ms": x.duration_ms, "poster": x.poster, "src": format!("/v/{}.mp4", x.id), "meta": format!("/meta/{}.json", x.id),
            })
        })
        .collect();
    serde_json::json!({ "videos": items }).to_string()
}

/// Map `/v/<id>.mp4` or `/meta/<id>.json` to the feed record (feed ids only).
pub fn lookup(v: &[Video], path: &str) -> Option<Video> {
    let id = path
        .strip_prefix("/v/")
        .and_then(|p| p.strip_suffix(".mp4"))
        .or_else(|| path.strip_prefix("/meta/").and_then(|p| p.strip_suffix(".json")))?;
    v.iter().find(|x| x.id == id).cloned()
}

fn handle(mut s: TcpStream, feed: &Feed, res: &Resolver) -> Result<()> {
    let req = read_req(&s)?;
    let head = req.method == "HEAD";
    if req.method != "GET" && !head {
        return send(&mut s, "405 Method Not Allowed", "text/plain", b"", false);
    }
    let path = req.path.split('?').next().unwrap_or("/").to_string();
    match path.as_str() {
        "/" | "/index.html" => send(&mut s, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes(), head),
        "/queue.json" => {
            let j = queue_json(&feed.read().unwrap());
            send(&mut s, "200 OK", "application/json", j.as_bytes(), head)
        }
        p if p.starts_with("/v/") || p.starts_with("/meta/") => {
            let Some(v) = lookup(&feed.read().unwrap(), p) else {
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
                    send(&mut s, "502 Bad Gateway", "text/plain", b"cannot resolve clip", head)
                }
            }
        }
        _ => send(&mut s, "404 Not Found", "text/plain", b"not found", head),
    }
}

/// Stream the upstream mp4, passing Range through so Safari gets 206 + Content-Range.
fn proxy(s: &mut TcpStream, url: &str, range: Option<&str>, head: bool) -> Result<()> {
    let mut req = fetch::agent().get(url);
    if let Some(r) = range {
        req = req.header("Range", r);
    }
    let resp = req.call().map_err(|e| anyhow!("{e}"))?;
    let st = resp.status().as_u16();
    let h = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string())
    };
    let reason = match st { 200 => "OK", 206 => "Partial Content", 416 => "Range Not Satisfiable", _ => "Upstream" };
    let mut out = format!(
        "HTTP/1.1 {st} {reason}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        h("content-type").unwrap_or_else(|| "video/mp4".into())
    );
    if let Some(l) = h("content-length") { out += &format!("Content-Length: {l}\r\n"); }
    if let Some(c) = h("content-range") { out += &format!("Content-Range: {c}\r\n"); }
    out += "\r\n";
    s.write_all(out.as_bytes())?;
    if head { return Ok(()); }
    let mut body = resp.into_body().into_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 { break; }
        if s.write_all(&buf[..n]).is_err() { break; } // client seeked / closed
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(id: &str, mp4: Option<&str>) -> Video {
        Video { id: id.into(), url: format!("https://x.com/a/status/{id}"), creator: "a".into(), name: None,
            caption: "c".into(), posted_at: String::new(), mp4: mp4.map(|s| s.into()), poster: None,
            duration_ms: None, likes: None, views: None }
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
        let j: serde_json::Value = serde_json::from_str(&queue_json(&[v("1", Some("u")), v("2", None)])).unwrap();
        let a = j["videos"].as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0]["handle"], "@a");
        assert_eq!(a[0]["src"], "/v/1.mp4");
        assert_eq!(a[0]["url"], "https://x.com/a/status/1");
    }
}
