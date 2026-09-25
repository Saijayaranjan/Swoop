#!/usr/bin/env node
// Generates the extension's PNG icons at build time using only Node's built-in `zlib` module —
// no third-party image library. Draws the Osprey mark — the wing glyph on a blue tile, the same
// artwork as the desktop app and src/shared-ui/brand.ts — with supersampled anti-aliasing.

import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ICONS_DIR = path.join(__dirname, '..', 'icons');

/** Tile gradient, top-left to bottom-right (matches `.mark` in src/shared-ui/tokens.css). */
const TILE_FROM = { r: 0x40, g: 0x9e, b: 0xff };
const TILE_TO = { r: 0x29, g: 0x5c, b: 0xed };
/** The wing glyph on a 100×64 grid (kept in sync with WING_PATH in src/shared-ui/brand.ts). */
const WING_PATH =
  'M50 40C56 30 61 15 71 13C81 11 92 16 99 24L90 26.5L94 31L84 30L87 35L77 32.5C67 33 58 42 52 52Z' +
  'M50 40C44 30 39 15 29 13C19 11 8 16 1 24L10 26.5L6 31L16 30L13 35L23 32.5C33 33 42 42 48 52Z' +
  'M50 34C53 38 55 45 54 50L50 62L46 50C45 45 47 38 50 34Z';
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

/** Parses the absolute M/C/L/Z path above into closed polygons, flattening each curve. */
function wingPolygons() {
  const tokens = WING_PATH.match(/[MCLZ]|-?\d*\.?\d+/g) ?? [];
  const polygons = [];
  let current = [];
  let i = 0;
  const num = () => Number(tokens[i++]);
  while (i < tokens.length) {
    const cmd = tokens[i++];
    if (cmd === 'M') {
      current = [[num(), num()]];
    } else if (cmd === 'L') {
      current.push([num(), num()]);
    } else if (cmd === 'C') {
      const [x0, y0] = current[current.length - 1];
      const x1 = num(), y1 = num(), x2 = num(), y2 = num(), x3 = num(), y3 = num();
      for (let s = 1; s <= 24; s++) {
        const t = s / 24;
        const u = 1 - t;
        current.push([
          u * u * u * x0 + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t * t * t * x3,
          u * u * u * y0 + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * y3,
        ]);
      }
    } else if (cmd === 'Z') {
      polygons.push(current);
    }
  }
  return polygons;
}

function insidePolygon(poly, x, y) {
  let winding = 0;
  for (let k = 0, j = poly.length - 1; k < poly.length; j = k++) {
    const [xi, yi] = poly[k];
    const [xj, yj] = poly[j];
    if (yj <= y) {
      if (yi > y && (xi - xj) * (y - yj) - (x - xj) * (yi - yj) > 0) winding += 1;
    } else if (yi <= y && (xi - xj) * (y - yj) - (x - xj) * (yi - yj) < 0) {
      winding -= 1;
    }
  }
  return winding !== 0;
}

function insideRoundedRect(x, y, size, radius) {
  const cx = Math.min(Math.max(x, radius), size - radius);
  const cy = Math.min(Math.max(y, radius), size - radius);
  return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius;
}

/** The mark: a full-bleed rounded tile with the white wing glyph centred on it. */
function drawIcon(size) {
  const rgba = Buffer.alloc(size * size * 4);
  const polygons = wingPolygons();
  const radius = size * 0.26;
  const glyphW = size * 0.76;
  const glyphH = glyphW * 0.64;
  const gx = (size - glyphW) / 2;
  const gy = (size - glyphH) / 2 + size * 0.03;
  const samples = size <= 32 ? 8 : 4;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let tile = 0;
      let glyph = 0;
      for (let sy = 0; sy < samples; sy++) {
        for (let sx = 0; sx < samples; sx++) {
          const px = x + (sx + 0.5) / samples;
          const py = y + (sy + 0.5) / samples;
          if (!insideRoundedRect(px, py, size, radius)) continue;
          tile += 1;
          const ux = ((px - gx) / glyphW) * 100;
          const uy = ((py - gy) / glyphH) * 64;
          if (ux >= 0 && ux <= 100 && uy >= 0 && uy <= 64 && polygons.some((p) => insidePolygon(p, ux, uy))) glyph += 1;
        }
      }
      const total = samples * samples;
      const t = (x + y) / (2 * size);
      const base = {
        r: TILE_FROM.r + (TILE_TO.r - TILE_FROM.r) * t,
        g: TILE_FROM.g + (TILE_TO.g - TILE_FROM.g) * t,
        b: TILE_FROM.b + (TILE_TO.b - TILE_FROM.b) * t,
      };
      const g = tile > 0 ? glyph / tile : 0;
      const i = (y * size + x) * 4;
      rgba[i] = Math.round(base.r + (255 - base.r) * g);
      rgba[i + 1] = Math.round(base.g + (255 - base.g) * g);
      rgba[i + 2] = Math.round(base.b + (255 - base.b) * g);
      rgba[i + 3] = Math.round((255 * tile) / total);
    }
  }
  return rgba;
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
