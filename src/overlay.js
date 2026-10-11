// slopTV overlay: injected by the controller through Chromium's remote debugging port
// (Page.addScriptToEvaluateOnNewDocument) into the top-level X video page. It talks to the
// controller only through the Runtime binding `slopTV` (page -> controller) and `window.__slopSet`
// (controller -> page). It never reads cookies, storage or X's network traffic.
(() => {
  if (window.top !== window || window.__slopTV) return;
  const h = location.hostname;
  if (!(location.protocol === 'about:' || (location.protocol === 'data:' && document.title === 'slopTV') || h === 'x.com' || h.endsWith('.x.com') || h === 'twitter.com' || h.endsWith('.twitter.com'))) return;
  window.__slopTV = true;

  // SYNC-BEGIN (pure logic, unit-tested with node)
  // Follows the REAL <video>: currentTime, duration, paused, ended. Fallback to the feed duration (+grace) only
  // while no usable <video> duration has ever been seen for this clip.
  function makeFollower(graceMs) {
    return {
      feed: 0, auto: false, t0: 0, lastNow: 0, fired: false, lastT: 0, sawVideo: false, frac: 0, src: 'none',
      start(feedSecs, auto, now) {
        this.feed = feedSecs > 0 ? feedSecs : 0; this.auto = !!auto; this.t0 = now; this.lastNow = now;
        this.fired = false; this.lastT = 0; this.sawVideo = false; this.frac = 0; this.src = 'none';
      },
      // s = {t, d, paused, ended} sampled from the real video, or null when there is none.
      // Returns true exactly once per clip, when auto-advance should happen.
      step(s, now) {
        let due = false;
        const dt = now - this.lastNow; this.lastNow = now;
        if (s && isFinite(s.d) && s.d > 0) {
          this.sawVideo = true; this.src = 'video';
          this.frac = Math.min(1, Math.max(0, s.t / s.d));
          const looped = this.lastT > s.d - 0.6 && s.t < this.lastT - 1;
          this.lastT = s.t;
          due = !!s.ended || s.t >= s.d - 0.05 || looped;
        } else if (!this.sawVideo && this.feed > 0) {
          this.src = 'feed';
          if (s && s.paused) this.t0 += dt;           // user paused a video whose duration we cannot read
          const total = this.feed * 1000 + graceMs;
          const el = now - this.t0;
          this.frac = Math.min(1, el / total);
          due = el >= total;
        }
        if (due && this.auto && !this.fired) { this.fired = true; return true; }
        return false;
      },
    };
  }
  // SYNC-END

  const GRACE = 1000;
  const F = makeFollower(GRACE);
  let st = null, native = false, P = false, uiUntil = 0, idleT = Date.now(), dbgOn = false, toastT = 0, lastId = null;
  const send = cmd => { try { window.slopTV(JSON.stringify({cmd})); return true; } catch (e) { return false; } };

  // ---- UI (shadow DOM, so X's CSS cannot touch it and ours cannot touch X) ----
  const host = document.createElement('div');
  host.id = 'slop-tv-overlay';
  host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none';
  const root = host.attachShadow({mode: 'open'});
  root.innerHTML = `<style>
  :host{all:initial}
  *{box-sizing:border-box}
  .w{position:fixed;inset:0;color:#d8d8d8;font:13px/1.35 system-ui,sans-serif;--fg:#d8d8d8;--dim:#8a8a8a;--accent:#78e8c3}
  #attr{position:fixed;left:14px;top:14px;max-width:min(60vw,420px);padding:5px 10px;border-radius:999px;background:rgba(18,18,20,.72);color:var(--dim);
    font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;opacity:0;transition:opacity .25s;pointer-events:none;backdrop-filter:blur(6px)}
  .ui #attr{opacity:1;pointer-events:auto}
  #attr b{color:var(--fg);font-weight:600}#attr a{color:var(--accent);text-decoration:none;margin-left:6px;cursor:pointer}#attr a:hover{text-decoration:underline}
  #prog{position:fixed;left:0;bottom:0;height:2px;width:0;background:var(--accent);opacity:.7;pointer-events:none}
  .card{position:fixed;bottom:64px;width:min(270px,28vw);display:flex;gap:9px;align-items:center;padding:7px;border:0;border-radius:10px;
    background:rgba(18,18,20,.72);color:var(--fg);font:inherit;text-align:left;cursor:pointer;
    backdrop-filter:blur(6px);opacity:0;transform:translateY(6px);pointer-events:none;transition:opacity .25s,transform .25s,background .15s}
  .ui .card:not(.empty){opacity:1;transform:none;pointer-events:auto}
  .ui .card:not(.empty):hover{background:rgba(34,34,38,.88)}
  #cp{left:14px}#cn{right:14px;flex-direction:row-reverse;text-align:right}
  .card img{width:72px;height:41px;object-fit:cover;border-radius:6px;background:#1c1c1e;flex:none}
  .card .t{min-width:0;font-size:12px}
  .card b{display:block;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .card .c{display:block;color:var(--dim);display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
  .card em{display:block;font-style:normal;color:var(--accent);font-size:11px}
  #dbg{position:fixed;right:10px;top:10px;display:none;padding:4px 8px;border-radius:6px;background:rgba(0,0,0,.7);color:#9f9;font:11px/1.4 ui-monospace,monospace;white-space:pre;pointer-events:none}
  #toast{position:fixed;left:50%;bottom:16px;transform:translateX(-50%);padding:5px 12px;border-radius:999px;background:rgba(18,18,20,.8);
    color:var(--fg);font-size:12px;opacity:0;transition:opacity .25s;pointer-events:none;white-space:nowrap}
  #toast.on{opacity:1}
  #notice{position:fixed;inset:0;display:none;align-items:center;justify-content:center;background:#000;pointer-events:auto}
  #notice div{max-width:480px;padding:16px 18px;border:1px solid #2a2a2e;border-radius:12px;color:var(--dim);font-size:12.5px;line-height:1.5}
  #notice button{margin-top:10px;margin-right:6px;background:#1c1c1e;color:var(--fg);border:1px solid #333;border-radius:8px;padding:5px 12px;cursor:pointer;font:inherit}
  #notice button:hover{border-color:var(--accent)}
  </style><div class="w" id="w">
  <div id="attr"><span>&#x1D54F;</span> <b id="ah"></b> <span id="an"></span><a id="av">View on X</a></div>
  <div id="prog"></div>
  <button class="card empty" id="cp"><img alt=""><span class="t"><b></b><span class="c"></span></span></button>
  <button class="card empty" id="cn"><img alt=""><span class="t"><b></b><span class="c"></span><em></em></span></button>
  <div id="toast"></div><div id="dbg"></div>
  <div id="notice"><div>This plays each post in X's own video player page (x.com/i/videos/tweet/...), an unofficial, undocumented X page that may change or be blocked. slopTV opens it as the top-level page of a Chromium app window and runs its own overlay script inside that page through Chromium's local remote debugging port (127.0.0.1 only), to draw the prev/next cards, handle keys and follow the real video's progress. This may be against X's Terms of Service; use at your own risk. X's scripts and media load and X may set cookies in slopTV's private browser profile (not your normal one).<br>
  <button id="ok">OK</button><button id="no">Quit</button></div></div></div>`;
  const $ = id => root.getElementById(id);
  const W = $('w');
  // never append to `document` itself: at document start there may be no <html> yet, and the host would become the root
  const mount = () => { if (!host.isConnected && document.documentElement) document.documentElement.appendChild(host); };
  mount(); new MutationObserver(mount).observe(document, {childList: true}); document.addEventListener('DOMContentLoaded', mount);
  setInterval(mount, 1000);

  function toast(msg) { const t = $('toast'); t.textContent = msg; t.classList.add('on'); clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove('on'), 1800); }
  function showUi(ms) { uiUntil = Math.max(uiUntil, Date.now() + ms); W.classList.add('ui'); document.documentElement.style.cursor = ''; }
  function card(el, c) {
    el.classList.toggle('empty', !c);
    if (!c) return;
    const img = el.querySelector('img');
    if (c.poster) { if (img.dataset.src !== c.poster) { img.dataset.src = c.poster; img.referrerPolicy = 'no-referrer'; img.src = c.poster; } img.style.visibility = 'visible'; }
    else { img.removeAttribute('src'); delete img.dataset.src; img.style.visibility = 'hidden'; }
    el.querySelector('b').textContent = '@' + c.creator;
    el.querySelector('.c').textContent = c.caption;
    const e = el.querySelector('em'); if (e) e.textContent = c.wraps ? '\u21bb loops to start' : '';
  }
  function show(s) {
    const changed = !st || st.id !== s.id;
    st = s;
    native = !!s.hypr_native;
    $('notice').style.display = s.notice ? 'flex' : 'none';
    $('ah').textContent = '@' + s.creator; $('an').textContent = s.name ? '\u00b7 ' + s.name + ' \u00b7' : '\u00b7';
    $('av').style.display = s.post_url ? '' : 'none';
    card($('cp'), s.prev); card($('cn'), s.next);
    if (changed) { lastId = s.id; F.start(s.notice ? 0 : s.duration_secs, s.auto, Date.now()); if (!s.notice) showUi(3500); }
    else F.auto = !!s.auto;
  }
  window.__slopSet = show;

  // ---- the real <video> ----
  function vid() {
    const vs = [...document.querySelectorAll('video')];
    return vs.find(v => v.readyState > 0 && v.videoWidth > 0) || vs[0] || null;
  }
  function sample() {
    const v = vid();
    return v ? {t: v.currentTime || 0, d: v.duration, paused: v.paused, ended: v.ended} : null;
  }
  function tick() {
    const now = Date.now();
    if (W.classList.contains('ui') && now > uiUntil) W.classList.remove('ui');
    const s = sample();
    P = !!(s && s.paused);
    if (!st || st.notice) { $('prog').style.width = '0'; return; }
    const due = F.step(s, now);
    $('prog').style.width = (st.auto && (F.sawVideo || F.feed) ? F.frac * 100 : 0) + '%';
    if (due) send('next');
    dbgDraw(s, now);
  }
  function dbgDraw(s, now) {
    if (!dbgOn) return;
    const v = vid();
    $('dbg').textContent = ['id ' + (st && st.id), 'follow ' + F.src + (F.sawVideo ? '' : ' (feed ' + F.feed + 's)'),
      s ? 't ' + s.t.toFixed(1) + ' / d ' + (isFinite(s.d) ? s.d.toFixed(1) : '?') + ' paused ' + s.paused + ' ended ' + s.ended : 'no <video>',
      v ? 'video ' + v.videoWidth + 'x' + v.videoHeight : '', 'auto ' + (st && st.auto)].join('\n');
  }
  setInterval(tick, 250);
  // ended does not bubble; capture it on document
  document.addEventListener('ended', () => { if (st && !st.notice) tick(); }, true);

  // ---- controls ----
  function setVideoPaused(on) {
    const v = vid(); if (!v) return;
    if (on) v.pause(); else v.play().catch(() => {});
  }
  function webFullscreen() { if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen().catch(() => {}); }
  function fullscreen() { if (native) send('fullscreen'); else webFullscreen(); }
  function cmd(c, msg) { if (msg) toast(msg); send(c); }
  addEventListener('keydown', e => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const tg = e.target; if (tg && (tg.tagName === 'INPUT' || tg.tagName === 'TEXTAREA' || tg.isContentEditable)) return;
    if (st && st.notice) return;
    const k = e.key; idleT = Date.now();
    let used = true;
    if (k === 'l' || k === 'n' || k === 'ArrowRight') send('next');
    else if (k === 'h' || k === 'ArrowLeft') send('prev');
    else if (k === 's') cmd('shuffle', st && st.shuffle ? 'shuffle off' : 'shuffle on');
    else if (k === 'a') cmd('auto', st && st.auto ? 'auto-advance off' : 'auto-advance on');
    else if (k === 'p' || k === ' ') { const v = vid(); setVideoPaused(!(v && v.paused)); showUi(1500); }
    else if (k === 'f') cmd('like', 'opening like page on X');
    else if (k === 'o') cmd('open', 'opening post on X');
    else if (k === 'd') { dbgOn = !dbgOn; $('dbg').style.display = dbgOn ? 'block' : 'none'; dbgDraw(sample(), Date.now()); }
    else if (k === 'w' || k === 'F11') fullscreen();
    else if (k === 'q') send('quit');
    else used = false;
    if (used) { e.preventDefault(); e.stopImmediatePropagation(); }
  }, true);
  addEventListener('mousemove', () => { idleT = Date.now(); showUi(2200); }, true);
  $('cp').onclick = () => send('prev'); $('cn').onclick = () => send('next');
  $('av').onclick = e => { e.preventDefault(); cmd('open', 'opening post on X'); };
  $('ok').onclick = () => send('ack'); $('no').onclick = () => send('quit');
  addEventListener('dblclick', e => { if (e.clientY < 80) fullscreen(); }, true);

  // ask the controller for state; retry until the binding exists
  let hello = 0;
  (function ask() { if (st || hello++ > 40) return; if (!send('hello')) setTimeout(ask, 100); else setTimeout(() => { if (!st) ask(); }, 500); })();
})();
