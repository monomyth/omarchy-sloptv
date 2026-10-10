//! Feed loading (ETag-conditional), file downloads, poster tiles.
use crate::cache::Cache;
use crate::config::Config;
use crate::model::{parse_any, Video};
use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .timeout_connect(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .user_agent("slopTV/0.1 (+https://jon-slop.grok.me)")
        .build()
        .into()
}

pub enum Got {
    NotModified,
    Body(Vec<u8>, Option<String>),
}

pub fn is_remote(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
}

/// GET with If-None-Match. Local paths / file:// are read directly (development).
pub fn get_conditional(src: &str, etag: Option<&str>) -> Result<Got> {
    if !is_remote(src) {
        let p = src.strip_prefix("file://").unwrap_or(src);
        return Ok(Got::Body(std::fs::read(p)?, None));
    }
    let mut req = agent()
        .get(src)
        .header("Accept", "application/json, text/html;q=0.5");
    if let Some(e) = etag {
        req = req.header("If-None-Match", e);
    }
    let resp = req.call().map_err(|e| anyhow!("{e}"))?;
    let status = resp.status().as_u16();
    if status == 304 {
        return Ok(Got::NotModified);
    }
    if status != 200 {
        return Err(anyhow!("HTTP {status}"));
    }
    let et = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = resp
        .into_body()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| anyhow!("{e}"))?;
    Ok(Got::Body(body, et))
}

pub struct FeedResult {
    pub videos: Vec<Video>,
    /// true when the network answered (200 or 304); false when we fell back to the on-disk copy.
    pub online: bool,
}

pub fn read_cached(cache: &Cache) -> Vec<Video> {
    std::fs::read(cache.feed())
        .ok()
        .and_then(|b| parse_any(&b).ok())
        .unwrap_or_default()
}

/// Load the feed: source_url (ETag) -> embedded-page fallback (last resort) -> cached copy.
pub fn load_feed(cfg: &Config, cache: &Cache) -> FeedResult {
    let cached = read_cached(cache);
    let etag = if cached.is_empty() {
        None
    } else {
        std::fs::read_to_string(cache.etag())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    match get_conditional(&cfg.source_url, etag.as_deref()) {
        Ok(Got::NotModified) => {
            return FeedResult {
                videos: cached,
                online: true,
            }
        }
        Ok(Got::Body(b, et)) => match parse_any(&b) {
            Ok(v) if !v.is_empty() => {
                let _ = std::fs::write(cache.feed(), &b);
                match et {
                    Some(e) => {
                        let _ = std::fs::write(cache.etag(), e);
                    }
                    None => {
                        let _ = std::fs::remove_file(cache.etag());
                    }
                }
                return FeedResult {
                    videos: v,
                    online: true,
                };
            }
            Ok(_) => eprintln!("slop-tv: feed is empty"),
            Err(e) => eprintln!("slop-tv: cannot parse {}: {e}", cfg.source_url),
        },
        Err(e) => eprintln!("slop-tv: fetch {} failed: {e}", cfg.source_url),
    }
    if !cfg.fallback_url.is_empty() && cfg.fallback_url != cfg.source_url {
        match get_conditional(&cfg.fallback_url, None) {
            Ok(Got::Body(b, _)) => match parse_any(&b) {
                Ok(v) if !v.is_empty() => {
                    let env = serde_json::json!({ "name": "slopTV cache (from embedded page)", "videos": v });
                    let _ =
                        std::fs::write(cache.feed(), serde_json::to_vec(&env).unwrap_or_default());
                    let _ = std::fs::remove_file(cache.etag());
                    return FeedResult {
                        videos: v,
                        online: true,
                    };
                }
                _ => eprintln!("slop-tv: fallback page had no usable data"),
            },
            Ok(Got::NotModified) => {}
            Err(e) => eprintln!("slop-tv: fallback {} failed: {e}", cfg.fallback_url),
        }
    }
    FeedResult {
        videos: cached,
        online: false,
    }
}

/// Download to `dest` via dest.part + rename. Local paths are copied (development).
pub fn download(url: &str, dest: &Path) -> Result<u64> {
    let part = dest.with_extension("part");
    let r = (|| -> Result<u64> {
        let mut out = std::fs::File::create(&part)?;
        let n = if is_remote(url) {
            let resp = agent().get(url).call().map_err(|e| anyhow!("{e}"))?;
            let st = resp.status().as_u16();
            if st != 200 {
                return Err(anyhow!("HTTP {st}"));
            }
            let mut rd = resp.into_body().into_reader();
            std::io::copy(&mut rd, &mut out)?
        } else {
            let mut f = std::fs::File::open(url.strip_prefix("file://").unwrap_or(url))?;
            std::io::copy(&mut f, &mut out)?
        };
        out.flush()?;
        if n == 0 {
            return Err(anyhow!("empty download"));
        }
        std::fs::rename(&part, dest)?;
        Ok(n)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    r
}

#[cfg(feature = "raw-mpv")]
/// Decode a poster (jpg/png), cover-crop to w x h, write raw BGRA (opaque) for mpv `overlay-add`.
pub fn make_tile(poster: &Path, out: &Path, w: u32, h: u32) -> Result<()> {
    let img = image::ImageReader::open(poster)?
        .with_guessed_format()?
        .decode()?
        .to_rgba8();
    let (iw, ih) = img.dimensions();
    let scale = (w as f64 / iw as f64).max(h as f64 / ih as f64);
    let (sw, sh) = (
        ((iw as f64 * scale).ceil() as u32).max(w),
        ((ih as f64 * scale).ceil() as u32).max(h),
    );
    let rs = image::imageops::resize(&img, sw, sh, image::imageops::FilterType::Triangle);
    let (x0, y0) = ((sw - w) / 2, (sh - h) / 2);
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = rs.get_pixel(x0 + x, y0 + y).0;
            buf.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    let tmp = out.with_extension("tmp");
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, out)?;
    Ok(())
}

#[allow(dead_code)]
fn _unused(_: &mut dyn Read) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// Tiny HTTP server: 200 with ETag, then 304 when If-None-Match matches; counts requests.
    fn serve(body: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<bool>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/videos.json", l.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut rd = BufReader::new(s.try_clone().unwrap());
                let mut inm = false;
                loop {
                    let mut line = String::new();
                    if rd.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if line.to_lowercase().starts_with("if-none-match:") && line.contains("\"v1\"")
                    {
                        inm = true;
                    }
                }
                log2.lock().unwrap().push(inm);
                let resp = if inm {
                    "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    format!("HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (url, log)
    }

    #[test]
    fn etag_roundtrip_200_then_304_then_offline_cache() {
        let (url, log) = serve(r#"{"videos":[{"id":"1","creator":"a","caption":"x"}]}"#);
        let dir = std::env::temp_dir().join(format!("slop-tv-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(dir.clone());
        let cfg = Config {
            source_url: url.clone(),
            fallback_url: String::new(),
            ..Config::default()
        };
        let r = load_feed(&cfg, &cache);
        assert!(r.online && r.videos.len() == 1);
        let r = load_feed(&cfg, &cache);
        assert!(r.online && r.videos.len() == 1, "304 serves cached copy");
        assert_eq!(
            *log.lock().unwrap(),
            vec![false, true],
            "second request was conditional"
        );
        // dead server -> offline, cached list still returned
        let dead = Config {
            source_url: "http://127.0.0.1:1/videos.json".into(),
            fallback_url: String::new(),
            ..Config::default()
        };
        let r = load_feed(&dead, &cache);
        assert!(!r.online);
        assert_eq!(r.videos.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_embedded_page_then_local_files() {
        let dir = std::env::temp_dir().join(format!("slop-tv-fb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(dir.clone());
        let page = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/page-embedded.html"
        );
        let cfg = Config {
            source_url: "http://127.0.0.1:1/videos.json".into(),
            fallback_url: page.into(),
            ..Config::default()
        };
        let r = load_feed(&cfg, &cache);
        assert!(r.online);
        assert_eq!(r.videos.len(), 10);
        assert_eq!(read_cached(&cache).len(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "raw-mpv")]
    #[test]
    fn download_and_tile() {
        let dir = std::env::temp_dir().join(format!("slop-tv-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        std::fs::write(&src, b"hello").unwrap();
        let dst = dir.join("dst.bin");
        assert_eq!(download(src.to_str().unwrap(), &dst).unwrap(), 5);
        assert!(download("/nonexistent/x", &dir.join("n.bin")).is_err());
        assert!(!dir.join("n.part").exists());
        let png = dir.join("p.png");
        image::RgbaImage::from_pixel(64, 36, image::Rgba([10, 20, 30, 255]))
            .save(&png)
            .unwrap();
        let out = dir.join("t.bgra");
        make_tile(&png, &out, 40, 30).unwrap();
        let b = std::fs::read(&out).unwrap();
        assert_eq!(b.len(), 40 * 30 * 4);
        assert_eq!(&b[0..4], &[30, 20, 10, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
