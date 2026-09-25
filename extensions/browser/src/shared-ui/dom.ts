/** Tiny DOM helpers shared by the popup, options page and in-page UI. Plain DOM, no framework. */

import { t } from '../shared/i18n.ts';
import { icon, type IconName } from './icons.ts';
import { fileKindOf, type FileKind } from './file-kind.ts';

type Child = Node | string | null | undefined | false;

export interface ElementProps {
  class?: string;
  text?: string;
  attrs?: Record<string, string>;
  title?: string;
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: ElementProps = {},
  children: Child[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (props.class) node.className = props.class;
  if (props.text !== undefined) node.textContent = props.text;
  if (props.title) node.title = props.title;
  for (const [key, value] of Object.entries(props.attrs ?? {})) node.setAttribute(key, value);
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child);
  }
  return node;
}

export function iconButton(
  name: IconName,
  label: string,
  onClick: (event: MouseEvent) => void | Promise<void>,
  className = 'icon-btn',
): HTMLButtonElement {
  const btn = h('button', { class: className, attrs: { type: 'button', 'aria-label': label }, title: label }, [
    icon(name),
  ]);
  btn.addEventListener('click', (event) => {
    void onClick(event);
  });
  return btn;
}

export function textButton(
  label: string,
  onClick: (event: MouseEvent) => void | Promise<void>,
  className = 'btn',
  iconName?: IconName,
): HTMLButtonElement {
  const btn = h('button', { class: className, attrs: { type: 'button' } }, [
    iconName ? icon(iconName) : null,
    h('span', { text: label }),
  ]);
  btn.addEventListener('click', (event) => {
    void onClick(event);
  });
  return btn;
}

export function fileTile(kind: FileKind): HTMLSpanElement {
  return h('span', { class: `tile t-${kind.tint}`, attrs: { 'aria-hidden': 'true' } }, [icon(kind.icon)]);
}

export function fileTileFor(name: string, taskKind?: string): HTMLSpanElement {
  return fileTile(fileKindOf(name, taskKind));
}

export type StatusColor = 'blue' | 'green' | 'orange' | 'red' | 'grey';

export function capsule(text: string, color: StatusColor, iconName?: IconName): HTMLSpanElement {
  return h('span', { class: `capsule is-${color}` }, [iconName ? icon(iconName) : null, h('span', { text })]);
}

/** Slim progress bar. `fraction === null` draws an indeterminate bar. */
export function progressBar(fraction: number | null, color: StatusColor, label: string): HTMLDivElement {
  const fill = h('span');
  const bar = h('div', {
    class: `bar is-${color}${fraction === null ? ' is-indeterminate' : ''}`,
    attrs: { role: 'progressbar', 'aria-label': label, 'aria-valuemin': '0', 'aria-valuemax': '100' },
  }, [fill]);
  setProgress(bar, fraction);
  return bar;
}

export function setProgress(bar: HTMLElement, fraction: number | null): void {
  const fill = bar.firstElementChild as HTMLElement | null;
  if (fraction === null) {
    bar.removeAttribute('aria-valuenow');
    return;
  }
  const pct = Math.max(0, Math.min(100, fraction * 100));
  bar.setAttribute('aria-valuenow', String(Math.round(pct)));
  if (fill) fill.style.width = `${pct}%`;
}

/**
 * Localises static markup: `data-i18n` (text), `data-i18n-aria` (aria-label),
 * `data-i18n-title` (tooltip) and `data-i18n-placeholder`. Keys that resolve to nothing keep the
 * markup's English fallback.
 */
export function applyI18n(root: ParentNode = document): void {
  const resolve = (key: string | undefined): string | null => {
    if (!key) return null;
    const value = t(key);
    return value && value !== key ? value : null;
  };
  for (const node of root.querySelectorAll<HTMLElement>('[data-i18n]')) {
    const value = resolve(node.dataset['i18n']);
    if (value) node.textContent = value;
  }
  for (const node of root.querySelectorAll<HTMLElement>('[data-i18n-aria]')) {
    const value = resolve(node.dataset['i18nAria']);
    if (value) node.setAttribute('aria-label', value);
  }
  for (const node of root.querySelectorAll<HTMLElement>('[data-i18n-title]')) {
    const value = resolve(node.dataset['i18nTitle']);
    if (value) node.title = value;
  }
  for (const node of root.querySelectorAll<HTMLInputElement>('[data-i18n-placeholder]')) {
    const value = resolve(node.dataset['i18nPlaceholder']);
    if (value) node.placeholder = value;
  }
  const lang = t('localeCode');
  if (lang && lang !== 'localeCode') document.documentElement.lang = lang;
}

/** `t()` with an English fallback for keys missing from the running build. */
export function tr(key: string, fallback: string, substitutions?: string | string[]): string {
  const value = t(key, substitutions);
  if (value && value !== key) return value;
  const subs = substitutions === undefined ? [] : Array.isArray(substitutions) ? substitutions : [substitutions];
  return fallback.replace(/\$(\d)/g, (_match, n: string) => subs[Number(n) - 1] ?? '');
}
