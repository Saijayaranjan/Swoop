/** Options page entry: the same settings form as the popup's Settings tab, full-page. */

import browser from 'webextension-polyfill';
import { SettingsStore } from '../shared/settings.ts';
import { mountSettingsForm } from '../shared-ui/settings-form.ts';
import { t } from '../shared/i18n.ts';

function applyStaticI18n(): void {
  document.title = `${t('extensionName') || 'Osprey'} – ${t('optionsSubtitle') || 'Settings'}`;
  for (const node of document.querySelectorAll<HTMLElement>('[data-i18n]')) {
    const key = node.dataset['i18n'];
    if (key) node.textContent = t(key);
  }
}

async function main(): Promise<void> {
  applyStaticI18n();
  const root = document.getElementById('settings-root');
  if (!root) return;
  const store = new SettingsStore(browser.storage.local);
  await mountSettingsForm(root, store);
}

document.addEventListener('DOMContentLoaded', () => {
  void main();
});
