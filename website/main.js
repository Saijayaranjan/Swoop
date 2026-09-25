/* ═══════════════════════════════════════════════════════════════════
   SITE CONFIG: change `owner` to your GitHub user or organisation.
   Every element with a data-gh attribute gets its href from this.
   ═══════════════════════════════════════════════════════════════════ */
const CONFIG = {
  owner: 'OWNER',
  repo: 'osprey',
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
    if (lightbox.isOpen()) lightbox.render();
  }
  if (toggle) {
    toggle.addEventListener('click', function () {
      const next = effectiveTheme() === 'dark' ? 'light' : 'dark';
      // Choosing the theme the system already uses clears the override.
      const systemTheme = darkQuery.matches ? 'dark' : 'light';
      if (next === systemTheme) {
        root.removeAttribute('data-theme');
        try { localStorage.removeItem('osprey-theme'); } catch (e) {}
      } else {
        root.setAttribute('data-theme', next);
        try { localStorage.setItem('osprey-theme', next); } catch (e) {}
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

  /* ── Gallery and lightbox ─────────────────────────────────────── */
  const SHOTS = [
    ['dashboard', 'Dashboard', 1280, 800],
    ['downloads', 'Downloads', 1280, 800],
    ['inspector', 'Download inspector', 1280, 800],
    ['queue', 'A queue', 1280, 800],
    ['torrents', 'Torrents', 1280, 800],
    ['scheduled', 'Scheduled', 1280, 800],
    ['history', 'History', 1280, 800],
    ['grabber', 'Site Grabber', 1280, 800],
    ['downloads-empty', 'Ready for a first download', 1280, 800],
    ['add', 'Add Download', 660, 720],
    ['menubar', 'Menu bar extra', 350, 572],
    ['about', 'About Osprey', 380, 548],
    ['settings-general', 'Settings: General', 1000, 728],
    ['settings-categories', 'Settings: Categories', 1000, 728],
    ['settings-bandwidth', 'Settings: Bandwidth', 1000, 728],
    ['settings-remote', 'Settings: Remote', 1000, 728],
  ];

  const grid = document.getElementById('gallery-grid');
  if (grid) {
    const frag = document.createDocumentFragment();
    SHOTS.forEach(function (s, i) {
      const li = document.createElement('li');
      li.className = 'reveal in';
      li.innerHTML =
        '<button class="tile" type="button" aria-label="View larger: ' + s[1] + '">' +
          '<span class="tile-media"><picture>' +
            '<source srcset="assets/thumbs/' + s[0] + '-dark.webp" media="(prefers-color-scheme: dark)" data-dark>' +
            '<img src="assets/thumbs/' + s[0] + '-light.webp" alt="" loading="lazy" decoding="async" width="' + Math.min(640, s[2]) + '" height="' + Math.round(Math.min(640, s[2]) * s[3] / s[2]) + '">' +
          '</picture></span>' +
          '<span class="tile-cap"><span>' + s[1] + '</span><svg class="ic" aria-hidden="true"><use href="#i-expand"/></svg></span>' +
        '</button>';
      li.querySelector('button').addEventListener('click', function () { lightbox.open(i, this); });
      frag.appendChild(li);
    });
    grid.appendChild(frag);
  }

  const lightbox = (function () {
    const el = document.getElementById('lightbox');
    const img = document.getElementById('lb-img');
    const cap = document.getElementById('lb-caption');
    const count = document.getElementById('lb-count');
    const closeBtn = el.querySelector('.lb-close');
    let index = 0;
    let opener = null;
    let touchX = null;

    function render() {
      const s = SHOTS[index];
      img.src = 'assets/screens/' + s[0] + '-' + effectiveTheme() + '.webp';
      img.width = s[2];
      img.height = s[3];
      img.alt = 'Osprey screenshot: ' + s[1];
      cap.textContent = s[1];
      count.textContent = (index + 1) + ' of ' + SHOTS.length;
      // Preload neighbours.
      [index - 1, index + 1].forEach(function (n) {
        const t = SHOTS[(n + SHOTS.length) % SHOTS.length];
        const p = new Image();
        p.src = 'assets/screens/' + t[0] + '-' + effectiveTheme() + '.webp';
      });
    }
    function go(delta) { index = (index + delta + SHOTS.length) % SHOTS.length; render(); }
    function open(i, from) {
      index = i;
      opener = from || document.activeElement;
      render();
      el.hidden = false;
      root.style.overflow = 'hidden';
      closeBtn.focus();
    }
    function close() {
      el.hidden = true;
      root.style.overflow = '';
      if (opener && opener.focus) opener.focus();
    }
    function isOpen() { return !el.hidden; }

    el.querySelectorAll('[data-close]').forEach(function (c) { c.addEventListener('click', close); });
    el.querySelector('.lb-prev').addEventListener('click', function () { go(-1); });
    el.querySelector('.lb-next').addEventListener('click', function () { go(1); });
    document.addEventListener('keydown', function (e) {
      if (!isOpen()) return;
      if (e.key === 'Escape') { e.preventDefault(); close(); }
      else if (e.key === 'ArrowRight') { e.preventDefault(); go(1); }
      else if (e.key === 'ArrowLeft') { e.preventDefault(); go(-1); }
      else if (e.key === 'Tab') {
        // Keep focus inside the dialog.
        const focusables = Array.prototype.slice.call(el.querySelectorAll('button'));
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    });
    el.addEventListener('touchstart', function (e) { touchX = e.touches[0].clientX; }, { passive: true });
    el.addEventListener('touchend', function (e) {
      if (touchX === null) return;
      const dx = e.changedTouches[0].clientX - touchX;
      if (Math.abs(dx) > 50) go(dx < 0 ? 1 : -1);
      touchX = null;
    });

    return { open: open, close: close, render: render, isOpen: isOpen };
  })();

  /* ── Misc ─────────────────────────────────────────────────────── */
  const year = document.getElementById('year');
  if (year) year.textContent = String(new Date().getFullYear());

  applyTheme();
})();
