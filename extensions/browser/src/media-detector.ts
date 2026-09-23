/**
 * Passive network-level media detection (docs/api/extension.md "Media detection"):
 * `webRequest.onHeadersReceived` in observe-only mode (MV3 cannot use `declarativeNetRequest` to
 * *observe* response headers, and blocking `webRequest` is unavailable under MV3, so this simply
 * watches — it is never in the request path and can't slow anything down or break a page).
 * Records per-tab candidates, size from `Content-Length`, dedups by URL, caps at 200 entries per
 * tab, and clears a tab's entries on top-frame navigation. `.m3u8` entries are left thin (no
 * variants) until `enrichHlsForTab` is called — the popup does this once, on open, via
 * `POST /api/v1/media/detect`.
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './background/native-port.ts';
import type { SettingsStore } from './shared/settings.ts';
import { classifyMedia, findHeader, parseContentLength } from './shared/media-classify.ts';
import type { DetectedMedia } from './shared/types.ts';

const MAX_ENTRIES_PER_TAB = 200;

export class MediaDetector {
  private perTab = new Map<number, Map<string, DetectedMedia>>();
  private settingsStore: SettingsStore;

  constructor(settingsStore: SettingsStore) {
    this.settingsStore = settingsStore;
  }

  register(): void {
    browser.webRequest.onHeadersReceived.addListener(
      (details) => {
        void this.handleHeaders(details);
      },
      { urls: ['<all_urls>'] },
      ['responseHeaders'],
    );

    browser.webNavigation.onBeforeNavigate.addListener((details) => {
      if (details.frameId === 0) this.clearTab(details.tabId);
    });

    browser.tabs.onRemoved.addListener((tabId) => this.clearTab(tabId));
  }

  private async handleHeaders(details: browser.WebRequest.OnHeadersReceivedDetailsType): Promise<void> {
    if (details.tabId < 0) return;
    const settings = await this.settingsStore.get();
    if (!settings.detect_media) return;

    const headers = details.responseHeaders as
      | { name: string; value?: string }[]
      | undefined;
    const contentType = findHeader(headers, 'content-type');
    const kind = classifyMedia(contentType, details.url);
    if (!kind) return;

    const size = parseContentLength(findHeader(headers, 'content-length'));
    this.addItem(details.tabId, {
      url: details.url,
      kind,
      mime: contentType ?? undefined,
      size,
    });
  }

  private addItem(tabId: number, item: DetectedMedia): void {
    let map = this.perTab.get(tabId);
    if (!map) {
      map = new Map();
      this.perTab.set(tabId, map);
    }
    if (map.has(item.url)) return;
    if (map.size >= MAX_ENTRIES_PER_TAB) return;
    map.set(item.url, item);
  }

  clearTab(tabId: number): void {
    this.perTab.delete(tabId);
  }

  getForTab(tabId: number): DetectedMedia[] {
    return [...(this.perTab.get(tabId)?.values() ?? [])];
  }

  /** Resolve HLS master-playlist variants/estimated size once, when the popup opens on this tab. */
  async enrichHlsForTab(tabId: number, pageUrl: string | null, nativePort: NativePort): Promise<void> {
    const map = this.perTab.get(tabId);
    if (!map) return;
    const targets = [...map.values()].filter(
      (item) => item.kind === 'hls_playlist' && (!item.variants || item.variants.length === 0),
    );
    await Promise.all(
      targets.map(async (item) => {
        try {
          const detected = await nativePort.post<DetectedMedia>('/api/v1/media/detect', {
            url: item.url,
            page_url: pageUrl,
          });
          map.set(item.url, { ...item, ...detected });
        } catch {
          // Leave the thin entry in place; the popup can still offer the raw playlist URL.
        }
      }),
    );
  }
}
