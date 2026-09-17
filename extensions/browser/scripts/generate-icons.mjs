#!/usr/bin/env node
// Generates the extension's PNG icons at build time using only Node's built-in `zlib` module —
// no third-party image library. Draws a filled circle in the Osprey accent colour on a
// transparent background, anti-aliased at the edge, for each required icon size.
//
// This keeps `icons/*.png` out of source control (see icons/README noted in build.mjs) while
// still producing real, valid PNGs that Chrome/Firefox/Edge can load as action/toolbar icons.

import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ICONS_DIR = path.join(__dirname, '..', 'icons');

/** Osprey accent colour (brand blue). */
const ACCENT = { r: 0x2f, g: 0x6f, b: 0xed };
const SIZES = [16, 32, 48, 128];

// --- Minimal PNG encoder -------------------------------------------------------------------

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  }
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const typeBuf = Buffer.from(type, 'ascii');
  const lenBuf = Buffer.alloc(4);
  lenBuf.writeUInt32BE(data.length, 0);
  const crcInput = Buffer.concat([typeBuf, data]);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(crcInput), 0);
  return Buffer.concat([lenBuf, typeBuf, data, crcBuf]);
}

/** Encode an RGBA8 buffer (row-major, no filter bytes) as a PNG file buffer. */
function encodePng(width, height, rgba) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  const ihdrData = Buffer.alloc(13);
  ihdrData.writeUInt32BE(width, 0);
  ihdrData.writeUInt32BE(height, 4);
  ihdrData.writeUInt8(8, 8); // bit depth
  ihdrData.writeUInt8(6, 9); // color type: RGBA
  ihdrData.writeUInt8(0, 10); // compression
  ihdrData.writeUInt8(0, 11); // filter
  ihdrData.writeUInt8(0, 12); // interlace
  const ihdr = chunk('IHDR', ihdrData);

  // Prefix every scanline with filter type 0 (None).
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  }
  const idat = chunk('IDAT', deflateSync(raw, { level: 9 }));

  const iend = chunk('IEND', Buffer.alloc(0));

  return Buffer.concat([signature, ihdr, idat, iend]);
}

// --- Drawing ---------------------------------------------------------------------------------

/** Filled circle with a soft anti-aliased edge, centred, ~88% of the canvas diameter. */
function drawIcon(size) {
  const rgba = Buffer.alloc(size * size * 4);
  const cx = (size - 1) / 2;
  const cy = (size - 1) / 2;
  const r = size * 0.44;
  const featherStart = r - 1;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = x - cx;
      const dy = y - cy;
      const dist = Math.sqrt(dx * dx + dy * dy);
      let alpha = 0;
      if (dist <= featherStart) {
        alpha = 255;
      } else if (dist <= r) {
        alpha = Math.round(255 * (1 - (dist - featherStart) / (r - featherStart)));
      }
      const i = (y * size + x) * 4;
      rgba[i] = ACCENT.r;
      rgba[i + 1] = ACCENT.g;
      rgba[i + 2] = ACCENT.b;
      rgba[i + 3] = alpha;
    }
  }

  // A simple downward "arrow into a tray" glyph in white, scaled to the icon — evokes a
  // download manager without needing any vector/image library.
  const glyphColor = { r: 255, g: 255, b: 255 };
  const stemWidth = Math.max(1, Math.round(size * 0.09));
  const stemTop = size * 0.26;
  const stemBottom = size * 0.56;
  const stemCx = cx;
  for (let y = Math.round(stemTop); y <= Math.round(stemBottom); y++) {
    for (let x = Math.round(stemCx - stemWidth / 2); x <= Math.round(stemCx + stemWidth / 2); x++) {
      setPixelIfInside(rgba, size, x, y, glyphColor, 255);
    }
  }
  // Arrow head (triangle) under the stem.
  const headHalf = size * 0.2;
  const headTop = stemBottom;
  const headBottom = size * 0.72;
  for (let y = Math.round(headTop); y <= Math.round(headBottom); y++) {
    const t = (y - headTop) / (headBottom - headTop);
    const half = headHalf * (1 - t);
    for (let x = Math.round(stemCx - half); x <= Math.round(stemCx + half); x++) {
      setPixelIfInside(rgba, size, x, y, glyphColor, 255);
    }
  }
  // Base tray line.
  const trayY = Math.round(size * 0.8);
  const trayHalf = size * 0.3;
  const trayThickness = Math.max(1, Math.round(size * 0.07));
  for (let y = trayY; y < trayY + trayThickness; y++) {
    for (let x = Math.round(stemCx - trayHalf); x <= Math.round(stemCx + trayHalf); x++) {
      setPixelIfInside(rgba, size, x, y, glyphColor, 255);
    }
  }

  return rgba;
}

function setPixelIfInside(rgba, size, x, y, color, alpha) {
  if (x < 0 || y < 0 || x >= size || y >= size) return;
  const i = (y * size + x) * 4;
  const existingAlpha = rgba[i + 3] ?? 0;
  if (existingAlpha === 0) return; // stay within the circle background
  rgba[i] = color.r;
  rgba[i + 1] = color.g;
  rgba[i + 2] = color.b;
  rgba[i + 3] = alpha;
}

function main() {
  mkdirSync(ICONS_DIR, { recursive: true });
  for (const size of SIZES) {
    const rgba = drawIcon(size);
    const png = encodePng(size, size, rgba);
    const out = path.join(ICONS_DIR, `icon${size}.png`);
    writeFileSync(out, png);
    console.log(`wrote ${path.relative(process.cwd(), out)} (${png.length} bytes)`);
  }
}

main();
