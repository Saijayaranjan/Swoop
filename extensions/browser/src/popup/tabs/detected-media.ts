/**
 * "Detected media" tab: merges network-level detections (`media-detector.ts`, via the background)
 * with the content script's DOM scan (`<video>`/`<audio>`/`<img>` large/`link rel=alternate`),
 * deduped by URL. Every action (Download, Queue, Copy URL, Open source) calls the real API.
 */

import browser from 'webextension-polyfill';
import { t } from '../../shared/i18n.ts';
import { formatBytes } from '../../shared/format.ts';
import { hostnameOf } from '../../shared/url-utils.ts';
import { enrichHls, getDetectedMedia, quickDownload } from '../../shared/background-client.ts';
import type { CandidatePageMedia, ScanResultMessage } from '../../shared/messages.ts';
import type { AddTaskResult, DetectedMedia, MediaKind, NewTaskRequest } from '../../shared/types.ts';
import { renderEmptyState, renderItemRow, renderList } from '../render-utils.ts';

interface UnifiedItem {
  url: string;
  kind: MediaKind;
  title: string | null;
  size: number | null;
  notDownloadable: boolean;
  variants: DetectedMedia['variants'];
  pageUrl: string | null;
}

function kindLabel(kind: MediaKind): string {
  switch (kind) {
    case 'video':
      return t('mediaKindVideo') || 'Video';
    case 'audio':
      return t('mediaKindAudio') || 'Audio';
    case 'image':
      return t('mediaKindImage') || 'Image';
    case 'hls_playlist':
      return t('mediaKindHls') || 'HLS';
    case 'dash_manifest':
      return t('mediaKindDash') || 'DASH';
    default:
      return t('mediaKindUnknown') || 'Media';
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
    });
  }
  for (const item of page) {
    const existing = byUrl.get(item.url);
    if (existing) {
      existing.title = existing.title ?? item.title;
      existing.notDownloadable = existing.notDownloadable || item.notDownloadable;
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
    });
  }
  return [...byUrl.values()];
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

export interface DetectedMediaTabHandle {
  refresh(): Promise<void>;
}

export function initDetectedMediaTab(
  container: HTMLElement,
  tabId: number | null,
  showToast: (message: string) => void,
): DetectedMediaTabHandle {
  const selectedVariant = new Map<string, string>();

  async function refresh(): Promise<void> {
    if (tabId === null) {
      renderEmptyState(container, t('noActiveTab') || 'No active tab.');
      return;
    }
    renderEmptyState(container, t('loading') || 'Loading…');

    let networkItems: DetectedMedia[] = [];
    let pageResult: ScanResultMessage | undefined;
    try {
      const [detected] = await Promise.all([
        getDetectedMedia(tabId),
        enrichHls(tabId, null).catch(() => null),
      ]);
      networkItems = detected.items;
    } catch {
      networkItems = [];
    }
    try {
      pageResult = (await browser.tabs.sendMessage(tabId, { type: 'scan' })) as
        | ScanResultMessage
        | undefined;
    } catch {
      pageResult = undefined;
    }

    // Re-fetch once more in case enrichHls resolved variants after the first read.
    try {
      const detectedAfter = await getDetectedMedia(tabId);
      networkItems = detectedAfter.items;
    } catch {
      // keep the first read
    }

    const items = mergeItems(networkItems, pageResult?.media ?? [], pageResult?.pageUrl ?? '');
    if (items.length === 0) {
      renderEmptyState(container, t('noMediaDetected') || 'No media detected on this page yet.');
      return;
    }

    const rows = items.map((item) => {
      const domain = hostnameOf(item.pageUrl ?? item.url) ?? '';
      const meta = [kindLabel(item.kind), domain].filter(Boolean);
      if (item.size !== null) meta.push(formatBytes(item.size));

      const variantOptions =
        item.variants && item.variants.length > 0
          ? item.variants.map((v) => ({
              value: v.id,
              label: `${v.label}${v.estimated_size ? ` · ${formatBytes(v.estimated_size)}` : ''}`,
            }))
          : undefined;
      if (variantOptions && !selectedVariant.has(item.url)) {
        selectedVariant.set(item.url, variantOptions[0]?.value ?? '');
      }

      const disabled = item.notDownloadable;
      return renderItemRow({
        title: item.title || item.url,
        meta,
        badges: disabled
          ? [{ text: t('notDownloadable') || 'Not downloadable', protectedStyle: true }]
          : [],
        variantSelect: variantOptions
          ? {
              options: variantOptions,
              ariaLabel: t('quality') || 'Quality',
              onChange: (value) => selectedVariant.set(item.url, value),
            }
          : undefined,
        actions: [
          {
            label: t('actionDownload') || 'Download',
            primary: true,
            disabled,
            onClick: async () => {
              const variantId = selectedVariant.get(item.url) ?? null;
              try {
                await quickDownload<AddTaskResult>(buildRequest(item, true, variantId));
                showToast(t('toastDownloadStarted') || 'Download started');
              } catch (err) {
                showToast(err instanceof Error ? err.message : String(err));
              }
            },
          },
          {
            label: t('actionQueue') || 'Queue',
            disabled,
            onClick: async () => {
              const variantId = selectedVariant.get(item.url) ?? null;
              try {
                await quickDownload<AddTaskResult>(buildRequest(item, false, variantId));
                showToast(t('toastQueued') || 'Queued');
              } catch (err) {
                showToast(err instanceof Error ? err.message : String(err));
              }
            },
          },
          {
            label: t('actionCopyUrl') || 'Copy URL',
            onClick: async () => {
              await navigator.clipboard.writeText(item.url);
              showToast(t('toastCopied') || 'Copied');
            },
          },
          {
            label: t('actionOpenSource') || 'Open source',
            onClick: async () => {
              await browser.tabs.create({ url: item.pageUrl ?? item.url });
            },
          },
        ],
      });
    });

    renderList(container, rows);
  }

  return { refresh };
}
