//! slopTV: keyboard-driven player for the Slop Cannon feed.
//! Default: X's official embedded player in a Chromium app window (see embed.rs).
//! Hidden, unofficial: libmpv raw-mp4 streaming (`--raw-mpv` / `raw_mpv = true`, feature `raw-mpv`).
#![cfg_attr(not(feature = "raw-mpv"), allow(dead_code))]
mod args;
mod cache;
mod config;
mod embed;
mod fetch;
mod hypr;
mod model;
mod queue;
#[cfg(feature = "raw-mpv")]
mod raw;
#[cfg(feature = "raw-mpv")]
mod ui;

use anyhow::Result;
use cache::Cache;
use config::Config;

fn main() -> Result<()> {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("slop-tv: {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        println!("slopTV {}\n\n{}", env!("CARGO_PKG_VERSION"), args::HELP);
        return Ok(());
    }
    let cfg_path = args.config.clone().unwrap_or_else(config::config_path);
    let mut cfg = Config::load(&cfg_path);
    if args.download {
        cfg.download_clips = true;
    }
    if let Some(s) = &args.source {
        cfg.source_url = s.clone();
        if config::host_of(s) != config::host_of(config::DEFAULT_SOURCE) {
            cfg.fallback_url = String::new();
        }
    }
    let cache = Cache::new(args.cache_dir.clone().unwrap_or_else(config::cache_root));
    if args.check {
        let r = fetch::load_feed(&cfg, &cache);
        println!(
            "source: {}\nonline: {}\nvideos: {}",
            cfg.source_url,
            r.online,
            r.videos.len()
        );
        if let Some(v) = r.videos.first() {
            println!("newest: {} @{} {}", v.posted_at, v.creator, v.short_url());
        }
        println!("cache: {} ({} bytes)", cache.root.display(), cache.size());
        if !r.online {
            println!("Can't reach {}", cfg.host());
        }
        return Ok(());
    }
    if config::raw_mode(&cfg, args.raw_mpv) {
        #[cfg(feature = "raw-mpv")]
        return raw::run(cfg, cache, &args);
        #[cfg(not(feature = "raw-mpv"))]
        {
            eprintln!("slop-tv: raw mpv mode is not built into this binary (built without the raw-mpv feature)");
            std::process::exit(2);
        }
    }
    embed::run(cfg, cache, args.reset_notice)
}
