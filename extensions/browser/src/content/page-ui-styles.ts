/**
 * Styles for the in-page UI, applied inside its closed shadow root (so nothing here leaks into
 * the page and page CSS can't leak in). Kept as a string so the content script stays a single
 * classic script. Same tokens as the popup: frosted glass, hairline edges, one system blue.
 */

export const PAGE_UI_CSS = `
:host {
  all: initial;
  --font: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, Roboto, "Noto Sans", sans-serif;
  --glass: rgba(250, 252, 255, 0.9);
  --glass-strong: rgba(255, 255, 255, 0.94);
  --hairline: rgba(11, 27, 51, 0.1);
  --hairline-strong: rgba(11, 27, 51, 0.16);
  --shadow: 0 18px 48px rgba(16, 36, 72, 0.22), 0 2px 8px rgba(16, 36, 72, 0.1);
  --text: #0e1726;
  --text-2: #566072;
  --accent: #007aff;
  --accent-fill: #0070f0;
  --accent-fill-hover: #0064dc;
  --accent-ink: #0060cc;
  --accent-soft: rgba(0, 122, 255, 0.12);
  --well: rgba(11, 27, 51, 0.06);
  --green: #34c759;
  --green-ink: #1f7a37;
  --orange: #ff9500;
  --orange-ink: #a24a00;
  --red: #ff3b30;
  --red-ink: #c4161c;
  --tint: #007aff;
  --focus: #007aff;
  --ease: cubic-bezier(0.2, 0.7, 0.2, 1);
}

@media (prefers-color-scheme: dark) {
  :host {
    --glass: rgba(30, 28, 56, 0.88);
    --glass-strong: rgba(40, 37, 72, 0.94);
    --hairline: rgba(255, 255, 255, 0.1);
    --hairline-strong: rgba(255, 255, 255, 0.16);
    --shadow: 0 22px 56px rgba(0, 0, 0, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3);
    --text: #f3f2fa;
    --text-2: #b3b1c9;
    --accent: #0a84ff;
    --accent-fill: #0a6ce6;
    --accent-fill-hover: #1a7af0;
    --accent-ink: #62b0ff;
    --accent-soft: rgba(10, 132, 255, 0.2);
    --well: rgba(255, 255, 255, 0.08);
    --green: #30d158;
    --green-ink: #4fdc76;
    --orange: #ff9f0a;
    --orange-ink: #ffb340;
    --red: #ff453a;
    --red-ink: #ff6b61;
    --focus: #4da3ff;
  }
}

*, *::before, *::after { box-sizing: border-box; }

.layer {
  font-family: var(--font);
  font-size: 13px;
  line-height: 1.35;
  color: var(--text);
  -webkit-font-smoothing: antialiased;
  text-align: left;
  letter-spacing: normal;
  text-transform: none;
  font-weight: 400;
  font-style: normal;
}

button, select { font: inherit; color: inherit; margin: 0; }
button { cursor: pointer; }
:focus { outline: none; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
[hidden] { display: none !important; }

.icon { width: 16px; height: 16px; flex: none; display: block; }

.glass {
  background: var(--glass);
  border: 1px solid var(--hairline);
  box-shadow: var(--shadow);
  -webkit-backdrop-filter: saturate(180%) blur(24px);
  backdrop-filter: saturate(180%) blur(24px);
}

.mark {
  display: inline-grid;
  place-items: center;
  flex: none;
  border-radius: 30%;
  background: linear-gradient(135deg, #409eff, #295ced);
  box-shadow: inset 0 0 0 0.75px rgba(255, 255, 255, 0.35), 0 3px 8px rgba(41, 92, 237, 0.3);
}
.mark svg { width: 74%; height: auto; margin-top: 4%; }

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 30px;
  padding: 0 12px;
  border-radius: 999px;
  border: 1px solid var(--hairline-strong);
  background: var(--glass-strong);
  font-size: 12.5px;
  font-weight: 600;
  white-space: nowrap;
  transition: background 160ms var(--ease), transform 120ms var(--ease);
}
.btn:hover { background: var(--accent-soft); }
.btn:active { transform: scale(0.97); }
.btn.primary { background: var(--accent-fill); border-color: transparent; color: #fff; }
.btn.primary:hover { background: var(--accent-fill-hover); }
.btn:disabled { opacity: 0.55; cursor: default; }
.btn .icon { width: 14px; height: 14px; }

.icon-btn {
  display: inline-grid;
  place-items: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: var(--text-2);
  transition: background 160ms var(--ease), color 160ms var(--ease);
}
.icon-btn:hover { background: var(--well); color: var(--text); }
.icon-btn .icon { width: 14px; height: 14px; }
.icon-btn.filled { background: var(--accent-fill); color: #fff; }
.icon-btn.filled:hover { background: var(--accent-fill-hover); color: #fff; }

/* ---- Prompts (after a captured download) ---- */

.stack {
  position: fixed;
  top: 16px;
  right: 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 388px;
  max-width: calc(100vw - 32px);
  pointer-events: none;
}

.prompt {
  position: relative;
  display: flex;
  gap: 12px;
  padding: 14px 12px 14px 14px;
  border-radius: 18px;
  overflow: hidden;
  pointer-events: auto;
  animation: prompt-in 320ms var(--ease);
}
.prompt.is-leaving { animation: prompt-out 200ms var(--ease) forwards; }

.prompt-badge {
  position: relative;
  flex: none;
  width: 38px;
  height: 38px;
}
.prompt-badge .mark { width: 38px; height: 38px; }
.prompt-badge .state {
  position: absolute;
  right: -4px;
  bottom: -4px;
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  color: #fff;
  background: var(--green);
  border: 2px solid var(--glass-strong);
}
.prompt-badge .state .icon { width: 10px; height: 10px; stroke-width: 3; }
.prompt.is-warning .prompt-badge .state { background: var(--orange); }
.prompt.is-error .prompt-badge .state { background: var(--red); }

.prompt-body { flex: 1; min-width: 0; }
.prompt-head { display: flex; align-items: flex-start; gap: 6px; }
.prompt-title { flex: 1; margin: 1px 0 0; font-size: 14px; font-weight: 700; letter-spacing: -0.01em; }
.prompt-head .icon-btn { margin: -3px -2px 0 0; }
.prompt-file {
  margin-top: 3px;
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.prompt-meta, .prompt-text { margin-top: 2px; font-size: 12px; color: var(--text-2); }
.prompt-text { margin-top: 4px; line-height: 1.4; }
.prompt-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 11px; }

.timer {
  position: absolute;
  left: 14px;
  right: 14px;
  bottom: 0;
  height: 2px;
  border-radius: 2px;
  background: var(--accent);
  opacity: 0.45;
  transform-origin: left center;
}

@keyframes prompt-in {
  from { opacity: 0; transform: translateY(-10px) scale(0.98); }
}
@keyframes prompt-out {
  to { opacity: 0; transform: translateY(-6px) scale(0.98); }
}
@keyframes timer { from { transform: scaleX(1); } to { transform: scaleX(0); } }

/* ---- Media button + panel ---- */

.media-btn {
  position: fixed;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 32px;
  padding: 0 12px 0 5px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 650;
  color: var(--text);
  pointer-events: auto;
  animation: fade-in 160ms var(--ease);
  transition: transform 120ms var(--ease), background 160ms var(--ease);
}
.media-btn:hover { background: var(--glass-strong); }
.media-btn:active { transform: scale(0.97); }
.media-btn .mark { width: 22px; height: 22px; }
.media-btn .chev { width: 12px; height: 12px; color: var(--text-2); }

.panel {
  position: fixed;
  width: 330px;
  max-width: calc(100vw - 24px);
  max-height: min(380px, calc(100vh - 24px));
  display: flex;
  flex-direction: column;
  border-radius: 18px;
  pointer-events: auto;
  animation: panel-in 200ms var(--ease);
}
.panel-head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 12px 10px 8px 14px;
}
.panel-head .mark { width: 24px; height: 24px; }
.panel-title { flex: 1; margin: 0; font-size: 14px; font-weight: 700; letter-spacing: -0.01em; }
.panel-list { list-style: none; margin: 0; padding: 0 6px 6px; overflow-y: auto; }
.panel-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 8px;
  border-radius: 12px;
}
.panel-row:hover, .panel-row:focus-within { background: var(--well); }
.tile {
  display: grid;
  place-items: center;
  flex: none;
  width: 32px;
  height: 32px;
  border-radius: 10px;
  color: var(--tint);
  background: color-mix(in srgb, var(--tint) 16%, transparent);
}
.tile .icon { width: 16px; height: 16px; }
.tile.t-audio { --tint: #e0527a; }
.tile.t-video, .tile.t-stream { --tint: #8e5be8; }
@media (prefers-color-scheme: dark) {
  .tile.t-audio { --tint: #ff7da0; }
  .tile.t-video, .tile.t-stream { --tint: #b08cff; }
}
.row-text { flex: 1; min-width: 0; }
.row-title { font-size: 12.5px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-meta { display: flex; align-items: center; gap: 6px; margin-top: 2px; font-size: 11.5px; color: var(--text-2); }
.row-meta span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.variant {
  appearance: none;
  -webkit-appearance: none;
  max-width: 110px;
  height: 22px;
  padding: 0 20px 0 8px;
  border-radius: 999px;
  border: 1px solid var(--hairline-strong);
  background:
    linear-gradient(45deg, transparent 50%, var(--text-2) 50%) calc(100% - 11px) 9px / 4px 4px no-repeat,
    linear-gradient(135deg, var(--text-2) 50%, transparent 50%) calc(100% - 7px) 9px / 4px 4px no-repeat,
    var(--glass-strong);
  font-size: 11px;
  font-weight: 600;
  flex: none;
}
.flag { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; font-weight: 650; color: var(--green-ink); }
.flag .icon { width: 13px; height: 13px; stroke-width: 2.6; }
.flag.is-error { color: var(--red-ink); }
.lock {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  height: 18px;
  padding: 0 7px;
  border-radius: 999px;
  background: var(--well);
  font-size: 10.5px;
  font-weight: 650;
  color: var(--text-2);
  flex: none;
}
.lock .icon { width: 10px; height: 10px; }
.panel-empty { padding: 4px 16px 18px; font-size: 12.5px; color: var(--text-2); line-height: 1.45; }
.panel-foot {
  padding: 8px 14px 11px;
  border-top: 1px solid var(--hairline);
  font-size: 11.5px;
  color: var(--text-2);
}

@keyframes fade-in { from { opacity: 0; } }
@keyframes panel-in { from { opacity: 0; transform: translateY(-6px) scale(0.98); } }

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation-duration: 0.001ms !important; transition-duration: 0.001ms !important; }
  .timer { display: none; }
}

@media (prefers-contrast: more) {
  :host { --hairline: rgba(127, 127, 127, 0.6); --hairline-strong: rgba(127, 127, 127, 0.8); }
}
`;
