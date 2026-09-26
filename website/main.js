/* ═══════════════════════════════════════════════════════════════════
   SITE CONFIG: change `owner` to your GitHub user or organisation.
   Every element with a data-gh attribute gets its href from this.
   ═══════════════════════════════════════════════════════════════════ */
const CONFIG = {
  owner: 'OWNER',
  repo: 'swoop',
};

(function () {
  'use strict';

  const root = document.documentElement;
  const darkQuery = window.matchMedia('(prefers-color-scheme: dark)');
  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

  /* ── GitHub links ─────────────────────────────────────────────── */
  const base = 'https://github.com/' + CONFIG.owner + '/' + CONFIG.repo;
  document.querySelectorAll('[data-gh]').forEach(function (a) {
    const key = a.getAttribute('data-gh');
    if (key === 'repo') a.href = base;
    else if (key === 'releases') a.href = base + '/releases/latest';
    else a.href = base + '/' + key;
  });

  /* ── Theme ────────────────────────────────────────────────────── */
  function storedTheme() {
    const t = root.getAttribute('data-theme');
    return t === 'light' || t === 'dark' ? t : null;
  }
  function effectiveTheme() {
    return storedTheme() || (darkQuery.matches ? 'dark' : 'light');
  }
  // <picture> sources marked data-dark follow the effective theme.
  function syncPictures() {
    const forced = storedTheme();
    const media = forced === 'dark' ? 'all' : forced === 'light' ? 'not all' : '(prefers-color-scheme: dark)';
    document.querySelectorAll('source[data-dark]').forEach(function (s) {
      if (s.getAttribute('media') !== media) s.setAttribute('media', media);
    });
  }
  const toggle = document.querySelector('.theme-toggle');
  function syncToggle() {
    const next = effectiveTheme() === 'dark' ? 'light' : 'dark';
    if (toggle) toggle.setAttribute('aria-label', 'Switch to ' + next + ' theme');
  }
  function applyTheme() {
    syncPictures();
    syncToggle();
  }
  if (toggle) {
    toggle.addEventListener('click', function () {
      const next = effectiveTheme() === 'dark' ? 'light' : 'dark';
      // Choosing the theme the system already uses clears the override.
      const systemTheme = darkQuery.matches ? 'dark' : 'light';
      if (next === systemTheme) {
        root.removeAttribute('data-theme');
        try { localStorage.removeItem('swoop-theme'); } catch (e) {}
      } else {
        root.setAttribute('data-theme', next);
        try { localStorage.setItem('swoop-theme', next); } catch (e) {}
      }
      applyTheme();
    });
  }
  if (darkQuery.addEventListener) darkQuery.addEventListener('change', applyTheme);

  /* ── Navigation ───────────────────────────────────────────────── */
  const nav = document.querySelector('.nav');
  const menuBtn = document.querySelector('.menu-toggle');
  function setMenu(open) {
    nav.classList.toggle('open', open);
    menuBtn.setAttribute('aria-expanded', String(open));
    menuBtn.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
  }
  if (menuBtn) {
    menuBtn.addEventListener('click', function () { setMenu(!nav.classList.contains('open')); });
    document.querySelectorAll('.nav-links a').forEach(function (a) {
      a.addEventListener('click', function () { setMenu(false); });
    });
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && nav.classList.contains('open')) { setMenu(false); menuBtn.focus(); }
    });
    document.addEventListener('click', function (e) {
      if (nav.classList.contains('open') && !nav.contains(e.target)) setMenu(false);
    });
  }
  function onScroll() { nav.classList.toggle('scrolled', window.scrollY > 12); }
  window.addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  /* ── Scroll reveal ────────────────────────────────────────────── */
  const reveals = document.querySelectorAll('.reveal');
  if (!('IntersectionObserver' in window) || reduceMotion.matches) {
    reveals.forEach(function (el) { el.classList.add('in'); });
  } else {
    const io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) { entry.target.classList.add('in'); io.unobserve(entry.target); }
      });
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.08 });
    reveals.forEach(function (el) { io.observe(el); });
  }

  /* ── Tabs (shared by Settings and the terminal) ──────────────── */
  function tabs(list, onSelect) {
    const buttons = Array.prototype.slice.call(list.querySelectorAll('[role="tab"]'));
    function select(btn, focus) {
      buttons.forEach(function (b) {
        const on = b === btn;
        b.setAttribute('aria-selected', String(on));
        b.tabIndex = on ? 0 : -1;
      });
      if (focus) btn.focus();
      onSelect(btn);
    }
    buttons.forEach(function (b, i) {
      b.addEventListener('click', function () { select(b, false); });
      b.addEventListener('keydown', function (e) {
        let j = null;
        if (e.key === 'ArrowRight') j = (i + 1) % buttons.length;
        else if (e.key === 'ArrowLeft') j = (i - 1 + buttons.length) % buttons.length;
        else if (e.key === 'Home') j = 0;
        else if (e.key === 'End') j = buttons.length - 1;
        if (j !== null) { e.preventDefault(); select(buttons[j], true); }
      });
    });
  }

  const settingsTabs = document.querySelector('.dive-settings [role="tablist"]');
  const settingsPane = document.getElementById('pane-settings');
  if (settingsTabs && settingsPane) {
    const src = settingsPane.querySelector('source');
    const img = settingsPane.querySelector('img');
    // Warm the cache so switching panes is instant.
    let warmed = false;
    function warm() {
      if (warmed) return;
      warmed = true;
      settingsTabs.querySelectorAll('[data-shot]').forEach(function (b) {
        const i = new Image();
        i.src = 'assets/screens/' + b.dataset.shot + '-' + effectiveTheme() + '.webp';
      });
    }
    settingsTabs.addEventListener('pointerenter', warm);
    settingsTabs.addEventListener('focusin', warm);
    tabs(settingsTabs, function (btn) {
      const shot = btn.dataset.shot;
      settingsPane.setAttribute('aria-labelledby', btn.id);
      settingsPane.classList.add('swapping');
      window.setTimeout(function () {
        src.srcset = 'assets/screens/' + shot + '-dark.webp';
        img.src = 'assets/screens/' + shot + '-light.webp';
        img.alt = btn.dataset.alt || '';
        const done = function () { settingsPane.classList.remove('swapping'); };
        if (img.complete) done(); else { img.onload = done; img.onerror = done; }
      }, reduceMotion.matches ? 0 : 140);
    });
  }

  const term = document.querySelector('[data-term]');
  if (term) {
    const list = term.querySelector('[role="tablist"]');
    tabs(list, function (btn) {
      term.querySelectorAll('.term-pane').forEach(function (p) {
        p.hidden = p.id !== btn.getAttribute('aria-controls');
      });
    });
  }

  /* ── Copy buttons ─────────────────────────────────────────────── */
  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text).catch(function () { return legacyCopy(text); });
    }
    return legacyCopy(text);
  }
  function legacyCopy(text) {
    return new Promise(function (resolve, reject) {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand('copy') ? resolve() : reject(new Error('copy failed')); }
      catch (err) { reject(err); }
      finally { document.body.removeChild(ta); }
    });
  }
  function textFor(btn) {
    const foot = btn.closest('.term-foot');
    if (foot) return foot.querySelector('code').textContent.trim();
    const pane = btn.closest('[data-term]').querySelector('.term-pane:not([hidden]) pre');
    return pane ? pane.textContent.trim() : '';
  }
  document.querySelectorAll('.copy-btn').forEach(function (btn) {
    const label = btn.querySelector('.copy-label');
    const original = label ? label.textContent : '';
    let timer;
    btn.addEventListener('click', function () {
      copyText(textFor(btn)).then(function () {
        btn.classList.add('copied');
        if (label) label.textContent = 'Copied';
        btn.setAttribute('aria-label', 'Copied to clipboard');
      }, function () {
        if (label) label.textContent = 'Press ⌘C';
      }).then(function () {
        clearTimeout(timer);
        timer = setTimeout(function () {
          btn.classList.remove('copied');
          if (label) label.textContent = original;
          btn.setAttribute('aria-label', btn.dataset.label || 'Copy commands');
        }, 1800);
      });
    });
  });

  /* ── Segmented download demo ──────────────────────────────────── */
  // One file split across eight connections. Each connection fills its own range; when one
  // finishes early it takes the back half of the largest range still in flight.
  const segTrack = document.querySelector('[data-segbar]');
  if (segTrack && !reduceMotion.matches && 'requestAnimationFrame' in window) {
    const stateEl = document.querySelector('[data-segbar-state]');
    const connsEl = document.querySelector('[data-segbar-conns]');
    const N = 8;
    const RATES = [1.3, 0.62, 1.05, 0.5, 1.2, 0.85, 0.95, 0.72];
    const BASE = 0.03; // file fraction per second at rate 1
    let conns = [];
    let fills = [];
    let heads = [];
    let phase = 'run';
    let phaseAt = 0;
    let last = 0;
    let visible = false;
    let raf = 0;

    function reset() {
      segTrack.textContent = '';
      segTrack.classList.add('live');
      segTrack.classList.remove('done', 'fade');
      conns = []; fills = []; heads = [];
      for (let i = 0; i < N; i++) {
        conns.push({ start: i / N, pos: i / N, end: (i + 1) / N, rate: RATES[i], busy: true });
      }
      phase = 'run';
      if (stateEl) { stateEl.textContent = '0%'; stateEl.classList.remove('ok'); }
      if (connsEl) connsEl.textContent = N + ' connections';
    }

    function el(cls) {
      const d = document.createElement('i');
      d.className = cls;
      segTrack.appendChild(d);
      return d;
    }

    function flash(at) {
      const f = el('flash');
      f.style.left = (at * 100) + '%';
      f.addEventListener('animationend', function () { f.remove(); });
    }

    function steal(c) {
      let victim = null;
      conns.forEach(function (o) {
        if (o.busy && o !== c && (!victim || o.end - o.pos > victim.end - victim.pos)) victim = o;
      });
      if (!victim || victim.end - victim.pos < 0.02) { c.busy = false; return; }
      const mid = victim.pos + (victim.end - victim.pos) / 2;
      conns.push({ start: mid, pos: mid, end: victim.end, rate: c.rate, busy: true, from: c });
      victim.end = mid;
      c.busy = false;
      c.handedOff = true;
      flash(mid);
    }

    function render() {
      while (fills.length < conns.length) fills.push(el('fill'));
      while (heads.length < conns.length) heads.push(el('head'));
      let done = 0;
      let active = 0;
      conns.forEach(function (c, i) {
        fills[i].style.left = (c.start * 100) + '%';
        fills[i].style.width = c.pos > c.start ? 'calc(' + ((c.pos - c.start) * 100) + '% + 1px)' : '0';
        heads[i].style.left = (c.pos * 100) + '%';
        heads[i].classList.toggle('idle', !c.busy);
        done += c.pos - c.start;
        if (c.busy) active++;
      });
      if (stateEl && phase === 'run') stateEl.textContent = Math.min(99, Math.floor(done * 100)) + '%';
      if (connsEl && phase === 'run') connsEl.textContent = active + (active === 1 ? ' connection' : ' connections') + (conns.length > N ? ' · work-stealing' : '');
      return done;
    }

    function tick(now) {
      raf = 0;
      if (!visible) return;
      const dt = Math.min(0.05, last ? (now - last) / 1000 : 0);
      last = now;
      if (phase === 'run') {
        const t = now / 1000;
        conns.slice().forEach(function (c, i) {
          if (!c.busy) return;
          const wobble = 1 + 0.18 * Math.sin(t * 1.7 + i * 1.3);
          c.pos = Math.min(c.end, c.pos + BASE * c.rate * wobble * dt);
          if (c.pos >= c.end - 1e-6) { c.pos = c.end; steal(c); }
        });
        const done = render();
        if (done >= 0.9999 || conns.every(function (c) { return !c.busy; })) {
          phase = 'done';
          phaseAt = now;
          segTrack.classList.add('done');
          if (stateEl) { stateEl.textContent = 'Verified ✓'; stateEl.classList.add('ok'); }
          if (connsEl) connsEl.textContent = 'Downloaded with ' + conns.length + ' segments';
        }
      } else if (phase === 'done' && now - phaseAt > 1800) {
        phase = 'fade';
        phaseAt = now;
        segTrack.classList.add('fade');
      } else if (phase === 'fade' && now - phaseAt > 550) {
        reset();
      }
      raf = requestAnimationFrame(tick);
    }

    reset();
    render();
    const seen = new IntersectionObserver(function (entries) {
      visible = entries[0].isIntersecting;
      if (visible && !raf) { last = 0; raf = requestAnimationFrame(tick); }
    });
    seen.observe(segTrack);
  }

  /* ── Misc ─────────────────────────────────────────────────────── */
  const year = document.getElementById('year');
  if (year) year.textContent = String(new Date().getFullYear());

  applyTheme();
})();
