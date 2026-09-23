/**
 * Classifies network responses / URLs into a `MediaKind` for the media detector
 * (docs/api/extension.md "Media detection"). Pure module — no browser globals.
 */

import type { MediaKind } from './types.ts';

const PLAYLIST_MIME_TYPES = new Set([
  'application/vnd.apple.mpegurl',
  'application/x-mpegurl',
  'audio/mpegurl',
  'audio/x-mpegurl',
]);

const DASH_MIME_TYPES = new Set(['application/dash+xml']);

const MEDIA_EXTENSIONS: Record<string, MediaKind> = {
  m3u8: 'hls_playlist',
  mpd: 'dash_manifest',
  mp4: 'video',
  m4v: 'video',
  mkv: 'video',
  webm: 'video',
  mov: 'video',
  avi: 'video',
  flv: 'video',
  ts: 'video',
  mp3: 'audio',
  m4a: 'audio',
  aac: 'audio',
  flac: 'audio',
  wav: 'audio',
  ogg: 'audio',
  opus: 'audio',
};

export function classifyContentType(contentType: string | null | undefined): MediaKind | null {
  if (!contentType) return null;
  const mime = contentType.split(';')[0]?.trim().toLowerCase() ?? '';
  if (!mime) return null;
  if (PLAYLIST_MIME_TYPES.has(mime)) return 'hls_playlist';
  if (DASH_MIME_TYPES.has(mime)) return 'dash_manifest';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  if (mime.startsWith('image/')) return 'image';
  return null;
}

export function classifyUrlExtension(url: string): MediaKind | null {
  let pathname: string;
  try {
    pathname = new URL(url).pathname;
  } catch {
    pathname = url;
  }
  const base = pathname.split('/').pop() ?? '';
  const dot = base.lastIndexOf('.');
  if (dot <= 0 || dot === base.length - 1) return null;
  const ext = base.slice(dot + 1).toLowerCase();
  return MEDIA_EXTENSIONS[ext] ?? null;
}

/** Content-Type takes priority; URL extension is the fallback (docs/api/extension.md). */
export function classifyMedia(
  contentType: string | null | undefined,
  url: string,
): MediaKind | null {
  return classifyContentType(contentType) ?? classifyUrlExtension(url);
}

export function parseContentLength(value: string | null | undefined): number | null {
  if (!value) return null;
  const n = Number.parseInt(value, 10);
  return Number.isFinite(n) && n >= 0 ? n : null;
}

export function findHeader(
  headers: readonly { name: string; value?: string }[] | undefined,
  name: string,
): string | null {
  if (!headers) return null;
  const lower = name.toLowerCase();
  const found = headers.find((h) => h.name.toLowerCase() === lower);
  return found?.value ?? null;
}
