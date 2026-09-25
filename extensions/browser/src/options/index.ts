/**
 * Options page: a sidebar of sections (Capture, Sites, Media, Notifications, Connection,
 * Shortcuts, About) and a floating panel showing one at a time. The section lives in the URL hash
 * (`#connection`), so the popup can deep-link to it. Every change saves immediately.
 */

import browser from 'webextension-polyfill';
import { SettingsStore } from '../shared/settings.ts';
import type { ExtensionSettings } from '../shared/types.ts';
import { getConnectionStatus } from '../shared/background-client.ts';
import type { ConnectionStatus } from '../shared/messages.ts';
import { brandMark } from '../shared-ui/brand.ts';
import { applyI18n, h, tr } from '../shared-ui/dom.ts';
import { hydrateIcons, icon } from '../shared-ui/icons.ts';
import { connectionDot, connectionSection } from './connection.ts';
import { STATIC_SECTIONS, type SectionDef } from './sections.ts';

function byId<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

async function main(): Promise<void> {
  applyI18n();
  hydrateIcons(document);
  byId('brand-mark').replaceWith(brandMark(40));

  const store = new SettingsStore(browser.storage.local);
  let settings: ExtensionSettings = await store.get();

  const dot = byId('nav-connection-dot');
  const onStatus = (status: ConnectionStatus | null): void => {
    dot.className = `nav-dot ${connectionDot(status)}`;
  };
  void getConnectionStatus().then(onStatus, () => onStatus(null));

  const [capture, sites, media, notifications, shortcuts, about] = STATIC_SECTIONS as [
    SectionDef, SectionDef, SectionDef, SectionDef, SectionDef, SectionDef,
  ];
  const sections: SectionDef[] = [capture, sites, media, notifications, connectionSection(onStatus), shortcuts, about];

  const title = byId('section-title');
  const sub = byId('section-sub');
  const headIcon = byId('section-icon');
  const body = byId('section-body');
  const saveStatus = byId('save-status');
  const panel = byId('panel');

  // --- saving -------------------------------------------------------------------------------
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let pending: Partial<ExtensionSettings> = {};

  function flash(ok: boolean): void {
    saveStatus.replaceChildren(icon(ok ? 'check' : 'warning'), h('span', { text: ok ? tr('saved', 'Saved') : tr('saveFailed', 'Save failed') }));
    saveStatus.classList.toggle('is-error', !ok);
    saveStatus.classList.add('is-visible');
    if (hideTimer !== undefined) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => saveStatus.classList.remove('is-visible'), 1800);
  }

  function patch(next: Partial<ExtensionSettings>): void {
    settings = { ...settings, ...next };
    pending = { ...pending, ...next };
    if (saveTimer !== undefined) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      const toSave = pending;
      pending = {};
      store
        .update(toSave)
        .then((saved) => {
          settings = { ...saved, ...pending };
          flash(true);
        })
        .catch(() => flash(false));
    }, 200);
  }

  // --- routing ------------------------------------------------------------------------------
  let current: SectionDef = sections[0] as SectionDef;
  let renderToken = 0;

  async function render(focusPanel: boolean): Promise<void> {
    const token = ++renderToken;
    document.title = `${current.title()} · ${tr('extensionName', 'Osprey')}`;
    title.textContent = current.title();
    sub.textContent = current.subtitle();
    headIcon.replaceChildren(icon(current.icon));
    for (const link of document.querySelectorAll<HTMLAnchorElement>('#nav a')) {
      if (link.dataset['section'] === current.id) link.setAttribute('aria-current', 'page');
      else link.removeAttribute('aria-current');
    }
    const content = await current.render({ settings, patch, rerender: () => void render(false) });
    if (token !== renderToken) return;
    const scrollY = window.scrollY;
    body.replaceChildren(...content);
    if (focusPanel) {
      panel.focus({ preventScroll: true });
      window.scrollTo({ top: 0 });
    } else {
      window.scrollTo({ top: scrollY });
    }
  }

  function route(focusPanel: boolean): void {
    const id = location.hash.replace(/^#/, '');
    current = sections.find((s) => s.id === id) ?? (sections[0] as SectionDef);
    void render(focusPanel);
  }

  window.addEventListener('hashchange', () => route(true));

  // Reflect changes made elsewhere (e.g. the popup's Capture switch) unless the user is typing.
  store.onChange((next) => {
    const changed = JSON.stringify(next) !== JSON.stringify({ ...settings, ...pending });
    settings = { ...next, ...pending };
    const typing = document.activeElement instanceof HTMLInputElement && document.activeElement.type !== 'checkbox';
    if (changed && !typing) void render(false);
  });

  route(false);
}

document.addEventListener('DOMContentLoaded', () => {
  void main();
});
