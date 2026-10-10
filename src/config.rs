use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_SOURCE: &str = "https://jon-slop.grok.me/videos.json";
pub const DEFAULT_FALLBACK: &str = "https://jon-slop.grok.me";

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// videos.json URL, or a local path / file:// URL (for development).
    pub source_url: String,
    /// Last-resort page with the embedded `<script id="data">` JSON. Empty string disables it.
    pub fallback_url: String,
    pub volume: i64,
    pub shuffle: bool,
    /// true: loop the current clip until `n`; false: advance at the end of a clip.
    pub loop_clip: bool,
    pub fullscreen: bool,
    pub cache_max_mb: u64,
    pub poll_minutes: u64,
    pub font: String,
    pub hwdec: String,
    /// Optional mpv overrides (leave empty for mpv's defaults: gpu on Wayland).
    pub vo: String,
    pub ao: String,
    pub gpu_context: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source_url: DEFAULT_SOURCE.into(),
            fallback_url: DEFAULT_FALLBACK.into(),
            volume: 60,
            shuffle: true,
            loop_clip: true,
            fullscreen: true,
            cache_max_mb: 2048,
            poll_minutes: 60,
            font: "sans-serif".into(),
            hwdec: "auto-safe".into(),
            vo: String::new(),
            ao: String::new(),
            gpu_context: String::new(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
        let mut c: Config = toml::from_str(text)?;
        c.volume = c.volume.clamp(0, 130);
        Ok(c)
    }
    pub fn load(path: &std::path::Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(t) => Config::parse(&t).unwrap_or_else(|e| {
                eprintln!(
                    "slop-tv: bad config {}: {e} (using defaults)",
                    path.display()
                );
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }
    pub fn host(&self) -> String {
        host_of(&self.source_url)
    }
}

pub fn host_of(url: &str) -> String {
    let u = url.split("://").nth(1).unwrap_or(url);
    u.split('/').next().unwrap_or(u).to_string()
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("slop-tv/config.toml")
}
pub fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("slop-tv")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_partial() {
        let d = Config::default();
        assert!(d.shuffle && d.loop_clip);
        assert_eq!(d.source_url, DEFAULT_SOURCE);
        let c =
            Config::parse("volume = 30\nshuffle = false\nsource_url = \"/tmp/v.json\"").unwrap();
        assert_eq!(c.volume, 30);
        assert!(!c.shuffle);
        assert_eq!(c.source_url, "/tmp/v.json");
        assert_eq!(c.cache_max_mb, 2048);
        assert_eq!(Config::parse("volume = 900").unwrap().volume, 130);
        assert!(Config::parse("volume = \"x\"").is_err());
    }
    #[test]
    fn host() {
        assert_eq!(
            host_of("https://jon-slop.grok.me/videos.json"),
            "jon-slop.grok.me"
        );
    }
}
