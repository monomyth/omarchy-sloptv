//! Hyprland-native fullscreen for the slop-tv window. Chromium's own requestFullscreen() from a page whose
//! focus sits inside X's iframe, or a compositor that ignores a client's request, can leave `w` doing nothing,
//! so the controller drives the compositor directly: find the slop-tv window, focus it by address, toggle.
use serde_json::Value;
use std::process::Command;

pub const CLASS: &str = "slop-tv";

/// The hyprctl binary; `SLOPTV_HYPRCTL` overrides it (tests use a recording shim).
fn hyprctl() -> String {
    std::env::var("SLOPTV_HYPRCTL").unwrap_or_else(|_| "hyprctl".into())
}

pub fn available() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// Address and fullscreen state of our window from `hyprctl clients -j` output.
/// On Wayland Chromium derives the app-id from the --app URL and profile, so `--class=slop-tv` is ignored and
/// the class looks like `chrome-__tmp_slop-tv-launch-<controller pid>.html-Default`. Any class containing
/// `slop-tv` matches; one containing our controller pid is preferred (several slopTV instances, or a
/// leftover window, cannot steal the toggle).
pub fn find_window(clients_json: &str, pid: Option<u32>) -> Option<(String, bool)> {
    let v: Value = serde_json::from_str(clients_json).ok()?;
    let mut hits = v
        .as_array()?
        .iter()
        .filter(|c| c["class"].as_str().is_some_and(|k| k.contains(CLASS)))
        .collect::<Vec<_>>();
    if let Some(p) = pid {
        let mine = format!("slop-tv-launch-{p}.");
        if let Some(i) = hits
            .iter()
            .position(|c| c["class"].as_str().is_some_and(|k| k.contains(&mine)))
        {
            hits = vec![hits[i]];
        }
    }
    let w = hits.first()?;
    let addr = w["address"].as_str()?.to_string();
    if !addr.starts_with("0x") || !addr[2..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let fs = w["fullscreen"].as_i64().unwrap_or(0) > 0 || w["fullscreen"] == true;
    Some((addr, fs))
}

/// Lua-config Hyprland (0.55+): the old `hyprctl dispatch <name> <args>` text form is rejected there
/// ("return hl.dispatch(fullscreen 0)"), so dispatch through `hyprctl eval` and set the state explicitly:
/// focus the window by address, then fullscreen_state internal=N client=N (2 = fullscreen, 0 = off).
pub fn toggle_cmds(addr: &str, currently_fullscreen: bool) -> Vec<Vec<String>> {
    let n = if currently_fullscreen { 0 } else { 2 };
    vec![
        vec![
            "eval".into(),
            format!("hl.dispatch(hl.dsp.focus({{ window = 'address:{addr}' }}))"),
        ],
        vec![
            "eval".into(),
            format!(
                "hl.dispatch(hl.dsp.window.fullscreen_state({{ window = 'address:{addr}', action = 'set', internal = {n}, client = {n} }}))"
            ),
        ],
    ]
}

/// Older (non-Lua) Hyprland: classic dispatchers.
pub fn legacy_cmds(addr: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "dispatch".into(),
            "focuswindow".into(),
            format!("address:{addr}"),
        ],
        vec!["dispatch".into(), "fullscreen".into(), "0".into()],
    ]
}

/// Run hyprctl commands; true only when every one answers `ok` (hyprctl exits 0 even on a rejected dispatch).
fn run_all(cmds: &[Vec<String>]) -> bool {
    cmds.iter().all(|a| {
        Command::new(hyprctl())
            .args(a)
            .output()
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "ok")
            .unwrap_or(false)
    })
}

/// Toggle using the Lua form, falling back to the classic one.
fn apply_toggle(addr: &str, fs: bool) -> bool {
    run_all(&toggle_cmds(addr, fs)) || run_all(&legacy_cmds(addr))
}

fn clients() -> Option<String> {
    let o = Command::new(hyprctl())
        .args(["clients", "-j"])
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Toggle fullscreen of the slop-tv window. Returns false when the window or hyprctl is not available.
pub fn toggle() -> bool {
    let Some((addr, fs)) = clients().and_then(|c| find_window(&c, Some(std::process::id()))) else {
        return false;
    };
    apply_toggle(&addr, fs)
}

/// Startup only (never re-applied afterwards): once the window exists and has had a moment to honour
/// `--start-fullscreen` itself, toggle it into fullscreen if it still is not. A user who leaves fullscreen
/// later is never pushed back in.
pub fn ensure_fullscreen_once() {
    if !available() {
        return;
    }
    std::thread::spawn(|| {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if clients()
                .and_then(|c| find_window(&c, Some(std::process::id())))
                .is_some()
            {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Some((addr, false)) =
            clients().and_then(|c| find_window(&c, Some(std::process::id())))
        {
            apply_toggle(&addr, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real class string reported by Hyprland on Wayland (fullscreen=2 at startup); pid 2947700.
    const REAL: &str = "chrome-__tmp_slop-tv-launch-2947700.html-Default";
    fn clients(extra: &str) -> String {
        format!(
            r#"[{{"address":"0xaa11","class":"kitty","fullscreen":0}},{extra}
            {{"address":"0x5581deadbeef","class":"{REAL}","fullscreen":2,"fullscreenClient":2,"title":"slopTV"}}]"#
        )
    }
    fn j() -> String {
        clients("")
    }

    #[test]
    fn finds_the_real_wayland_class_and_the_plain_class() {
        assert_eq!(
            find_window(&j(), None),
            Some(("0x5581deadbeef".into(), true))
        );
        assert_eq!(
            find_window(
                r#"[{"address":"0x1","class":"slop-tv","fullscreen":0}]"#,
                None
            ),
            Some(("0x1".into(), false))
        );
        assert_eq!(
            find_window(r#"[{"address":"0x1","class":"kitty"}]"#, None),
            None
        );
        assert_eq!(find_window("not json", None), None);
        assert_eq!(
            find_window(r#"[{"address":"0x1;rm","class":"slop-tv"}]"#, None),
            None,
            "address is validated"
        );
    }

    #[test]
    fn prefers_the_window_of_our_controller_pid() {
        let other = r#"{"address":"0x99","class":"chrome-__tmp_slop-tv-launch-111.html-Default","fullscreen":0},"#;
        let c = clients(other);
        assert_eq!(
            find_window(&c, Some(2947700)),
            Some(("0x5581deadbeef".into(), true))
        );
        assert_eq!(find_window(&c, Some(111)), Some(("0x99".into(), false)));
        // unknown pid: first slop-tv window
        assert_eq!(find_window(&c, Some(5)), Some(("0x99".into(), false)));
        // pid prefix must not match a longer pid
        assert_eq!(find_window(&c, Some(29477)), Some(("0x99".into(), false)));
    }

    #[test]
    fn toggle_uses_lua_eval_and_sets_the_state_explicitly() {
        let on = toggle_cmds("0xab", false);
        assert_eq!(on[0][0], "eval");
        assert_eq!(on[0][1], "hl.dispatch(hl.dsp.focus({ window = 'address:0xab' }))");
        assert!(on[1][1].contains("fullscreen_state") && on[1][1].contains("internal = 2, client = 2"));
        let off = toggle_cmds("0xab", true);
        assert!(off[1][1].contains("internal = 0, client = 0"));
        assert_eq!(legacy_cmds("0xab")[1], vec!["dispatch", "fullscreen", "0"]);
    }

    #[test]
    fn toggle_runs_hyprctl_shim_with_those_calls() {
        let dir = std::env::temp_dir().join(format!("slop-tv-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        let shim = dir.join("hyprctl");
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nif [ \"$1\" = clients ]; then cat <<'EOF'\n{}\nEOF\nelse echo ok; fi\n",
                log.display(),
                j()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("SLOPTV_HYPRCTL", &shim);
        assert!(toggle());
        std::env::remove_var("SLOPTV_HYPRCTL");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            vec![
                "clients -j",
                "eval hl.dispatch(hl.dsp.focus({ window = 'address:0x5581deadbeef' }))",
                "eval hl.dispatch(hl.dsp.window.fullscreen_state({ window = 'address:0x5581deadbeef', action = 'set', internal = 0, client = 0 }))"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
