//! Video records and tolerant parsing of the feed.
//!
//! Accepted inputs (in order of preference):
//!  1. `videos.json` envelope: `{"name":..,"videos":[{id,url,creator,caption,posted_at,mp4,poster,
//!     duration_ms,likes,views,phase,unsure}, ...]}` (the live feed, newest first)
//!  2. a bare JSON array of the same records
//!  3. the embedded page JSON (`<script id="data">[{id,url,c,t,ti,mp,po,du,l,v,...}]</script>`)
//!     whose short keys are accepted as aliases.
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub url: String,
    pub creator: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub posted_at: String,
    #[serde(default)]
    pub mp4: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub likes: Option<u64>,
    #[serde(default)]
    pub views: Option<u64>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(deserialize_with = "stringish")]
    id: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(alias = "c", default)]
    creator: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(alias = "ti", default)]
    caption: Option<String>,
    #[serde(alias = "time", alias = "t", default)]
    posted_at: Option<String>,
    #[serde(alias = "mp", default)]
    mp4: Option<String>,
    #[serde(alias = "po", default)]
    poster: Option<String>,
    #[serde(alias = "du", alias = "duration", default)]
    duration_ms: Option<f64>,
    #[serde(alias = "l", default)]
    likes: Option<f64>,
    #[serde(alias = "v", default)]
    views: Option<f64>,
}

fn stringish<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom("id must be string or number")),
    }
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl Raw {
    fn into_video(self) -> Option<Video> {
        let creator = nonempty(self.creator)?.trim_start_matches('@').to_string();
        let url = nonempty(self.url)
            .unwrap_or_else(|| format!("https://x.com/{}/status/{}", creator, self.id));
        Some(Video {
            id: self.id,
            url,
            creator,
            name: nonempty(self.name),
            caption: self.caption.unwrap_or_default(),
            posted_at: self.posted_at.unwrap_or_default(),
            mp4: nonempty(self.mp4),
            poster: nonempty(self.poster),
            duration_ms: self.duration_ms.map(|x| x as u64),
            likes: self.likes.map(|x| x as u64),
            views: self.views.map(|x| x as u64),
        })
    }
}

/// Pull the JSON text out of `<script id="data" ...>...</script>`.
pub fn extract_embedded(html: &str) -> Option<&str> {
    let i = html.find("id=\"data\"")?;
    let rest = &html[i..];
    let start = rest.find('>')? + 1;
    let end = rest[start..].find("</script>")?;
    Some(&rest[start..start + end])
}

/// Parse any supported feed shape. Records that cannot be understood are skipped.
pub fn parse_any(bytes: &[u8]) -> Result<Vec<Video>> {
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow!("feed is not UTF-8"))?;
    let t = text.trim_start();
    let json: &str = if t.starts_with('<') {
        extract_embedded(t).ok_or_else(|| anyhow!("no <script id=\"data\"> block in page"))?
    } else {
        t
    };
    let v: serde_json::Value = serde_json::from_str(json)?;
    let arr = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(mut o) => match o.remove("videos") {
            Some(serde_json::Value::Array(a)) => a,
            _ => return Err(anyhow!("object has no \"videos\" array")),
        },
        _ => return Err(anyhow!("unexpected JSON shape")),
    };
    let total = arr.len();
    let vids: Vec<Video> = arr
        .into_iter()
        .filter_map(|x| serde_json::from_value::<Raw>(x).ok())
        .filter_map(Raw::into_video)
        .collect();
    if total > 0 && vids.is_empty() {
        return Err(anyhow!("no usable records in feed"));
    }
    Ok(vids)
}

impl Video {
    /// "x.com/handle/status/123" (no scheme).
    pub fn short_url(&self) -> String {
        self.url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_end_matches('/')
            .to_string()
    }
    pub fn handle(&self) -> String {
        format!("@{}", self.creator)
    }
    /// Title line of a card: display name when the feed has one, otherwise the @handle.
    pub fn title(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.handle())
    }
    /// Case-insensitive AND match of every whitespace-separated token against creator, name, caption.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!(
            "{} {} {}",
            self.creator,
            self.name.as_deref().unwrap_or(""),
            self.caption
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .map(|t| t.trim_start_matches('@'))
            .all(|t| hay.contains(t))
    }
}

pub fn filter(videos: &[Video], query: &str) -> Vec<usize> {
    videos
        .iter()
        .enumerate()
        .filter(|(_, v)| v.matches(query))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/videos.json"
        ))
        .unwrap()
    }
    #[test]
    fn parses_live_feed_snapshot() {
        let v = parse_any(&sample()).unwrap();
        assert_eq!(v.len(), 20);
        assert_eq!(v[0].id, "2108935501126271427");
        assert_eq!(v[0].creator, "Cryptonic_1200");
        assert!(v[0]
            .mp4
            .as_deref()
            .unwrap()
            .starts_with("https://video.twimg.com/"));
        assert_eq!(v[0].duration_ms, Some(163640));
        assert_eq!(v[0].name, None);
        assert!(v[0].posted_at > v[19].posted_at, "newest first");
        assert!(v.iter().all(|x| x.mp4.is_some() && x.poster.is_some()));
    }
    #[test]
    fn parses_embedded_page() {
        let html = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/page-embedded.html"
        ))
        .unwrap();
        let v = parse_any(&html).unwrap();
        assert_eq!(v.len(), 10);
        let live = parse_any(&sample()).unwrap();
        let ids: std::collections::HashSet<_> = live.iter().map(|x| x.id.clone()).collect();
        assert!(v.iter().all(|x| ids.contains(&x.id)));
        assert!(v.iter().all(|x| !x.creator.is_empty() && !x.url.is_empty()));
    }
    #[test]
    fn bare_array_numeric_id_and_junk() {
        let j = br#"[{"id":5,"creator":"@a","caption":"hi"},{"nope":1},{"id":"6","c":"b","ti":"yo","mp":"http://x/y.mp4","du":1500.0}]"#;
        let v = parse_any(j).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "5");
        assert_eq!(v[0].creator, "a");
        assert_eq!(v[0].url, "https://x.com/a/status/5");
        assert_eq!(v[1].mp4.as_deref(), Some("http://x/y.mp4"));
        assert_eq!(v[1].duration_ms, Some(1500));
    }
    #[test]
    fn rejects_garbage() {
        assert!(parse_any(b"<html>nothing</html>").is_err());
        assert!(parse_any(b"{\"a\":1}").is_err());
        assert!(parse_any(b"not json").is_err());
        assert_eq!(parse_any(b"{\"videos\":[]}").unwrap().len(), 0);
    }
    #[test]
    fn card_fields() {
        let v = parse_any(
            br#"[{"id":"1","creator":"cube__lol","url":"https://x.com/cube__lol/status/1"}]"#,
        )
        .unwrap();
        assert_eq!(v[0].short_url(), "x.com/cube__lol/status/1");
        assert_eq!(v[0].title(), "@cube__lol");
    }
    #[test]
    fn search_filter() {
        let v = parse_any(&sample()).unwrap();
        let all = filter(&v, "");
        assert_eq!(all.len(), v.len());
        let h = filter(&v, "@cube__lol");
        assert!(h.iter().any(|&i| v[i].creator == "cube__lol"));
        let c = filter(&v, "PARTY rocking");
        assert!(c.iter().any(|&i| v[i].id == "2092060538335351253"));
        assert!(filter(&v, "zzzzqqqq").is_empty());
    }
}
