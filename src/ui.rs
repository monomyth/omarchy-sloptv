//! Overlay layer: pure functions that turn state into ASS events for mpv's `osd-overlay`.
//! Palette: charcoal #1a1b1c, one accent lime #c3e878, greys. Flat fills only.
use crate::model::Video;

// ASS colours are BGR.
pub const LIME: &str = "78E8C3";
pub const CHARCOAL: &str = "1C1B1A";
pub const TILE_BG: &str = "2B2A29";
pub const WHITE: &str = "ECECEC";
pub const GREY: &str = "A0A0A0";
pub const DIM: &str = "707070";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Player,
    Grid,
    Card,
}

pub struct GridView<'a> {
    pub items: &'a [&'a Video],
    pub focus: usize,
    pub query: &'a str,
    pub searching: bool,
    pub tile_ready: &'a dyn Fn(&Video) -> bool,
}

pub struct View<'a> {
    pub w: f64,
    pub h: f64,
    pub font: &'a str,
    pub mode: Mode,
    pub chrome: f64,
    pub small_card: f64,
    pub video: Option<&'a Video>,
    pub progress: f64,
    pub status: String,
    pub notice: Option<String>,
    pub grid: Option<GridView<'a>>,
    pub help: bool,
}

fn alpha(a: f64) -> String {
    format!("{:02X}", ((1.0 - a.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

fn is_emoji(c: char) -> bool {
    let u = c as u32;
    u >= 0x1F000
        || (0x2190..=0x2BFF).contains(&u)
        || u == 0xFE0F
        || u == 0x200D
        || (0xE000..=0xF8FF).contains(&u)
        || (0x1F3FB..=0x1F3FF).contains(&u)
}

/// Make text safe for ASS and free of emoji/control characters.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !is_emoji(*c) && (!c.is_control() || *c == '\n'))
        .map(|c| match c {
            '{' => '(',
            '}' => ')',
            '\\' => '/',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Greedy word wrap by estimated glyph width; truncates with "..." after max_lines.
pub fn wrap(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let max_chars = max_chars.max(8);
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    let mut truncated = false;
    'outer: for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let need = if cur.is_empty() {
                word.chars().count()
            } else {
                cur.chars().count() + 1 + word.chars().count()
            };
            if need <= max_chars {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(&word);
                break;
            }
            if cur.is_empty() {
                let head: String = word.chars().take(max_chars).collect();
                word = word.chars().skip(max_chars).collect();
                cur = head;
            }
            lines.push(std::mem::take(&mut cur));
            if lines.len() == max_lines {
                truncated = true;
                break 'outer;
            }
            if word.is_empty() {
                break;
            }
        }
    }
    if !truncated && !cur.is_empty() {
        if lines.len() < max_lines {
            lines.push(cur);
        } else {
            truncated = true;
        }
    }
    if truncated {
        if let Some(l) = lines.last_mut() {
            let keep = max_chars.saturating_sub(3);
            let t: String = l.chars().take(keep).collect();
            *l = format!("{}...", t.trim_end());
        }
    }
    lines
}

pub fn one_line(text: &str, max_chars: usize) -> String {
    wrap(text, max_chars, 1)
        .into_iter()
        .next()
        .unwrap_or_default()
}

struct Out<'a> {
    ev: Vec<String>,
    font: &'a str,
}

impl Out<'_> {
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, col: &str, a: f64) {
        if a <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        self.ev.push(format!(
            "{{\\an7\\pos({:.1},{:.1})\\bord0\\shad0\\1c&H{}&\\1a&H{}&\\p1}}m 0 0 l {:.1} 0 {:.1} {:.1} 0 {:.1}",
            x, y, col, alpha(a), w, w, h, h
        ));
    }
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: f64, y: f64, an: u8, size: f64, col: &str, a: f64, lines: &[String]) {
        if a <= 0.0 || lines.is_empty() {
            return;
        }
        self.ev.push(format!(
            "{{\\an{}\\pos({:.1},{:.1})\\fn{}\\fs{:.0}\\b0\\bord0\\shad0\\q2\\1c&H{}&\\1a&H{}&}}{}",
            an,
            x,
            y,
            self.font,
            size,
            col,
            alpha(a),
            lines.join("\\N")
        ));
    }
}

// ---------- grid geometry (shared with the poster-bitmap placement in main) ----------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct GridGeom {
    pub cols: usize,
    pub rows: usize,
    pub tile_w: f64,
    pub tile_h: f64,
    pub row_h: f64,
    pub left: f64,
    pub top: f64,
    pub gap: f64,
    #[allow(dead_code)]
    pub label_h: f64,
}

pub const COLS: usize = 4;

pub fn grid_geom(w: f64, h: f64) -> GridGeom {
    let s = h / 1080.0;
    let margin = 56.0 * s;
    let gap = 24.0 * s;
    let tile_w = ((w - 2.0 * margin - gap * (COLS as f64 - 1.0)) / COLS as f64).floor();
    let tile_h = (tile_w * 9.0 / 16.0).floor();
    let label_h = 50.0 * s;
    let row_h = tile_h + label_h + gap;
    let top = 112.0 * s;
    let rows = (((h - top - margin * 0.5) / row_h).floor() as usize).clamp(1, 15);
    GridGeom {
        cols: COLS,
        rows,
        tile_w,
        tile_h,
        row_h,
        left: margin,
        top,
        gap,
        label_h,
    }
}

/// Visible page (page-flip by rows) -> (first item index, count).
pub fn grid_page(g: &GridGeom, focus: usize, total: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let focus = focus.min(total - 1);
    let per_page = g.cols * g.rows;
    let row = focus / g.cols;
    let first_row = (row / g.rows) * g.rows;
    let start = first_row * g.cols;
    (start, per_page.min(total - start))
}

pub fn tile_rect(g: &GridGeom, slot: usize) -> Tile {
    let (r, c) = (slot / g.cols, slot % g.cols);
    Tile {
        x: g.left + c as f64 * (g.tile_w + g.gap),
        y: g.top + r as f64 * g.row_h,
        w: g.tile_w,
        h: g.tile_h,
    }
}

/// (item index, tile rect) for every visible tile.
pub fn visible_tiles(w: f64, h: f64, focus: usize, total: usize) -> Vec<(usize, Tile)> {
    let g = grid_geom(w, h);
    let (start, n) = grid_page(&g, focus, total);
    (0..n).map(|s| (start + s, tile_rect(&g, s))).collect()
}

fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn meta_line(v: &Video) -> String {
    let mut parts = vec![];
    if let Some(d) = v.posted_at.get(0..10) {
        parts.push(d.to_string());
    }
    if let Some(ms) = v.duration_ms {
        let s = ms / 1000;
        parts.push(format!("{}:{:02}", s / 60, s % 60));
    }
    if let Some(l) = v.likes {
        parts.push(format!("{} likes", fmt_count(l)));
    }
    if let Some(n) = v.views {
        parts.push(format!("{} views", fmt_count(n)));
    }
    parts.join("  ")
}

pub const HELP: &[(&str, &str)] = &[
    ("Space", "pause"),
    ("l  Right", "next clip"),
    ("h  Left", "previous clip"),
    ("j  k  Up  Down", "volume (grid: move focus)"),
    ("m", "mute"),
    ("s", "shuffle on/off"),
    ("n", "next clip (always)"),
    ("f", "fullscreen"),
    ("i", "credit card"),
    ("o", "open post in browser"),
    ("y", "copy post link"),
    ("Tab", "grid"),
    ("/", "search (in grid)"),
    ("Enter", "grid: play tile"),
    ("Esc", "clear search / close"),
    ("?", "this help"),
    ("q", "quit"),
];

pub fn render(v: &View) -> String {
    let s = v.h / 1080.0;
    let mut o = Out {
        ev: vec![],
        font: v.font,
    };
    let margin = 48.0 * s;
    let chrome = v.chrome.clamp(0.0, 1.0);

    // 2 px progress line along the bottom (chrome)
    if v.video.is_some() && v.mode != Mode::Grid {
        o.rect(
            0.0,
            v.h - 2.0,
            v.w * v.progress.clamp(0.0, 1.0),
            2.0,
            LIME,
            chrome,
        );
    }

    match v.mode {
        Mode::Player => {
            if let Some(vid) = v.video {
                if v.small_card > 0.0 {
                    small_card(&mut o, v, vid, s, margin);
                }
            }
        }
        Mode::Card => {
            if let Some(vid) = v.video {
                big_card(&mut o, v, vid, s);
            }
        }
        Mode::Grid => {
            if let Some(g) = &v.grid {
                grid(&mut o, v, g, s);
            }
        }
    }

    if chrome > 0.0 && !v.status.is_empty() {
        o.text(
            v.w - margin,
            margin * 0.7,
            9,
            22.0 * s,
            GREY,
            chrome,
            &[v.status.clone()],
        );
    }
    if let Some(n) = &v.notice {
        // the one-line network notice; always readable while shown
        o.text(margin, margin * 0.7, 7, 24.0 * s, LIME, 1.0, &[clean(n)]);
    }
    if v.help {
        help(&mut o, v, s);
    }
    o.ev.join("\n")
}

fn small_card(o: &mut Out, v: &View, vid: &Video, s: f64, margin: f64) {
    let a = v.small_card;
    let w = (520.0 * s).min(v.w * 0.5);
    let pad = 22.0 * s;
    let (fs_t, fs_h, fs_c, fs_u) = (30.0 * s, 24.0 * s, 22.0 * s, 20.0 * s);
    let cpl = ((w - 2.0 * pad) / (fs_c * 0.52)) as usize;
    let cap = wrap(&clean(&vid.caption), cpl, 3);
    let mut y = pad;
    let mut h = pad * 2.0 + fs_t * 1.25;
    let has_name = vid.name.is_some();
    if has_name {
        h += fs_h * 1.3;
    }
    if !cap.is_empty() {
        h += 8.0 * s + cap.len() as f64 * fs_c * 1.3;
    }
    h += 8.0 * s + fs_u * 1.3;
    let x0 = margin;
    let y0 = v.h - margin - h;
    o.rect(x0, y0, w, h, CHARCOAL, 0.92 * a);
    o.rect(x0, y0, 3.0 * s.max(1.0), h, LIME, a);
    let tx = x0 + pad;
    o.text(tx, y0 + y, 7, fs_t, WHITE, a, &[clean(&vid.title())]);
    y += fs_t * 1.25;
    if has_name {
        o.text(tx, y0 + y, 7, fs_h, LIME, a, &[vid.handle()]);
        y += fs_h * 1.3;
    }
    if !cap.is_empty() {
        y += 8.0 * s;
        o.text(tx, y0 + y, 7, fs_c, GREY, a, &cap);
        y += cap.len() as f64 * fs_c * 1.3;
    }
    y += 8.0 * s;
    o.text(tx, y0 + y, 7, fs_u, DIM, a, &[clean(&vid.short_url())]);
}

fn big_card(o: &mut Out, v: &View, vid: &Video, s: f64) {
    let w = v.w / 3.0;
    let x0 = v.w - w;
    let pad = 40.0 * s;
    o.rect(x0, 0.0, w, v.h, CHARCOAL, 0.96);
    o.rect(x0, 0.0, 3.0, v.h, LIME, 1.0);
    let tx = x0 + pad;
    let (fs_t, fs_c, fs_m) = (42.0 * s, 28.0 * s, 22.0 * s);
    let cpl = ((w - 2.0 * pad) / (fs_c * 0.52)) as usize;
    let mut y = 96.0 * s;
    o.text(
        tx,
        y,
        7,
        fs_t,
        WHITE,
        1.0,
        &wrap(
            &clean(&vid.title()),
            ((w - 2.0 * pad) / (fs_t * 0.52)) as usize,
            2,
        ),
    );
    y += fs_t * 1.3;
    if vid.name.is_some() {
        o.text(tx, y, 7, fs_c, LIME, 1.0, &[vid.handle()]);
        y += fs_c * 1.5;
    }
    y += 24.0 * s;
    let max_lines = (((v.h - y - 260.0 * s) / (fs_c * 1.35)) as usize).clamp(3, 40);
    let cap = wrap(&clean(&vid.caption), cpl, max_lines);
    o.text(tx, y, 7, fs_c, GREY, 1.0, &cap);
    y += cap.len() as f64 * fs_c * 1.35 + 28.0 * s;
    o.text(tx, y, 7, fs_m, DIM, 1.0, &[meta_line(vid)]);
    y += fs_m * 1.8;
    o.text(
        tx,
        y,
        7,
        fs_m,
        LIME,
        1.0,
        &wrap(
            &clean(&vid.url),
            ((w - 2.0 * pad) / (fs_m * 0.52)) as usize,
            3,
        ),
    );
    o.text(
        tx,
        v.h - 56.0 * s,
        7,
        fs_m,
        DIM,
        1.0,
        &["o open   y copy link   i close".to_string()],
    );
}

fn grid(o: &mut Out, v: &View, g: &GridView, s: f64) {
    // flat dim over the still-playing video (drawn first; poster bitmaps are composited above ASS by mpv)
    o.rect(0.0, 0.0, v.w, v.h, CHARCOAL, 0.78);
    let geom = grid_geom(v.w, v.h);
    let m = geom.left;
    let hdr = if g.searching || !g.query.is_empty() {
        format!(
            "/ {}{}   {} clips",
            clean(g.query),
            if g.searching { "_" } else { "" },
            g.items.len()
        )
    } else {
        format!("{} clips   / search   Esc back", g.items.len())
    };
    o.text(
        m,
        52.0 * s,
        7,
        26.0 * s,
        if g.searching { LIME } else { GREY },
        1.0,
        &[hdr],
    );
    if g.items.is_empty() {
        o.text(
            v.w / 2.0,
            v.h / 2.0,
            5,
            28.0 * s,
            GREY,
            1.0,
            &["no matches".to_string()],
        );
        return;
    }
    let (start, n) = grid_page(&geom, g.focus, g.items.len());
    for slot in 0..n {
        let idx = start + slot;
        let vid = g.items[idx];
        let t = tile_rect(&geom, slot);
        let focused = idx == g.focus;
        if !(g.tile_ready)(vid) {
            o.rect(t.x, t.y, t.w, t.h, TILE_BG, 1.0);
        }
        if focused {
            let r = (3.0 * s).max(2.0);
            o.rect(t.x - r, t.y - r, t.w + 2.0 * r, r, LIME, 1.0);
            o.rect(t.x - r, t.y + t.h, t.w + 2.0 * r, r, LIME, 1.0);
            o.rect(t.x - r, t.y, r, t.h, LIME, 1.0);
            o.rect(t.x + t.w, t.y, r, t.h, LIME, 1.0);
        }
        let cpl = (t.w / (22.0 * s * 0.52)) as usize;
        let ly = t.y + t.h + 10.0 * s;
        let (l1, l2) = match &vid.name {
            Some(n) => (clean(n), vid.handle()),
            None => (vid.handle(), clean(&vid.caption)),
        };
        o.text(
            t.x,
            ly,
            7,
            22.0 * s,
            if focused { LIME } else { WHITE },
            1.0,
            &[one_line(&l1, cpl)],
        );
        o.text(
            t.x,
            ly + 25.0 * s,
            7,
            20.0 * s,
            if focused { GREY } else { DIM },
            1.0,
            &[one_line(&l2, cpl)],
        );
    }
}

fn help(o: &mut Out, v: &View, s: f64) {
    let w = 760.0 * s;
    let rowh = 34.0 * s;
    let h = rowh * (HELP.len() as f64 + 2.0) + 40.0 * s;
    let x0 = (v.w - w) / 2.0;
    let y0 = (v.h - h) / 2.0;
    o.rect(x0, y0, w, h, CHARCOAL, 0.97);
    o.rect(x0, y0, w, 3.0, LIME, 1.0);
    o.text(
        x0 + 36.0 * s,
        y0 + 30.0 * s,
        7,
        28.0 * s,
        LIME,
        1.0,
        &["slopTV".to_string()],
    );
    let keys: Vec<String> = HELP.iter().map(|(k, _)| (*k).to_string()).collect();
    let acts: Vec<String> = HELP.iter().map(|(_, a)| (*a).to_string()).collect();
    let ly = y0 + 30.0 * s + rowh * 1.5;
    // per-row events keep columns aligned regardless of font metrics
    for (i, (k, a)) in keys.iter().zip(acts.iter()).enumerate() {
        let y = ly + i as f64 * rowh;
        o.text(
            x0 + 36.0 * s,
            y,
            7,
            22.0 * s,
            WHITE,
            1.0,
            std::slice::from_ref(k),
        );
        o.text(
            x0 + 290.0 * s,
            y,
            7,
            22.0 * s,
            GREY,
            1.0,
            std::slice::from_ref(a),
        );
    }
}

/// Fade factor: 1.0 while within `hold` seconds of the last input, then linear fade over `fade` seconds.
pub fn fade(since: f64, hold: f64, fade: f64) -> f64 {
    if since <= hold {
        1.0
    } else if since >= hold + fade {
        0.0
    } else {
        1.0 - (since - hold) / fade
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_any;

    fn vid() -> Video {
        parse_any(br#"[{"id":"1","creator":"cube__lol","caption":"SORRY FOR PARTY ROCKING (closed-beta video) with a very long caption that goes on and on and on and on and on and on and on and on and on and on and on","url":"https://x.com/cube__lol/status/1","posted_at":"2026-08-25T01:24:35Z","duration_ms":50549,"likes":196,"views":11791}]"#).unwrap().remove(0)
    }
    fn view(mode: Mode, v: &Video) -> View<'_> {
        View {
            w: 1920.0,
            h: 1080.0,
            font: "sans-serif",
            mode,
            chrome: 1.0,
            small_card: 1.0,
            video: Some(v),
            progress: 0.5,
            status: "vol 60".into(),
            notice: None,
            grid: None,
            help: false,
        }
    }

    #[test]
    fn wrap_limits_lines_and_truncates() {
        let l = wrap(
            "one two three four five six seven eight nine ten eleven twelve",
            14,
            3,
        );
        assert_eq!(l.len(), 3);
        assert!(l.iter().all(|x| x.chars().count() <= 14));
        assert!(l[2].ends_with("..."));
        assert_eq!(wrap("short", 20, 3), vec!["short"]);
        assert_eq!(wrap("", 20, 3).len(), 0);
        let l = wrap("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 10, 5);
        assert!(l.len() >= 3 && l.iter().all(|x| x.chars().count() <= 10));
    }
    #[test]
    fn clean_strips_emoji_and_ass_chars() {
        assert_eq!(
            clean("hi \u{1F680}\u{1F525} {\\an5}there\n x"),
            "hi (/an5)there x"
        );
        assert_eq!(clean("a\u{FE0F}b"), "ab");
    }
    #[test]
    fn small_card_has_progress_and_three_line_caption() {
        let v = vid();
        let s = render(&view(Mode::Player, &v));
        assert!(s.contains("&H78E8C3&"), "lime accent present");
        assert!(s.contains("@cube__lol"));
        assert!(s.contains("x.com/cube__lol/status/1"));
        assert!(!s.contains("https://"));
        let cap_event = s.lines().find(|l| l.contains("SORRY")).unwrap();
        assert!(cap_event.matches("\\N").count() <= 2, "max 3 lines");
        assert!(cap_event.contains("..."));
        // progress line is 2px high at bottom, half the width
        assert!(s.contains("\\pos(0.0,1078.0)") && s.contains("l 960.0 0"));
    }
    #[test]
    fn chrome_fades_but_card_alpha_independent() {
        let v = vid();
        let mut vw = view(Mode::Player, &v);
        vw.chrome = 0.0;
        vw.small_card = 0.0;
        assert_eq!(render(&vw), "");
        vw.small_card = 1.0;
        let s = render(&vw);
        assert!(s.contains("@cube__lol") && !s.contains("vol 60"));
    }
    #[test]
    fn card_mode_right_third_full_caption() {
        let v = vid();
        let s = render(&view(Mode::Card, &v));
        assert!(s.contains("\\pos(1280.0,0.0)"), "panel starts at 2/3 width");
        assert!(s.contains("https://x.com/cube__lol/status/1"));
        assert!(!s.contains("..."), "full caption is not truncated");
        assert!(s.contains("196 likes"));
    }
    #[test]
    fn grid_geometry_four_columns_in_bounds() {
        for (w, h) in [(1920.0, 1080.0), (1280.0, 720.0), (3840.0, 2160.0)] {
            let g = grid_geom(w, h);
            assert_eq!(g.cols, 4);
            for slot in 0..g.cols * g.rows {
                let t = tile_rect(&g, slot);
                assert!(
                    t.x >= 0.0 && t.x + t.w <= w && t.y + t.h + g.label_h <= h,
                    "{w}x{h} slot {slot}"
                );
            }
            assert!(g.rows * g.cols <= 60, "fits mpv's 64 overlay ids");
        }
    }
    #[test]
    fn grid_paging() {
        let g = grid_geom(1920.0, 1080.0);
        let per = g.cols * g.rows;
        assert_eq!(grid_page(&g, 0, 594), (0, per));
        assert_eq!(grid_page(&g, per, 594).0, per);
        assert_eq!(grid_page(&g, 593, 594).0 + grid_page(&g, 593, 594).1, 594);
        assert_eq!(grid_page(&g, 0, 0), (0, 0));
        assert_eq!(visible_tiles(1920.0, 1080.0, 5, 3).len(), 3);
    }
    #[test]
    fn grid_render_focus_rim_and_labels() {
        let v = vid();
        let items = vec![&v, &v];
        let ready = |_: &Video| false;
        let mut vw = view(Mode::Grid, &v);
        vw.grid = Some(GridView {
            items: &items,
            focus: 1,
            query: "cube",
            searching: true,
            tile_ready: &ready,
        });
        let s = render(&vw);
        assert!(s.contains("/ cube_"));
        let lime_rects = s
            .lines()
            .filter(|l| l.contains("\\p1") && l.contains("&H78E8C3&"))
            .count();
        assert_eq!(lime_rects, 4, "rim is four lime edges");
        assert!(s.contains("@cube__lol"));
    }
    #[test]
    fn help_lists_all_keys() {
        let v = vid();
        let mut vw = view(Mode::Player, &v);
        vw.help = true;
        let s = render(&vw);
        for (k, _) in HELP {
            assert!(s.contains(k), "{k}");
        }
        assert!(!s.chars().any(is_emoji));
    }
    #[test]
    fn fade_curve() {
        assert_eq!(fade(0.5, 2.0, 0.4), 1.0);
        assert!((fade(2.2, 2.0, 0.4) - 0.5).abs() < 1e-9);
        assert_eq!(fade(3.0, 2.0, 0.4), 0.0);
    }
}
