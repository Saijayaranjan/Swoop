/** Popup "Settings" tab: mounts the shared settings form against `browser.storage.local`. */

import browser from 'webextension-polyfill';
import { SettingsStore } from '../../shared/settings.ts';
import { mountSettingsForm } from '../../shared-ui/settings-form.ts';

export function initSettingsTab(container: HTMLElement): { store: SettingsStore } {
  const store = new SettingsStore(browser.storage.local);
  void mountSettingsForm(container, store);
  return { store };
}
