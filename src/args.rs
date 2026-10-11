//! Command-line parsing. `--raw-mpv` and `--download` are accepted but deliberately not in --help.
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub source: Option<String>,
    pub config: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub windowed: bool,
    pub geometry: Option<String>,
    pub check: bool,
    pub help: bool,
    /// Hidden: unofficial libmpv raw-mp4 mode.
    pub raw_mpv: bool,
    /// Hidden: raw mode only, also save clips to the cache.
    pub download: bool,
    /// Forget the one-time embed privacy notice acknowledgement.
    pub reset_notice: bool,
}

pub const HELP: &str = "usage: slop-tv [--source URL|PATH] [--config FILE] [--cache-dir DIR] [--check] [--reset-notice]

Opens the official X embedded player for each post in a Chromium app window.

  --check          fetch and parse the feed, print a summary, exit (no window)
  --reset-notice   show the one-time privacy notice again

config: ~/.config/slop-tv/config.toml   cache: ~/.cache/slop-tv/
Keys: l/h next/prev, p pause, a auto-advance, s shuffle, f like, o open on X, w/F11 fullscreen, q quit.";

pub fn parse<I: Iterator<Item = String>>(mut it: I) -> Result<Args, String> {
    let mut a = Args::default();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--source" => a.source = it.next(),
            "--config" => a.config = it.next().map(PathBuf::from),
            "--cache-dir" => a.cache_dir = it.next().map(PathBuf::from),
            "--windowed" => a.windowed = true,
            "--geometry" => a.geometry = it.next(),
            "--check" => a.check = true,
            "--raw-mpv" => a.raw_mpv = true,
            "--download" => a.download = true,
            "--reset-notice" => a.reset_notice = true,
            "-h" | "--help" => a.help = true,
            other => return Err(format!("unknown argument {other} (try --help)")),
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(v: &[&str]) -> Result<Args, String> {
        parse(v.iter().map(|s| s.to_string()))
    }
    #[test]
    fn raw_flags_default_off_and_parse() {
        let a = p(&[]).unwrap();
        assert!(!a.raw_mpv && !a.download);
        assert!(p(&["--raw-mpv"]).unwrap().raw_mpv);
        assert!(p(&["--raw-mpv", "--download"]).unwrap().download);
        assert!(p(&["--bogus"]).is_err());
    }
    #[test]
    fn hidden_flags_not_in_help() {
        assert!(!HELP.contains("raw-mpv") && !HELP.contains("--download"));
        assert!(!HELP.to_lowercase().contains("mpv"));
    }
}
