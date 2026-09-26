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

  /* ── Misc ─────────────────────────────────────────────────────── */
  const year = document.getElementById('year');
  if (year) year.textContent = String(new Date().getFullYear());

  applyTheme();
})();
