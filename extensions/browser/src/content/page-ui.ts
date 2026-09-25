/**
 * Osprey's in-page UI, rendered in a closed shadow root on a single host element:
 *
 *  - Prompts (top-right) after the background captures a download: "Sent to Osprey" with
 *    "Use browser instead", "Kept in your browser" when Osprey is not running, and a sticky
 *    warning when the hand-off failed. Non-modal and never steal focus; Esc dismisses.
 *  - A media button over video/audio players (when enabled in settings): point at a player — or
 *    focus it and press Tab — to reveal it; it opens a small panel listing the page's media with
 *    one-click downloads. Esc closes the panel and returns focus.
 */

import type { CandidatePageMedia, PagePrompt } from '../shared/messages.ts';
import type { AddTaskResult, DetectedMedia, MediaKind, NewTaskRequest } from '../shared/types.ts';
import { formatBytes } from '../shared/format.ts';
import { hostnameOf } from '../shared/url-utils.ts';
import {
  BackgroundError,
  getPageMedia,
  getPageUiConfig,
  launchApp,
  promptAction,
  quickDownload,
} from '../shared/background-client.ts';
import { brandMark } from '../shared-ui/brand.ts';
import { h, tr } from '../shared-ui/dom.ts';
import { mediaKindTile } from '../shared-ui/file-kind.ts';
import { icon } from '../shared-ui/icons.ts';
import { PAGE_UI_CSS } from './page-ui-styles.ts';
import { collectAlternatePlaylists, collectVideoAudio, isProtectedMedia, mediaSourceUrl, nearbyTitle } from './scan.ts';

// --- Shadow host ---------------------------------------------------------------------------

let layer: HTMLElement | null = null;

function ensureLayer(): HTMLElement {
  if (layer?.isConnected) return layer;
  const host = document.createElement('osprey-layer');
  host.style.setProperty('all', 'initial', 'important');
  host.style.setProperty('position', 'fixed', 'important');
  host.style.setProperty('inset', '0 auto auto 0', 'important');
  host.style.setProperty('width', '0', 'important');
  host.style.setProperty('height', '0', 'important');
  host.style.setProperty('z-index', '2147483647', 'important');
  host.style.setProperty('pointer-events', 'none', 'important');
  const root = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  style.textContent = PAGE_UI_CSS;
  root.appendChild(style);
  try {
    // Constructable sheets aren't subject to a page's `style-src` CSP.
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(PAGE_UI_CSS);
    root.adoptedStyleSheets = [sheet];
  } catch {
    // The <style> element above covers engines that refuse this from a content script.
  }
  const inner = document.createElement('div');
  inner.className = 'layer';
  root.appendChild(inner);
  (document.body ?? document.documentElement).appendChild(host);
  layer = inner;
  return inner;
}

function onEscape(el: HTMLElement, fn: () => void): void {
  el.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      fn();
    }
  });
}

function reason(err: unknown): { text: string; notRunning: boolean } {
  const kind = err instanceof BackgroundError ? err.kind : undefined;
  if (kind === 'unavailable' || kind === 'unreachable') {
    return { text: tr('pageNotRunning', 'Osprey isn’t running'), notRunning: true };
  }
  return { text: err instanceof Error ? err.message : String(err), notRunning: false };
}

// --- Prompts -------------------------------------------------------------------------------

const AUTO_DISMISS_MS = 8000;
let stack: HTMLElement | null = null;

function ensureStack(): HTMLElement {
  const root = ensureLayer();
  if (stack?.isConnected) return stack;
  stack = h('div', { class: 'stack' });
  root.appendChild(stack);
  return stack;
}

export function showPrompt(prompt: PagePrompt): void {
  const container = ensureStack();
  const card = h('section', {
    class: `prompt glass${prompt.variant === 'not-running' ? ' is-warning' : prompt.variant === 'failed' ? ' is-error' : ''}`,
    attrs: {
      role: prompt.variant === 'failed' ? 'alert' : 'status',
      'aria-live': prompt.variant === 'failed' ? 'assertive' : 'polite',
      'aria-label': tr('extensionName', 'Osprey'),
    },
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  let timerBar: HTMLElement | null = null;
  const dismiss = (): void => {
    if (timer !== undefined) clearTimeout(timer);
    card.classList.add('is-leaving');
    setTimeout(() => card.remove(), 220);
  };
  const schedule = (ms: number): void => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(dismiss, ms);
    if (timerBar) {
      timerBar.style.animation = 'none';
      void timerBar.offsetWidth;
      timerBar.style.animation = `timer ${ms}ms linear forwards`;
    }
  };
  const pause = (): void => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    if (timerBar) timerBar.style.animationPlayState = 'paused';
  };
  let autoMs = prompt.variant === 'failed' ? 0 : AUTO_DISMISS_MS;
  let hovered = false;
  let focused = false;
  const resume = (): void => {
    if (autoMs > 0 && !hovered && !focused) schedule(autoMs);
  };

  const close = h('button', {
    class: 'icon-btn',
    attrs: { type: 'button', 'aria-label': tr('actionDismiss', 'Dismiss'), title: tr('actionDismiss', 'Dismiss') },
  }, [icon('close')]);
  close.addEventListener('click', dismiss);

  const title = h('p', { class: 'prompt-title' });
  const body = h('div', { class: 'prompt-body' });
  const actions = h('div', { class: 'prompt-actions' });
  const stateIcon = h('span', { class: 'state', attrs: { 'aria-hidden': 'true' } });
  const meta = [prompt.host, prompt.size ? formatBytes(prompt.size) : null].filter(Boolean).join(' · ');

  const setContent = (heading: string, text: string | null, buttons: HTMLElement[], stateGlyph: 'check' | 'warning' | 'close'): void => {
    // Keep keyboard users inside the card when the button they pressed is replaced.
    const root = card.getRootNode();
    const hadFocus = root instanceof ShadowRoot && root.activeElement !== null && card.contains(root.activeElement);
    title.textContent = heading;
    stateIcon.replaceChildren(icon(stateGlyph));
    const parts: HTMLElement[] = [
      h('div', { class: 'prompt-head' }, [title, close]),
      h('div', { class: 'prompt-file', text: prompt.fileName, title: prompt.fileName }),
    ];
    if (meta) parts.push(h('div', { class: 'prompt-meta', text: meta }));
    if (text) parts.push(h('div', { class: 'prompt-text', text }));
    if (buttons.length > 0) parts.push(actions);
    body.replaceChildren(...parts);
    actions.replaceChildren(...buttons);
    if (hadFocus) (buttons[0] ?? close).focus();
  };

  const button = (label: string, primary: boolean, fn: (btn: HTMLButtonElement) => void | Promise<void>, glyph?: 'open' | 'download' | 'power'): HTMLButtonElement => {
    const btn = h('button', { class: `btn${primary ? ' primary' : ''}`, attrs: { type: 'button' } }, [
      glyph ? icon(glyph) : null,
      h('span', { text: label }),
    ]);
    btn.addEventListener('click', () => void fn(btn));
    return btn;
  };

  const openApp = (): HTMLButtonElement =>
    button(tr('actionOpenApp', 'Open Osprey'), true, async () => {
      await launchApp().catch(() => {});
      dismiss();
    }, 'open');

  const alwaysBrowserState = (): void => {
    const host = prompt.host;
    setContent(
      tr('pageUsingBrowser', 'Downloading in your browser'),
      host ? tr('pageAlwaysQuestion', 'Always let the browser handle downloads from $1?', host) : null,
      host
        ? [
            button(tr('actionAlwaysForSite', 'Always for this site'), false, async (btn) => {
              btn.disabled = true;
              try {
                await promptAction(prompt.promptId, 'always-browser');
                setContent(tr('pageGotIt', 'Got it'), tr('pageAlwaysDone', 'Osprey will leave $1 to your browser. Change this in Osprey’s Sites settings.', host), [], 'check');
                autoMs = 4000;
                schedule(autoMs);
              } catch (err) {
                setContent(tr('pageSomethingWrong', 'Something went wrong'), reason(err).text, [], 'warning');
              }
            }),
          ]
        : [],
      'check',
    );
    autoMs = 7000;
    schedule(autoMs);
  };

  const useBrowser = async (btn: HTMLButtonElement): Promise<void> => {
    btn.disabled = true;
    try {
      await promptAction(prompt.promptId, 'use-browser');
      alwaysBrowserState();
    } catch (err) {
      btn.disabled = false;
      setContent(tr('pageSomethingWrong', 'Something went wrong'), reason(err).text, [], 'warning');
    }
  };

  switch (prompt.variant) {
    case 'sent':
      setContent(
        tr('pageSentTitle', 'Sent to Osprey'),
        null,
        [
          ...(prompt.canLaunch ? [openApp()] : []),
          button(tr('actionUseBrowser', 'Use browser instead'), !prompt.canLaunch, useBrowser),
        ],
        'check',
      );
      break;
    case 'not-running':
      setContent(
        tr('pageKeptTitle', 'Kept in your browser'),
        tr('pageKeptBody', 'Osprey isn’t running, so this download stayed in the browser.'),
        prompt.canLaunch
          ? [button(tr('actionLaunchApp', 'Launch Osprey'), true, async () => {
              await launchApp().catch(() => {});
              dismiss();
            }, 'power')]
          : [],
        'warning',
      );
      break;
    case 'failed':
      setContent(
        tr('pageFailedTitle', 'Couldn’t send to Osprey'),
        tr('pageFailedBody', 'The browser’s copy was already stopped. Download it in the browser instead?'),
        [button(tr('actionDownloadInBrowser', 'Download in browser'), true, useBrowser, 'download')],
        'close',
      );
      break;
  }

  card.append(h('div', { class: 'prompt-badge' }, [brandMark(38), stateIcon]), body);
  if (autoMs > 0) {
    timerBar = h('span', { class: 'timer', attrs: { 'aria-hidden': 'true' } });
    card.append(timerBar);
  }
  card.addEventListener('pointerenter', () => {
    hovered = true;
    pause();
  });
  card.addEventListener('pointerleave', () => {
    hovered = false;
    resume();
  });
  card.addEventListener('focusin', () => {
    focused = true;
    pause();
  });
  card.addEventListener('focusout', (event) => {
    if (event.relatedTarget instanceof Node && card.contains(event.relatedTarget)) return;
    focused = false;
    resume();
  });
  onEscape(card, dismiss);

  container.prepend(card);
  while (container.childElementCount > 3) container.lastElementChild?.remove();
  if (autoMs > 0) schedule(autoMs);
}

// --- Media button --------------------------------------------------------------------------

interface PanelItem {
  url: string;
  kind: MediaKind;
  title: string | null;
  size: number | null;
  height: number | null;
  notDownloadable: boolean;
  variants: DetectedMedia['variants'];
}

const MIN_VIDEO_W = 200;
const MIN_VIDEO_H = 110;
const HIDE_DELAY_MS = 900;

let networkCache: { at: number; items: DetectedMedia[] } | null = null;

async function networkMedia(): Promise<DetectedMedia[]> {
  if (networkCache && Date.now() - networkCache.at < 4000) return networkCache.items;
  try {
    const result = await getPageMedia();
    networkCache = { at: Date.now(), items: result.items.filter((i) => i.kind !== 'image') };
  } catch {
    networkCache = { at: Date.now(), items: [] };
  }
  return networkCache.items;
}

function eligible(el: HTMLMediaElement): boolean {
  const rect = el.getBoundingClientRect();
  if (el instanceof HTMLVideoElement) return rect.width >= MIN_VIDEO_W && rect.height >= MIN_VIDEO_H;
  return rect.width >= 180 && rect.height >= 24;
}

async function hasCandidates(el: HTMLMediaElement): Promise<boolean> {
  if (!isProtectedMedia(el)) return true;
  if (collectAlternatePlaylists().length > 0) return true;
  return (await networkMedia()).length > 0;
}

async function panelItems(focus: HTMLMediaElement | null): Promise<PanelItem[]> {
  const byUrl = new Map<string, PanelItem>();
  const add = (item: PanelItem): void => {
    const existing = byUrl.get(item.url);
    if (existing) {
      existing.title = existing.title ?? item.title;
      existing.notDownloadable = existing.notDownloadable || item.notDownloadable;
      return;
    }
    byUrl.set(item.url, item);
  };
  const fromPage = (m: CandidatePageMedia): PanelItem => ({
    url: m.url,
    kind: m.kind,
    title: m.title,
    size: null,
    height: m.height ?? null,
    notDownloadable: m.notDownloadable,
    variants: [],
  });
  if (focus) {
    const url = mediaSourceUrl(focus);
    if (url) {
      add({
        url,
        kind: focus instanceof HTMLVideoElement ? 'video' : 'audio',
        title: nearbyTitle(focus) ?? document.title,
        size: null,
        height: focus instanceof HTMLVideoElement ? focus.videoHeight || null : null,
        notDownloadable: isProtectedMedia(focus),
        variants: [],
      });
    }
  }
  for (const m of (await networkMedia())) {
    add({
      url: m.url,
      kind: m.kind,
      title: m.title ?? null,
      size: m.size ?? null,
      height: null,
      notDownloadable: m.protected ?? false,
      variants: m.variants,
    });
  }
  for (const m of collectAlternatePlaylists()) add(fromPage(m));
  for (const m of collectVideoAudio()) add(fromPage(m));
  // Protected entries are kept (so the panel can explain them) but sorted last.
  return [...byUrl.values()].sort((a, b) => Number(a.notDownloadable) - Number(b.notDownloadable));
}

function kindText(kind: MediaKind): string {
  switch (kind) {
    case 'video':
      return tr('mediaKindVideo', 'Video');
    case 'audio':
      return tr('mediaKindAudio', 'Audio');
    case 'hls_playlist':
      return tr('mediaKindHls', 'HLS');
    case 'dash_manifest':
      return tr('mediaKindDash', 'DASH');
    default:
      return tr('mediaKindUnknown', 'Media');
  }
}

function itemName(item: PanelItem): string {
  if (item.title) return item.title;
  try {
    const last = new URL(item.url).pathname.split('/').filter(Boolean).pop();
    if (last) return decodeURIComponent(last);
  } catch {
    // fall through
  }
  return item.url;
}

function requestFor(item: PanelItem, variantId: string | null): NewTaskRequest {
  const base: NewTaskRequest = {
    start: true,
    origin: 'browser',
    referer_page: location.href,
    name: item.title ?? undefined,
  };
  if (item.kind === 'hls_playlist') {
    return { ...base, hls_playlist_url: item.url, options: { media_variant: variantId ?? undefined } };
  }
  return { ...base, url: item.url };
}

export function initMediaButton(): void {
  // Settings are fetched lazily, the first time a player is pointed at or focused, so pages
  // without media never message (or wake) the background.
  let enabledPromise: Promise<boolean> | null = null;
  const enabled = (): Promise<boolean> => {
    enabledPromise ??= getPageUiConfig().then((c) => c.mediaButton, () => false);
    return enabledPromise;
  };

  let target: HTMLMediaElement | null = null;
  let button: HTMLButtonElement | null = null;
  let panel: HTMLElement | null = null;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let skipTabFrom: HTMLMediaElement | null = null;
  let frame = 0;

  const position = (): void => {
    if (!target || !button) return;
    const rect = target.getBoundingClientRect();
    if (rect.bottom < 0 || rect.top > innerHeight || rect.width === 0) {
      hide();
      return;
    }
    const btnWidth = button.offsetWidth || 120;
    button.style.top = `${Math.max(8, rect.top + 10)}px`;
    button.style.left = `${Math.min(innerWidth - btnWidth - 8, rect.right - btnWidth - 10)}px`;
    if (panel) {
      const bRect = button.getBoundingClientRect();
      const width = panel.offsetWidth || 330;
      panel.style.top = `${Math.min(bRect.bottom + 8, innerHeight - (panel.offsetHeight || 200) - 12)}px`;
      panel.style.left = `${Math.max(12, Math.min(innerWidth - width - 12, bRect.right - width))}px`;
    }
  };

  const closePanel = (restoreFocus: boolean): void => {
    panel?.remove();
    panel = null;
    button?.setAttribute('aria-expanded', 'false');
    if (restoreFocus) button?.focus();
  };

  function hide(): void {
    if (hideTimer !== undefined) clearTimeout(hideTimer);
    hideTimer = undefined;
    closePanel(false);
    button?.remove();
    button = null;
    target = null;
  }

  const scheduleHide = (): void => {
    if (panel) return;
    if (hideTimer !== undefined) clearTimeout(hideTimer);
    hideTimer = setTimeout(hide, HIDE_DELAY_MS);
  };
  const cancelHide = (): void => {
    if (hideTimer !== undefined) clearTimeout(hideTimer);
    hideTimer = undefined;
  };

  async function openPanel(): Promise<void> {
    if (!button) return;
    if (panel) {
      closePanel(true);
      return;
    }
    const root = ensureLayer();
    const list = h('ul', { class: 'panel-list' });
    const titleId = `osprey-panel-title`;
    const close = h('button', {
      class: 'icon-btn',
      attrs: { type: 'button', 'aria-label': tr('actionClose', 'Close'), title: tr('actionClose', 'Close') },
    }, [icon('close')]);
    close.addEventListener('click', () => closePanel(true));
    panel = h('div', {
      class: 'panel glass',
      attrs: { role: 'dialog', 'aria-labelledby': titleId },
    }, [
      h('div', { class: 'panel-head' }, [
        brandMark(24),
        h('p', { class: 'panel-title', text: tr('pageMediaTitle', 'Media on this page'), attrs: { id: titleId } }),
        close,
      ]),
      list,
    ]);
    onEscape(panel, () => closePanel(true));
    panel.addEventListener('pointerenter', cancelHide);
    root.appendChild(panel);
    button.setAttribute('aria-expanded', 'true');
    position();

    const items = await panelItems(target);
    if (!panel) return;
    const usable = items.filter((i) => !i.notDownloadable);
    if (items.length === 0 || usable.length === 0) {
      list.replaceWith(
        h('p', {
          class: 'panel-empty',
          text: tr('pageMediaProtected', 'This player streams protected media, which Osprey can’t save.'),
        }),
      );
    }
    for (const item of items.slice(0, 12)) list.append(renderItem(item));
    panel.append(h('div', { class: 'panel-foot', text: tr('pageMediaFoot', 'Downloads start in Osprey right away.') }));
    position();
    (panel.querySelector<HTMLElement>('.panel-list button, .panel-list select') ?? close).focus();
  }

  function renderItem(item: PanelItem): HTMLLIElement {
    const name = itemName(item);
    const tileKind = mediaKindTile(item.kind);
    const metaParts = [kindText(item.kind)];
    if (item.height) metaParts.push(`${item.height}p`);
    if (item.size) metaParts.push(formatBytes(item.size));
    const host = hostnameOf(item.url);
    if (host) metaParts.push(host);
    const meta = h('div', { class: 'row-meta' });
    let variant: HTMLSelectElement | null = null;
    if (item.notDownloadable) {
      meta.append(h('span', { class: 'lock' }, [icon('lock'), h('span', { text: tr('protectedShort', 'Protected') })]));
    } else if (item.variants && item.variants.length > 0) {
      variant = h('select', { class: 'variant', attrs: { 'aria-label': `${tr('quality', 'Quality')}: ${name}` } });
      for (const v of item.variants) {
        variant.append(h('option', { text: v.label, attrs: { value: v.id } }));
      }
      meta.append(variant);
    }
    meta.append(h('span', { text: metaParts.join(' · ') }));
    const li = h('li', { class: 'panel-row' }, [
      h('span', { class: `tile t-${tileKind.tint}`, attrs: { 'aria-hidden': 'true' } }, [icon(tileKind.icon)]),
      h('div', { class: 'row-text' }, [h('div', { class: 'row-title', text: name, title: item.url }), meta]),
    ]);
    if (!item.notDownloadable) {
      const slot = h('div');
      const dl = h('button', {
        class: 'icon-btn filled',
        attrs: {
          type: 'button',
          'aria-label': `${tr('actionDownloadWithOsprey', 'Download with Osprey')}: ${name}`,
          title: tr('actionDownloadWithOsprey', 'Download with Osprey'),
        },
      }, [icon('arrowDown')]);
      dl.addEventListener('click', async () => {
        dl.disabled = true;
        try {
          await quickDownload<AddTaskResult>(requestFor(item, variant?.value ?? null));
          slot.replaceChildren(h('span', { class: 'flag' }, [icon('check'), h('span', { text: tr('flagSent', 'Sent') })]));
          slot.setAttribute('role', 'status');
        } catch (err) {
          const why = reason(err);
          dl.disabled = false;
          const flag = h('span', { class: 'flag is-error', text: why.text });
          meta.replaceChildren(flag);
          if (why.notRunning) {
            const launch = h('button', { class: 'btn', attrs: { type: 'button' } }, [h('span', { text: tr('actionLaunch', 'Launch') })]);
            launch.addEventListener('click', () => void launchApp().catch(() => {}));
            meta.append(launch);
          }
        }
      });
      slot.append(dl);
      li.append(slot);
    }
    return li;
  }

  async function showFor(el: HTMLMediaElement): Promise<void> {
    cancelHide();
    if (target === el && button) {
      position();
      return;
    }
    if (panel) return;
    if (!eligible(el) || !(await enabled()) || !(await hasCandidates(el))) return;
    hide();
    target = el;
    const root = ensureLayer();
    button = h('button', {
      class: 'media-btn glass',
      attrs: {
        type: 'button',
        'aria-haspopup': 'dialog',
        'aria-expanded': 'false',
        'aria-label': tr('actionDownloadWithOsprey', 'Download with Osprey'),
      },
    }, [brandMark(22), h('span', { text: tr('actionDownload', 'Download') }), icon('chevronDown', 'icon chev')]);
    button.addEventListener('click', () => void openPanel());
    button.addEventListener('pointerenter', cancelHide);
    button.addEventListener('pointerleave', scheduleHide);
    button.addEventListener('keydown', (event) => {
      // Tab out of the button returns to the player, then continues through the page.
      if (event.key === 'Tab' && !panel && target) {
        event.preventDefault();
        skipTabFrom = target;
        const back = target;
        hide();
        back.focus();
      }
    });
    onEscape(button, () => {
      const back = target;
      hide();
      back?.focus();
    });
    root.appendChild(button);
    position();
  }

  // Live collections: checking their length is free, so pages without players cost nothing.
  const videos = document.getElementsByTagName('video');
  const audios = document.getElementsByTagName('audio');

  function mediaAt(x: number, y: number): HTMLMediaElement | null {
    if (videos.length === 0 && audios.length === 0) return null;
    const media = document.querySelectorAll<HTMLMediaElement>('video, audio');
    let count = 0;
    for (const el of media) {
      if (++count > 60) break;
      const r = el.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return el;
    }
    return null;
  }

  document.addEventListener(
    'pointermove',
    (event) => {
      if (frame) return;
      const { clientX, clientY } = event;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const el = mediaAt(clientX, clientY);
        if (el) void showFor(el);
        else if (button && !panel) scheduleHide();
      });
    },
    { passive: true, capture: true },
  );

  // Keyboard: focus a player, press Tab to reach the button.
  document.addEventListener(
    'keydown',
    (event) => {
      if (event.key !== 'Tab' || event.shiftKey) return;
      const active = document.activeElement;
      if (!(active instanceof HTMLMediaElement)) return;
      if (skipTabFrom === active) {
        skipTabFrom = null;
        return;
      }
      if (!button || target !== active) return;
      event.preventDefault();
      button.focus();
    },
    true,
  );
  document.addEventListener(
    'focusin',
    (event) => {
      if (event.target instanceof HTMLMediaElement) void showFor(event.target);
    },
    true,
  );

  document.addEventListener(
    'pointerdown',
    (event) => {
      if (!panel) return;
      const host = layer ? (layer.getRootNode() as ShadowRoot).host : null;
      if (host && event.composedPath().includes(host)) return;
      closePanel(false);
      scheduleHide();
    },
    true,
  );
  addEventListener('scroll', () => position(), { passive: true, capture: true });
  addEventListener('resize', () => position(), { passive: true });
}
