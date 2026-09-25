/**
 * Osprey's small stroke icon set, drawn on a 24×24 grid. Icons are built with
 * `createElementNS` from plain data (never parsed from markup strings) so they are safe to use in
 * the popup, the options page and inside the content script's shadow root on any page.
 */

const SVG_NS = 'http://www.w3.org/2000/svg';

type Attrs = Record<string, string | number>;
type IconNode = [tag: 'path' | 'rect' | 'circle', attrs: Attrs];

const FILL: Attrs = { fill: 'currentColor', stroke: 'none' };

const ICONS = {
  arrowDown: [['path', { d: 'M12 5v14M6.5 13.5 12 19l5.5-5.5' }]],
  arrowUp: [['path', { d: 'M12 19V5M6.5 10.5 12 5l5.5 5.5' }]],
  download: [
    ['path', { d: 'M12 4v10.5M7.5 10l4.5 4.5 4.5-4.5' }],
    ['path', { d: 'M5 19.5h14' }],
  ],
  pause: [
    ['rect', { x: 7, y: 5.5, width: 3.4, height: 13, rx: 1.3, ...FILL }],
    ['rect', { x: 13.6, y: 5.5, width: 3.4, height: 13, rx: 1.3, ...FILL }],
  ],
  play: [['path', { d: 'M8.5 6.3v11.4a1 1 0 0 0 1.52.85l9.1-5.7a1 1 0 0 0 0-1.7l-9.1-5.7a1 1 0 0 0-1.52.85z', ...FILL }]],
  close: [['path', { d: 'M7 7l10 10M17 7 7 17' }]],
  retry: [['path', { d: 'M19.5 12a7.5 7.5 0 1 1-2.2-5.3' }], ['path', { d: 'M19.5 4.5v4h-4' }]],
  open: [
    ['path', { d: 'M13.5 4.5h6v6M19.5 4.5 11 13' }],
    ['path', { d: 'M18 14v4a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 5 18V7.5A1.5 1.5 0 0 1 6.5 6H10' }],
  ],
  settings: [
    ['path', { d: 'M4 8h9M17 8h3M4 16h3M11 16h9' }],
    ['circle', { cx: 15, cy: 8, r: 2.1 }],
    ['circle', { cx: 9, cy: 16, r: 2.1 }],
  ],
  link: [
    ['path', { d: 'M10.5 13.5a3.5 3.5 0 0 0 5 0l3-3a3.5 3.5 0 0 0-5-5l-.8.8' }],
    ['path', { d: 'M13.5 10.5a3.5 3.5 0 0 0-5 0l-3 3a3.5 3.5 0 0 0 5 5l.8-.8' }],
  ],
  paste: [
    ['rect', { x: 5.5, y: 5, width: 13, height: 15.5, rx: 2.5 }],
    ['path', { d: 'M9 5v-.5A1.5 1.5 0 0 1 10.5 3h3A1.5 1.5 0 0 1 15 4.5V5M9 11h6M9 15h4' }],
  ],
  copy: [
    ['rect', { x: 8.5, y: 8.5, width: 11, height: 11, rx: 2.5 }],
    ['path', { d: 'M15.5 8.5V6A1.5 1.5 0 0 0 14 4.5H6A1.5 1.5 0 0 0 4.5 6v8A1.5 1.5 0 0 0 6 15.5h2.5' }],
  ],
  queue: [['path', { d: 'M4.5 7h11M4.5 12h11M4.5 17h6M17.5 14v6M14.5 17h6' }]],
  check: [['path', { d: 'M5.5 12.5l4 4 9-9' }]],
  plus: [['path', { d: 'M12 5v14M5 12h14' }]],
  video: [
    ['rect', { x: 3.5, y: 5.5, width: 17, height: 13, rx: 2.8 }],
    ['path', { d: 'M10.2 9.4v5.2l4.4-2.6z', ...FILL }],
  ],
  audio: [['path', { d: 'M4 10.5v3M8 7.5v9M12 4.5v15M16 8v8M20 10.5v3' }]],
  image: [
    ['rect', { x: 3.5, y: 5, width: 17, height: 14, rx: 2.8 }],
    ['circle', { cx: 9, cy: 10, r: 1.6 }],
    ['path', { d: 'M20.5 15.5 15.5 11 7 19' }],
  ],
  archive: [
    ['rect', { x: 3.5, y: 4.5, width: 17, height: 5, rx: 1.6 }],
    ['path', { d: 'M5 9.5V18a1.5 1.5 0 0 0 1.5 1.5h11A1.5 1.5 0 0 0 19 18V9.5M10 13h4' }],
  ],
  disk: [['circle', { cx: 12, cy: 12, r: 8.5 }], ['circle', { cx: 12, cy: 12, r: 2.3 }]],
  package: [['path', { d: 'M12 3.5l7.5 4.2v8.6L12 20.5l-7.5-4.2V7.7zM4.5 7.7 12 12l7.5-4.3M12 12v8.5' }]],
  doc: [
    ['path', { d: 'M13.5 3.5h-6A1.5 1.5 0 0 0 6 5v14a1.5 1.5 0 0 0 1.5 1.5h9A1.5 1.5 0 0 0 18 19V8z' }],
    ['path', { d: 'M13.5 3.5V8H18M9 13h6M9 16.5h4' }],
  ],
  table: [
    ['rect', { x: 4, y: 4.5, width: 16, height: 15, rx: 2.2 }],
    ['path', { d: 'M4 9.5h16M4 14.5h16M10 9.5v10' }],
  ],
  code: [['path', { d: 'M9 7.5 4.5 12 9 16.5M15 7.5l4.5 4.5-4.5 4.5' }]],
  torrent: [
    ['circle', { cx: 6, cy: 12, r: 2.2 }],
    ['circle', { cx: 18, cy: 6, r: 2.2 }],
    ['circle', { cx: 18, cy: 18, r: 2.2 }],
    ['path', { d: 'M8 11l8-4M8 13l8 4' }],
  ],
  stream: [
    ['rect', { x: 3.5, y: 5.5, width: 17, height: 13, rx: 2.8 }],
    ['path', { d: 'M7.5 15l3-3.2 2.6 2.4 3.4-4' }],
  ],
  file: [
    ['path', { d: 'M13.5 3.5h-6A1.5 1.5 0 0 0 6 5v14a1.5 1.5 0 0 0 1.5 1.5h9A1.5 1.5 0 0 0 18 19V8z' }],
    ['path', { d: 'M13.5 3.5V8H18' }],
  ],
  globe: [
    ['circle', { cx: 12, cy: 12, r: 8.5 }],
    ['path', { d: 'M3.5 12h17M12 3.5c2.4 2.5 3.5 5.3 3.5 8.5s-1.1 6-3.5 8.5c-2.4-2.5-3.5-5.3-3.5-8.5s1.1-6 3.5-8.5z' }],
  ],
  bell: [
    ['path', { d: 'M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 1.5h-15z' }],
    ['path', { d: 'M10 20.5a2 2 0 0 0 4 0' }],
  ],
  capture: [
    ['path', { d: 'M12 3.5v10M8 9.5l4 4 4-4' }],
    ['path', { d: 'M4.5 14v3.5a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V14' }],
  ],
  plug: [
    ['path', { d: 'M9 3.5V8M15 3.5V8M12 16v4.5' }],
    ['path', { d: 'M6.5 8h11v2.5a5.5 5.5 0 0 1-11 0z' }],
  ],
  keyboard: [
    ['rect', { x: 3, y: 6, width: 18, height: 12, rx: 2.6 }],
    ['path', { d: 'M7 10h.01M10.3 10h.01M13.7 10h.01M17 10h.01M8 14h8' }],
  ],
  info: [['circle', { cx: 12, cy: 12, r: 8.5 }], ['path', { d: 'M12 11v5.5M12 7.8v.01' }]],
  chevronLeft: [['path', { d: 'M14.5 6 8.5 12l6 6' }]],
  chevronRight: [['path', { d: 'M9.5 6l6 6-6 6' }]],
  chevronDown: [['path', { d: 'M6 9.5l6 6 6-6' }]],
  power: [['path', { d: 'M12 3.5v8M7.2 6.6a7 7 0 1 0 9.6 0' }]],
  media: [['circle', { cx: 12, cy: 12, r: 8.5 }], ['path', { d: 'M10.3 9v6l5-3z', ...FILL }]],
  shield: [
    ['path', { d: 'M12 3.5l7 2.8v5.2c0 4.4-3 7.7-7 9-4-1.3-7-4.6-7-9V6.3z' }],
    ['path', { d: 'M9 12l2.2 2.2L15.5 10' }],
  ],
  warning: [['path', { d: 'M12 4.2 21 19.5H3zM12 10v4.3M12 17v.01' }]],
  lock: [
    ['rect', { x: 5.5, y: 10.5, width: 13, height: 10, rx: 2.3 }],
    ['path', { d: 'M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5' }],
  ],
  server: [
    ['rect', { x: 4, y: 4.5, width: 16, height: 6.5, rx: 2 }],
    ['rect', { x: 4, y: 13, width: 16, height: 6.5, rx: 2 }],
    ['path', { d: 'M7.5 7.75h.01M7.5 16.25h.01' }],
  ],
  laptop: [['rect', { x: 5, y: 5, width: 14, height: 10, rx: 1.8 }], ['path', { d: 'M3 18.5h18' }]],
  hourglass: [['path', { d: 'M7 4h10M7 20h10M8 4c0 4 8 4 8 8s-8 4-8 8M16 4c0 4-8 4-8 8s8 4 8 8' }]],
  sparkle: [['path', { d: 'M12 4c.6 3.9 2.1 5.4 6 6-3.9.6-5.4 2.1-6 6-.6-3.9-2.1-5.4-6-6 3.9-.6 5.4-2.1 6-6z' }]],
} satisfies Record<string, IconNode[]>;

export type IconName = keyof typeof ICONS;

export function icon(name: IconName, className = 'icon'): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('focusable', 'false');
  svg.setAttribute('class', className);
  for (const [tag, attrs] of ICONS[name] as IconNode[]) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
    svg.appendChild(node);
  }
  return svg;
}

export function isIconName(name: string): name is IconName {
  return Object.prototype.hasOwnProperty.call(ICONS, name);
}

/** Replace every `<span data-icon="name">` placeholder under `root` with the real icon. */
export function hydrateIcons(root: ParentNode): void {
  for (const slot of root.querySelectorAll<HTMLElement>('[data-icon]')) {
    const name = slot.dataset['icon'] ?? '';
    if (!isIconName(name)) continue;
    const svg = icon(name, slot.className ? `icon ${slot.className}` : 'icon');
    slot.replaceWith(svg);
  }
}
