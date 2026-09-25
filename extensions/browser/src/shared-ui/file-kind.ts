/**
 * Maps a file name (and optionally the task kind) to the icon and tint of its file-type tile,
 * matching the desktop app's file badges. Pure: no DOM access.
 */

import type { IconName } from './icons.ts';

export interface FileKind {
  icon: IconName;
  /** Suffix of the `.tile.t-*` CSS class that picks the tint. */
  tint: 'video' | 'audio' | 'image' | 'archive' | 'disk' | 'package' | 'pdf' | 'table' | 'torrent' | 'stream' | 'default';
}

const BY_EXTENSION: Record<string, FileKind> = {};
function register(exts: string, kind: FileKind): void {
  for (const ext of exts.split(' ')) BY_EXTENSION[ext] = kind;
}
register('mp4 mkv mov webm m4v avi ts wmv flv', { icon: 'video', tint: 'video' });
register('m3u8 mpd', { icon: 'video', tint: 'stream' });
register('mp3 m4a flac wav aac ogg opus wma', { icon: 'audio', tint: 'audio' });
register('jpg jpeg png gif heic webp svg tiff avif bmp raw', { icon: 'image', tint: 'image' });
register('zip rar 7z gz tar bz2 xz tgz zst', { icon: 'archive', tint: 'archive' });
register('dmg iso img', { icon: 'disk', tint: 'disk' });
register('pkg app exe msi deb rpm apk appimage', { icon: 'package', tint: 'package' });
register('pdf', { icon: 'doc', tint: 'pdf' });
register('txt md rtf doc docx pages odt epub', { icon: 'doc', tint: 'default' });
register('csv xls xlsx numbers json xml sql', { icon: 'table', tint: 'table' });
register('swift py js ts rs go c h sh', { icon: 'code', tint: 'default' });
register('torrent', { icon: 'torrent', tint: 'torrent' });

export function extensionOf(name: string): string {
  const clean = name.split(/[?#]/)[0] ?? '';
  const match = /\.([a-z0-9]{1,10})$/i.exec(clean);
  return match?.[1]?.toLowerCase() ?? '';
}

export function fileKindOf(name: string, taskKind?: string): FileKind {
  const ext = extensionOf(name);
  if ((taskKind === 'torrent' || taskKind === 'magnet') && ext !== 'iso' && ext !== 'img') {
    return { icon: 'torrent', tint: 'torrent' };
  }
  if (taskKind === 'hls') return { icon: 'video', tint: 'stream' };
  return BY_EXTENSION[ext] ?? { icon: 'file', tint: 'default' };
}

export function mediaKindTile(kind: string): FileKind {
  switch (kind) {
    case 'video':
      return { icon: 'video', tint: 'video' };
    case 'audio':
      return { icon: 'audio', tint: 'audio' };
    case 'image':
      return { icon: 'image', tint: 'image' };
    case 'hls_playlist':
    case 'dash_manifest':
      return { icon: 'video', tint: 'stream' };
    default:
      return { icon: 'media', tint: 'default' };
  }
}
