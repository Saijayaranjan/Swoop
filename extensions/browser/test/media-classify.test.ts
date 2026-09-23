import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  classifyContentType,
  classifyMedia,
  classifyUrlExtension,
  findHeader,
  parseContentLength,
} from '../src/shared/media-classify.ts';

test('classifyContentType recognises video/audio/image and HLS/DASH mimes', () => {
  assert.equal(classifyContentType('video/mp4'), 'video');
  assert.equal(classifyContentType('audio/mpeg; charset=binary'), 'audio');
  assert.equal(classifyContentType('image/png'), 'image');
  assert.equal(classifyContentType('application/vnd.apple.mpegurl'), 'hls_playlist');
  assert.equal(classifyContentType('application/x-mpegurl'), 'hls_playlist');
  assert.equal(classifyContentType('application/dash+xml'), 'dash_manifest');
  assert.equal(classifyContentType('text/html'), null);
  assert.equal(classifyContentType(null), null);
  assert.equal(classifyContentType(undefined), null);
});

test('classifyUrlExtension falls back on the URL path', () => {
  assert.equal(classifyUrlExtension('https://example.com/video/master.m3u8'), 'hls_playlist');
  assert.equal(classifyUrlExtension('https://example.com/a/b/movie.mkv'), 'video');
  assert.equal(classifyUrlExtension('https://example.com/song.flac?x=1'), 'audio');
  assert.equal(classifyUrlExtension('https://example.com/manifest.mpd'), 'dash_manifest');
  assert.equal(classifyUrlExtension('https://example.com/page'), null);
});

test('classifyMedia prefers content-type over URL extension', () => {
  assert.equal(classifyMedia('video/mp4', 'https://example.com/file.unknown'), 'video');
  assert.equal(classifyMedia(null, 'https://example.com/clip.mp4'), 'video');
  assert.equal(classifyMedia('text/html', 'https://example.com/clip.mp4'), 'video');
  assert.equal(classifyMedia(null, 'https://example.com/page'), null);
});

test('parseContentLength parses valid non-negative integers only', () => {
  assert.equal(parseContentLength('12345'), 12345);
  assert.equal(parseContentLength('0'), 0);
  assert.equal(parseContentLength('-5'), null);
  assert.equal(parseContentLength('abc'), null);
  assert.equal(parseContentLength(null), null);
  assert.equal(parseContentLength(undefined), null);
});

test('findHeader is case-insensitive', () => {
  const headers = [
    { name: 'Content-Type', value: 'video/mp4' },
    { name: 'Content-Length', value: '42' },
  ];
  assert.equal(findHeader(headers, 'content-type'), 'video/mp4');
  assert.equal(findHeader(headers, 'CONTENT-LENGTH'), '42');
  assert.equal(findHeader(headers, 'etag'), null);
  assert.equal(findHeader(undefined, 'content-type'), null);
});
