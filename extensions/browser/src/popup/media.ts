/**
 * "On this page" section: merges network-level detections (`media-detector.ts`, via the
 * background) with the content script's DOM scan (`<video>`/`<audio>`/large `<img>`/
 * `link rel=alternate`), deduped by URL. Shown only when something was found. Download, Queue
 * and Copy URL call the real API; HLS playlists offer their variants as a quality picker.
 */

import browser from 'webextension-polyfill';
import { formatBytesCompact } from '../shared/format.ts';
import { hostnameOf } from '../shared/url-utils.ts';
import { enrichHls, getDetectedMedia, quickDownload } from '../shared/background-client.ts';
import type { CandidatePageMedia, ScanResultMessage } from '../shared/messages.ts';
import type { AddTaskResult, DetectedMedia, MediaKind, NewTaskRequest } from '../shared/types.ts';
import { capsule, fileTile, h, iconButton, tr } from '../shared-ui/dom.ts';
import { mediaKindTile } from '../shared-ui/file-kind.ts';
import { icon } from '../shared-ui/icons.ts';
import { errorMessage, type PopupContext } from './context.ts';

interface UnifiedItem {
  url: string;
  kind: MediaKind;
  title: string | null;
  size: number | null;
  notDownloadable: boolean;
  variants: DetectedMedia['variants'];
  pageUrl: string | null;
  width: number | null;
  height: number | null;
}

const COLLAPSED_COUNT = 2;

export function kindLabel(kind: MediaKind): string {
  switch (kind) {
    case 'video':
      return tr('mediaKindVideo', 'Video');
    case 'audio':
      return tr('mediaKindAudio', 'Audio');
    case 'image':
      return tr('mediaKindImage', 'Image');
    case 'hls_playlist':
      return tr('mediaKindHls', 'HLS');
    case 'dash_manifest':
      return tr('mediaKindDash', 'DASH');
    default:
      return tr('mediaKindUnknown', 'Media');
  }
}

function mergeItems(network: DetectedMedia[], page: CandidatePageMedia[], pageUrl: string): UnifiedItem[] {
  const byUrl = new Map<string, UnifiedItem>();
  for (const item of network) {
    byUrl.set(item.url, {
      url: item.url,
      kind: item.kind,
      title: item.title ?? null,
      size: item.size ?? null,
      notDownloadable: item.protected ?? false,
      variants: item.variants,
      pageUrl: item.page_url ?? pageUrl,
      width: null,
      height: null,
    });
  }
  for (const item of page) {
    const existing = byUrl.get(item.url);
    if (existing) {
      existing.title = existing.title ?? item.title;
      existing.notDownloadable = existing.notDownloadable || item.notDownloadable;
      existing.width = existing.width ?? item.width ?? null;
      existing.height = existing.height ?? item.height ?? null;
      continue;
    }
    byUrl.set(item.url, {
      url: item.url,
      kind: item.kind,
      title: item.title,
      size: null,
      notDownloadable: item.notDownloadable,
      variants: [],
      pageUrl,
      width: item.width ?? null,
      height: item.height ?? null,
    });
  }
  // Downloadable video/audio/streams first, then images, protected last.
  const rank = (item: UnifiedItem): number =>
    (item.notDownloadable ? 10 : 0) + (item.kind === 'image' ? 5 : 0) + (item.kind === 'unknown' ? 3 : 0);
  return [...byUrl.values()].sort((a, b) => rank(a) - rank(b));
}

function buildRequest(item: UnifiedItem, start: boolean, variantId: string | null): NewTaskRequest {
  const base: NewTaskRequest = {
    start,
    origin: 'browser',
    referer_page: item.pageUrl ?? undefined,
    name: item.title ?? undefined,
  };
  if (item.kind === 'hls_playlist') {
    return { ...base, hls_playlist_url: item.url, options: { media_variant: variantId ?? undefined } };
  }
  return { ...base, url: item.url };
}

function displayName(item: UnifiedItem): string {
  if (item.title) return item.title;
  try {
    const last = new URL(item.url).pathname.split('/').filter(Boolean).pop();
    if (last) return decodeURIComponent(last);
  } catch {
    // fall through
  }
  return item.url;
}

export interface MediaHandle {
  refresh(): Promise<void>;
}

export function initMedia(ctx: PopupContext): MediaHandle {
  const section = document.getElementById('media-section') as HTMLElement;
  const list = document.getElementById('media-list') as HTMLUListElement;
  const count = document.getElementById('media-count') as HTMLElement;
  const more = document.getElementById('media-more') as HTMLButtonElement;
  const selectedVariant = new Map<string, string>();
  let expanded = false;
  let items: UnifiedItem[] = [];

  more.addEventListener('click', () => {
    expanded = !expanded;
    render();
  });

  async function send(item: UnifiedItem, start: boolean, row: HTMLElement, actions: HTMLElement): Promise<void> {
    const variantId = selectedVariant.get(item.url) ?? null;
    try {
      await quickDownload<AddTaskResult>(buildRequest(item, start, variantId));
      ctx.toast(start ? tr('toastDownloadStarted', 'Download started') : tr('toastQueued', 'Queued'));
      const flag = h('span', { class: 'sent-flag' }, [
        icon('check'),
        h('span', { text: start ? tr('flagSent', 'Sent') : tr('toastQueued', 'Queued') }),
      ]);
      const previous = [...actions.childNodes];
      actions.replaceChildren(flag);
      setTimeout(() => {
        if (row.isConnected) actions.replaceChildren(...previous);
      }, 2400);
    } catch (err) {
      ctx.toast(errorMessage(err), 'error');
    }
  }

  function renderRow(item: UnifiedItem): HTMLLIElement {
    const name = displayName(item);
    const title = h('span', { class: 'row-title', text: name, title: item.url });
    const actions = h('div', { class: 'row-actions' });
    const li = h('li', { class: 'row' }, [
      fileTile(mediaKindTile(item.kind)),
      h('div', { class: 'row-body' }, [h('div', { class: 'row-top' }, [title, actions])]),
    ]);
    const body = li.querySelector('.row-body') as HTMLElement;

    const variants = item.variants ?? [];
    const host = hostnameOf(item.url);
    const metaText = h('span', { class: 'grow' });
    const paintMeta = (): void => {
      const parts: string[] = [kindLabel(item.kind)];
      if (item.height) parts.push(`${item.height}p`);
      const chosen = variants.find((v) => v.id === selectedVariant.get(item.url));
      const size = chosen?.estimated_size ?? item.size;
      if (size) parts.push(formatBytesCompact(size));
      if (host) parts.push(host);
      metaText.textContent = parts.join(' · ');
    };
    const meta = h('div', { class: 'row-meta' });

    if (item.notDownloadable) {
      meta.append(capsule(tr('notDownloadable', 'Not downloadable'), 'grey', 'lock'));
    }
    if (!item.notDownloadable && variants.length > 0) {
      const select = h('select', { class: 'variant', attrs: { 'aria-label': `${tr('quality', 'Quality')}: ${name}` } });
      for (const v of variants) {
        select.append(h('option', { text: v.label, attrs: { value: v.id } }));
      }
      if (!selectedVariant.has(item.url)) selectedVariant.set(item.url, variants[0]?.id ?? '');
      select.value = selectedVariant.get(item.url) ?? '';
      select.addEventListener('change', () => {
        selectedVariant.set(item.url, select.value);
        paintMeta();
      });
      meta.append(select);
    }
    paintMeta();
    meta.append(metaText);
    body.append(meta);

    if (!item.notDownloadable) {
      const download = iconButton('arrowDown', `${tr('actionDownload', 'Download')}: ${name}`, () =>
        send(item, true, li, actions),
      'icon-btn filled');
      download.title = tr('actionDownload', 'Download');
      const queue = iconButton('queue', `${tr('actionQueue', 'Queue')}: ${name}`, () => send(item, false, li, actions));
      queue.title = tr('actionQueue', 'Queue');
      actions.append(queue, download);
    }
    const copy = iconButton('copy', `${tr('actionCopyUrl', 'Copy URL')}: ${name}`, async () => {
      try {
        await navigator.clipboard.writeText(item.url);
        ctx.toast(tr('toastCopied', 'Copied'));
      } catch (err) {
        ctx.toast(errorMessage(err), 'error');
      }
    });
    copy.title = tr('actionCopyUrl', 'Copy URL');
    actions.prepend(copy);
    return li;
  }

  function render(): void {
    if (items.length === 0) {
      section.hidden = true;
      return;
    }
    section.hidden = false;
    count.textContent = String(items.length);
    const visible = expanded ? items : items.slice(0, COLLAPSED_COUNT);
    list.replaceChildren(...visible.map(renderRow));
    const hiddenCount = items.length - COLLAPSED_COUNT;
    more.hidden = hiddenCount <= 0;
    more.textContent = expanded ? tr('showLess', 'Show less') : tr('showAll', 'Show all');
    more.setAttribute('aria-expanded', String(expanded));
  }

  async function refresh(): Promise<void> {
    const tabId = ctx.tabId;
    if (tabId === null) {
      items = [];
      render();
      return;
    }
    let networkItems: DetectedMedia[] = [];
    let pageResult: ScanResultMessage | undefined;
    try {
      const [detected] = await Promise.all([getDetectedMedia(tabId), enrichHls(tabId, null).catch(() => null)]);
      networkItems = detected.items;
    } catch {
      networkItems = [];
    }
    try {
      pageResult = (await browser.tabs.sendMessage(tabId, { type: 'scan' })) as ScanResultMessage | undefined;
    } catch {
      pageResult = undefined;
    }
    // Re-read once more in case enrichHls resolved variants after the first read.
    try {
      networkItems = (await getDetectedMedia(tabId)).items;
    } catch {
      // keep the first read
    }
    items = mergeItems(networkItems, pageResult?.media ?? [], pageResult?.pageUrl ?? '');
    render();
  }

  return { refresh };
}
