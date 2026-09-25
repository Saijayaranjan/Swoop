/**
 * Swoop brand artwork, ported from the desktop app so the extension carries the same identity:
 *   - the wing glyph (a pair of crooked wings with swept feather tips over a tapered keel),
 *   - the mark (that glyph on a luminous blue tile),
 *   - the feather-fan illustration used for empty and offline states.
 * Everything is built with `createElementNS` from numbers — no markup parsing — so it is safe to
 * render inside the content script's shadow root as well as on extension pages.
 */

import { icon, type IconName } from './icons.ts';

const SVG_NS = 'http://www.w3.org/2000/svg';

type Attrs = Record<string, string | number>;

function svgEl<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  children: SVGElement[] = [],
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
  for (const child of children) node.appendChild(child);
  return node;
}

let idCounter = 0;
function uid(prefix: string): string {
  idCounter += 1;
  return `osp-${prefix}-${idCounter}`;
}

// --- Wing glyph ------------------------------------------------------------------------------

/** The wing glyph on a 100×64 grid (the right wing, its mirror image, and the keel). */
export const WING_PATH =
  'M50 40C56 30 61 15 71 13C81 11 92 16 99 24L90 26.5L94 31L84 30L87 35L77 32.5C67 33 58 42 52 52Z' +
  'M50 40C44 30 39 15 29 13C19 11 8 16 1 24L10 26.5L6 31L16 30L13 35L23 32.5C33 33 42 42 48 52Z' +
  'M50 34C53 38 55 45 54 50L50 62L46 50C45 45 47 38 50 34Z';

export function wingGlyph(fill = '#ffffff'): SVGSVGElement {
  return svgEl('svg', { viewBox: '0 0 100 64', 'aria-hidden': 'true', focusable: 'false' }, [
    svgEl('path', { d: WING_PATH, fill }),
  ]);
}

/** The Swoop mark: the wing glyph on a blue tile. Decorative (`aria-hidden`). */
export function brandMark(size: number): HTMLSpanElement {
  const tile = document.createElement('span');
  tile.className = 'mark';
  tile.setAttribute('aria-hidden', 'true');
  tile.style.width = `${size}px`;
  tile.style.height = `${size}px`;
  tile.appendChild(wingGlyph());
  return tile;
}

// --- Feather illustration ----------------------------------------------------------------------

type Pt = [number, number];

/** One asymmetric feather in unit space (bottom tip at 0.5,1): vane, notch, rounded crown. */
function featherPath(w: number, h: number): string {
  const p = ([x, y]: Pt): string => `${((x - 0.5) * w).toFixed(2)} ${((y - 1) * h).toFixed(2)}`;
  return (
    `M${p([0.5, 1])}` +
    `C${p([-0.05, 0.62])} ${p([0.12, 0.08])} ${p([0.58, 0])}` +
    `C${p([0.86, 0.08])} ${p([0.98, 0.26])} ${p([0.86, 0.42])}` +
    `L${p([0.66, 0.47])}L${p([0.82, 0.52])}` +
    `C${p([0.76, 0.75])} ${p([0.58, 0.86])} ${p([0.5, 1])}Z`
  );
}

/** The shaft with six pairs of faint barbs. */
function rachisPath(w: number, h: number): string {
  const p = ([x, y]: Pt): string => `${((x - 0.5) * w).toFixed(2)} ${((y - 1) * h).toFixed(2)}`;
  let d = `M${p([0.5, 1.06])}Q${p([0.46, 0.4])} ${p([0.57, 0.06])}`;
  for (let i = 1; i <= 6; i++) {
    const t = i / 7.5;
    const y = 1 - t * 0.95;
    const x = 0.49 + 0.02 * t;
    d += `M${p([x, y])}L${p([x + 0.22, y - 0.07])}M${p([x, y])}L${p([x - 0.22, y - 0.06])}`;
  }
  return d;
}

export interface FeatherArtOptions {
  /** Rendered width in CSS px (height is 0.8× this). */
  width: number;
  /** When set, a smaller fan frames a luminous tile carrying this icon. */
  glyph?: IconName;
  /** Muted variant for offline/warning states. */
  muted?: boolean;
}

/** The empty-state artwork: a fan of feathers rising from a soft glow. Decorative. */
export function featherIllustration(opts: FeatherArtOptions): SVGSVGElement {
  const S = 200;
  const cx = 100;
  const pivotY = 80 + S * 0.18;
  const feathers: Array<[angle: number, scale: number, hue: number]> = opts.glyph
    ? [[-58, 0.62, 0.6], [-30, 0.78, 0.63], [30, 0.78, 0.7], [58, 0.62, 0.73]]
    : [[-52, 0.62, 0.6], [-27, 0.8, 0.63], [0, 1, 0.66], [27, 0.8, 0.7], [52, 0.62, 0.73]];

  const defs = svgEl('defs');
  const glowId = uid('glow');
  defs.appendChild(
    svgEl('radialGradient', { id: glowId }, [
      svgEl('stop', { offset: '0%', 'stop-color': opts.muted ? '#8e8e93' : '#3d8bff', 'stop-opacity': 0.34 }),
      svgEl('stop', { offset: '50%', 'stop-color': '#7a5cf0', 'stop-opacity': 0.12 }),
      svgEl('stop', { offset: '100%', 'stop-color': '#7a5cf0', 'stop-opacity': 0 }),
    ]),
  );

  const floatGroup = svgEl('g', { class: 'float' });
  const fan = svgEl('g', { style: 'filter: drop-shadow(0 6px 8px rgba(70, 80, 220, 0.28))' });
  for (const [angle, scale, hue] of feathers) {
    const h = S * 0.56 * scale;
    const w = h * 0.36;
    const gradId = uid('vane');
    const sheenId = uid('sheen');
    const deg = Math.round(hue * 360);
    const sat = opts.muted ? 18 : 100;
    const sat2 = opts.muted ? 14 : 81;
    defs.appendChild(
      svgEl('linearGradient', { id: gradId, x1: 0, y1: 0, x2: 0, y2: 1 }, [
        svgEl('stop', { offset: '0%', 'stop-color': `hsl(${deg} ${sat}% 73%)` }),
        svgEl('stop', { offset: '100%', 'stop-color': `hsl(${deg + 7} ${sat2}% 57%)` }),
      ]),
    );
    defs.appendChild(
      svgEl('linearGradient', { id: sheenId, x1: 0, y1: 0, x2: 0.6, y2: 0.6 }, [
        svgEl('stop', { offset: '0%', 'stop-color': '#ffffff', 'stop-opacity': 0.5 }),
        svgEl('stop', { offset: '100%', 'stop-color': '#ffffff', 'stop-opacity': 0 }),
      ]),
    );
    const d = featherPath(w, h);
    fan.appendChild(
      svgEl('g', { transform: `translate(${cx} ${pivotY}) rotate(${angle})` }, [
        svgEl('path', { d, fill: `url(#${gradId})` }),
        svgEl('path', { d, fill: `url(#${sheenId})` }),
        svgEl('path', {
          d: rachisPath(w, h),
          fill: 'none',
          stroke: '#ffffff',
          'stroke-opacity': 0.55,
          'stroke-width': 1.1,
          'stroke-linecap': 'round',
        }),
      ]),
    );
  }
  floatGroup.appendChild(fan);

  if (opts.glyph) {
    const tileId = uid('tile');
    const edgeId = uid('edge');
    defs.appendChild(
      svgEl('linearGradient', { id: tileId, x1: 0, y1: 0, x2: 1, y2: 1 }, [
        svgEl('stop', { offset: '0%', 'stop-color': opts.muted ? '#9aa3b5' : '#4da3ff' }),
        svgEl('stop', { offset: '100%', 'stop-color': opts.muted ? '#6f6c8f' : '#5c4deb' }),
      ]),
    );
    defs.appendChild(
      svgEl('linearGradient', { id: edgeId, x1: 0, y1: 0, x2: 0, y2: 1 }, [
        svgEl('stop', { offset: '0%', 'stop-color': '#ffffff', 'stop-opacity': 0.6 }),
        svgEl('stop', { offset: '100%', 'stop-color': '#ffffff', 'stop-opacity': 0.05 }),
      ]),
    );
    const size = S * 0.34;
    const x = cx - size / 2;
    const y = 80 + S * 0.02 - size / 2;
    floatGroup.appendChild(
      svgEl('rect', {
        x,
        y,
        width: size,
        height: size,
        rx: S * 0.1,
        fill: `url(#${tileId})`,
        stroke: `url(#${edgeId})`,
        'stroke-width': 1,
        style: 'filter: drop-shadow(0 6px 10px rgba(61, 139, 255, 0.4))',
      }),
    );
    const glyph = icon(opts.glyph, '');
    const g = size * 0.46;
    glyph.setAttribute('x', String(cx - g / 2));
    glyph.setAttribute('y', String(y + (size - g) / 2));
    glyph.setAttribute('width', String(g));
    glyph.setAttribute('height', String(g));
    glyph.setAttribute('stroke', '#ffffff');
    glyph.setAttribute('stroke-width', '2.2');
    glyph.setAttribute('color', '#ffffff');
    floatGroup.appendChild(glyph);
  } else {
    const capId = uid('cap');
    defs.appendChild(
      svgEl('linearGradient', { id: capId, x1: 0, y1: 0, x2: 0, y2: 1 }, [
        svgEl('stop', { offset: '0%', 'stop-color': '#ffffff' }),
        svgEl('stop', { offset: '100%', 'stop-color': '#c7cfff' }),
      ]),
    );
    floatGroup.appendChild(
      svgEl('circle', {
        cx,
        cy: pivotY,
        r: S * 0.035,
        fill: `url(#${capId})`,
        style: 'filter: drop-shadow(0 0 5px rgba(61, 139, 255, 0.6))',
      }),
    );
  }

  for (let i = 0; i < 5; i++) {
    const a = i * 1.3 + 0.4;
    floatGroup.appendChild(
      svgEl('circle', {
        cx: (cx + Math.cos(a) * S * 0.44).toFixed(1),
        cy: (80 + Math.sin(a) * S * 0.3 - S * 0.12).toFixed(1),
        r: (3 + (i % 3)) / 2,
        fill: '#ffffff',
        'fill-opacity': 0.8,
        style: 'filter: drop-shadow(0 0 3px rgba(61, 139, 255, 0.7))',
      }),
    );
  }

  return svgEl(
    'svg',
    {
      class: 'illustration',
      viewBox: `0 0 ${S} ${S * 0.8}`,
      width: opts.width,
      height: Math.round(opts.width * 0.8),
      'aria-hidden': 'true',
      focusable: 'false',
    },
    [defs, svgEl('ellipse', { cx, cy: 84, rx: S * 0.5, ry: S * 0.38, fill: `url(#${glowId})` }), floatGroup],
  );
}
