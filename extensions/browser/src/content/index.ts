/**
 * Content script (docs/api/extension.md "Media detection" + deliverable spec): scans the page for
 * candidate links and media on demand — it never injects any UI, and does nothing until asked.
 *
 * DRM note: `navigator.requestMediaKeySystemAccess` usage cannot be observed from a content
 * script, so instead we flag `<video>`/`<audio>` elements whose `src` is `blob:` (the universal
 * pattern for MSE/EME playback — YouTube, Netflix, etc all feed a `MediaSource` blob URL into the
 * element) and elements that fire the `encrypted` event, both as "not downloadable".
 */

import browser from 'webextension-polyfill';
import type { CandidateLink, CandidatePageMedia, ScanResultMessage } from '../shared/messages.ts';

const LARGE_IMAGE_MIN_DIMENSION = 300;
const DOWNLOAD_EXTENSIONS = new Set([
  'zip', 'rar', '7z', 'tar', 'gz', 'iso', 'dmg', 'pkg', 'exe', 'msi', 'mp4', 'mkv', 'mp3',
  'flac', 'pdf', 'epub', 'apk', 'deb', 'rpm', 'torrent', 'm3u8', 'webm', 'mov', 'avi', 'wav',
]);

const PLAYLIST_LINK_TYPES = new Set([
  'application/vnd.apple.mpegurl',
  'application/x-mpegurl',
  'application/dash+xml',
]);

/** Elements observed to have fired the `encrypted` event (EME) — never cleared; a page that
 *  starts DRM playback stays flagged for the lifetime of this content script instance. */
const encryptedElements = new WeakSet<Element>();

document.addEventListener(
  'encrypted',
  (event) => {
    if (event.target instanceof Element) encryptedElements.add(event.target);
  },
  true, // capture: `encrypted` does not bubble
);

function looksDownloadable(url: string): boolean {
  const match = /\.([a-z0-9]{1,8})(?:[?#]|$)/i.exec(url);
  const ext = match?.[1]?.toLowerCase();
  return !!ext && DOWNLOAD_EXTENSIONS.has(ext);
}

function isHttpUrl(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

function collectLinks(onlyWithinSelection: boolean): CandidateLink[] {
  const anchors = Array.from(document.querySelectorAll<HTMLAnchorElement>('a[href]'));
  const selection = onlyWithinSelection ? window.getSelection() : null;
  const seen = new Set<string>();
  const out: CandidateLink[] = [];

  for (const a of anchors) {
    const url = a.href;
    if (!isHttpUrl(url) || seen.has(url)) continue;
    if (onlyWithinSelection) {
      if (!selection || selection.isCollapsed || !selection.containsNode(a, true)) continue;
    }
    seen.add(url);
    const text = (a.textContent ?? '').trim().replace(/\s+/g, ' ').slice(0, 200);
    out.push({ url, text: text.length > 0 ? text : url, looksDownloadable: looksDownloadable(url) });
  }
  return out;
}

function mediaSourceUrl(el: HTMLMediaElement): string | null {
  if (el.currentSrc) return el.currentSrc;
  if (el.src) return el.src;
  const source = el.querySelector('source[src]');
  return source instanceof HTMLSourceElement ? source.src : null;
}

function nearbyTitle(el: Element): string | null {
  const aria = el.getAttribute('aria-label') || el.getAttribute('title');
  if (aria) return aria.trim();
  const heading = el.closest('article, section, figure')?.querySelector('h1, h2, h3, figcaption');
  const text = heading?.textContent?.trim();
  return text && text.length > 0 ? text : null;
}

function collectVideoAudio(): CandidatePageMedia[] {
  const out: CandidatePageMedia[] = [];
  const elements = document.querySelectorAll<HTMLMediaElement>('video, audio');
  for (const el of elements) {
    const url = mediaSourceUrl(el);
    if (!url) continue;
    const isBlob = url.startsWith('blob:');
    out.push({
      url,
      kind: el.tagName.toLowerCase() === 'video' ? 'video' : 'audio',
      title: nearbyTitle(el) ?? document.title,
      notDownloadable: isBlob || encryptedElements.has(el),
      poster: el instanceof HTMLVideoElement ? el.poster || null : null,
      duration: Number.isFinite(el.duration) ? el.duration : null,
      width: el instanceof HTMLVideoElement ? el.videoWidth || null : null,
      height: el instanceof HTMLVideoElement ? el.videoHeight || null : null,
    });
  }
  return out;
}

function collectLargeImages(): CandidatePageMedia[] {
  const out: CandidatePageMedia[] = [];
  const images = document.querySelectorAll<HTMLImageElement>('img[src]');
  for (const img of images) {
    const width = img.naturalWidth || img.width;
    const height = img.naturalHeight || img.height;
    if (width < LARGE_IMAGE_MIN_DIMENSION && height < LARGE_IMAGE_MIN_DIMENSION) continue;
    if (!isHttpUrl(img.src)) continue;
    out.push({
      url: img.src,
      kind: 'image',
      title: img.alt || nearbyTitle(img),
      notDownloadable: false,
      width,
      height,
    });
  }
  return out;
}

function collectAlternatePlaylists(): CandidatePageMedia[] {
  const out: CandidatePageMedia[] = [];
  const links = document.querySelectorAll<HTMLLinkElement>('link[rel~="alternate"][href]');
  for (const link of links) {
    const type = (link.type || '').toLowerCase();
    if (!PLAYLIST_LINK_TYPES.has(type)) continue;
    if (!isHttpUrl(link.href)) continue;
    out.push({
      url: link.href,
      kind: type === 'application/dash+xml' ? 'dash_manifest' : 'hls_playlist',
      title: link.title || document.title,
      notDownloadable: false,
    });
  }
  return out;
}

function buildScanResult(mode: 'scan' | 'scan-selection'): ScanResultMessage {
  const media = [...collectVideoAudio(), ...collectLargeImages(), ...collectAlternatePlaylists()];
  return {
    type: 'scan-result',
    pageUrl: location.href,
    pageTitle: document.title,
    links: collectLinks(mode === 'scan-selection'),
    media,
  };
}

browser.runtime.onMessage.addListener((message: unknown) => {
  if (typeof message !== 'object' || message === null || !('type' in message)) return undefined;
  const type = (message as { type: unknown }).type;
  if (type === 'scan' || type === 'scan-selection') {
    return Promise.resolve(buildScanResult(type));
  }
  return undefined;
});
