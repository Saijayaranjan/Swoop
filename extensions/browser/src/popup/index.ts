/**
 * Popup entry point: tab switching (keyboard-accessible per WAI-ARIA tabs pattern), connection
 * status, and wiring each tab module to the background (direct `tabs.sendMessage` to the content
 * script for on-page scans, `runtime.sendMessage`/the long-lived port for everything else).
 */

import browser from 'webextension-polyfill';
import { t } from '../shared/i18n.ts';
import { POPUP_PORT_NAME, type PopupPortMessage } from '../shared/messages.ts';
import {
  getActiveTabId,
  getConnectionStatus,
  launchApp,
  takePendingBulkLinks,
} from '../shared/background-client.ts';
import { initDetectedMediaTab } from './tabs/detected-media.ts';
import { initLinksTab } from './tabs/links.ts';
import { initDownloadsTab } from './tabs/downloads.ts';
import { initSettingsTab } from './tabs/settings-tab.ts';
import type { OspreyEvent } from '../shared/types.ts';

const TAB_NAMES = ['media', 'links', 'downloads', 'settings'] as const;
type TabName = (typeof TAB_NAMES)[number];

function requireEl(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

let toastTimer: ReturnType<typeof setTimeout> | undefined;
function showToast(message: string): void {
  const toast = requireEl('toast');
  toast.textContent = message;
  toast.hidden = false;
  if (toastTimer !== undefined) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.hidden = true;
  }, 3000);
}

function setActiveTab(name: TabName): void {
  for (const tab of TAB_NAMES) {
    const tabBtn = requireEl(`tab-${tab}`);
    const panel = requireEl(`panel-${tab}`);
    const active = tab === name;
    tabBtn.setAttribute('aria-selected', String(active));
    tabBtn.tabIndex = active ? 0 : -1;
    panel.hidden = !active;
  }
}

function currentTab(): TabName {
  return TAB_NAMES.find((tab) => requireEl(`tab-${tab}`).getAttribute('aria-selected') === 'true') ?? 'media';
}

function wireTabs(onChange: (name: TabName) => void): void {
  const tablist = document.querySelector('[role="tablist"]');
  for (const tab of TAB_NAMES) {
    requireEl(`tab-${tab}`).addEventListener('click', () => {
      setActiveTab(tab);
      onChange(tab);
    });
  }
  tablist?.addEventListener('keydown', (event) => {
    const key = (event as KeyboardEvent).key;
    if (key !== 'ArrowRight' && key !== 'ArrowLeft' && key !== 'Home' && key !== 'End') return;
    event.preventDefault();
    const currentIndex = TAB_NAMES.indexOf(currentTab());
    let nextIndex = currentIndex;
    if (key === 'ArrowRight') nextIndex = (currentIndex + 1) % TAB_NAMES.length;
    if (key === 'ArrowLeft') nextIndex = (currentIndex - 1 + TAB_NAMES.length) % TAB_NAMES.length;
    if (key === 'Home') nextIndex = 0;
    if (key === 'End') nextIndex = TAB_NAMES.length - 1;
    const next = TAB_NAMES[nextIndex] as TabName;
    setActiveTab(next);
    requireEl(`tab-${next}`).focus();
    onChange(next);
  });
}

function applyStaticI18n(): void {
  document.title = t('extensionName') || 'Osprey';
  for (const node of document.querySelectorAll<HTMLElement>('[data-i18n]')) {
    const key = node.dataset['i18n'];
    if (key) node.textContent = t(key);
  }
}

async function refreshStatus(): Promise<void> {
  const statusEl = requireEl('status');
  statusEl.replaceChildren();
  try {
    const status = await getConnectionStatus();
    if (status.connected && status.running) {
      statusEl.textContent = status.version
        ? `${t('statusConnectedPrefix') || 'Osprey'} ${status.version} · ${t('statusConnected') || 'connected'}`
        : t('statusConnected') || 'connected';
      statusEl.classList.add('status-online');
      statusEl.classList.remove('status-offline');
      return;
    }
    statusEl.classList.add('status-offline');
    statusEl.classList.remove('status-online');
    const label = document.createElement('span');
    label.textContent = `${t('statusNotRunning') || 'Osprey not running'} — `;
    statusEl.appendChild(label);
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'link-button';
    btn.textContent = t('actionLaunch') || 'Launch';
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      try {
        await launchApp();
        showToast(t('toastLaunching') || 'Launching Osprey…');
        setTimeout(() => void refreshStatus(), 2000);
      } catch (err) {
        showToast(err instanceof Error ? err.message : String(err));
      } finally {
        btn.disabled = false;
      }
    });
    statusEl.appendChild(btn);
  } catch (err) {
    statusEl.textContent = err instanceof Error ? err.message : String(err);
  }
}

async function main(): Promise<void> {
  applyStaticI18n();

  const tabId = await getActiveTabId();

  const media = initDetectedMediaTab(requireEl('panel-media'), tabId, showToast);
  const links = initLinksTab(requireEl('panel-links'), tabId, showToast);
  const downloads = initDownloadsTab(requireEl('panel-downloads'), showToast);
  initSettingsTab(requireEl('panel-settings'));

  wireTabs((name) => {
    if (name === 'media') void media.refresh();
    if (name === 'links') void links.refresh();
    if (name === 'downloads') void downloads.refresh();
  });

  setActiveTab('media');
  void media.refresh();
  void downloads.refresh();
  void refreshStatus();

  const pending = await takePendingBulkLinks();
  if (pending.length > 0) {
    setActiveTab('links');
    links.showPending(pending);
  }

  const port = browser.runtime.connect({ name: POPUP_PORT_NAME });
  port.onMessage.addListener((raw) => {
    const message = raw as PopupPortMessage;
    if (message.type === 'connection-status') {
      void refreshStatus();
    } else if (message.type === 'event') {
      downloads.handleEvent(message.event as OspreyEvent);
    }
  });
}

document.addEventListener('DOMContentLoaded', () => {
  void main();
});
