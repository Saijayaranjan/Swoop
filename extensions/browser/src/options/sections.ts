/**
 * Options page sections. Each builder returns the section's content; edits go through `patch`,
 * which saves to `browser.storage.local` (SettingsStore) and flashes the "Saved" capsule.
 * Every setting shown here is read by the background: interception rules
 * (src/shared/url-utils.ts), media detection, notifications and the in-page UI.
 */

import browser from 'webextension-polyfill';
import { defaultBrowserSettings, type ExtensionSettings } from '../shared/types.ts';
import { getConnectionStatus } from '../shared/background-client.ts';
import { brandMark } from '../shared-ui/brand.ts';
import { capsule, h, textButton, tr } from '../shared-ui/dom.ts';
import { icon, type IconName } from '../shared-ui/icons.ts';
import { chipEditor, group, note, stepperRow, switchRow, valueRow } from './controls.ts';

export interface SectionContext {
  settings: ExtensionSettings;
  patch(next: Partial<ExtensionSettings>): void;
  /** Re-render the current section (e.g. after a dependent toggle changes). */
  rerender(): void;
}

export interface SectionDef {
  id: string;
  icon: IconName;
  title: () => string;
  subtitle: () => string;
  render(ctx: SectionContext): HTMLElement[] | Promise<HTMLElement[]>;
}

const MB = 1024 * 1024;

function normalizeDomain(raw: string): string | null {
  let value = raw.trim().toLowerCase();
  value = value.replace(/^[a-z]+:\/\//, '').replace(/[/?#].*$/, '').replace(/^\*?\./, '').replace(/:\d+$/, '');
  if (!/^[a-z0-9-]+(\.[a-z0-9-]+)*$/.test(value)) return null;
  if (!value.includes('.') && value !== 'localhost') return null;
  return value;
}

function normalizeExtension(raw: string): string | null {
  const value = raw.trim().toLowerCase().replace(/^\*?\./, '');
  return /^[a-z0-9]{1,10}$/.test(value) ? value : null;
}

function normalizePattern(raw: string): string | null {
  const value = raw.trim();
  return value.length > 0 && !/\s/.test(value) ? value : null;
}

// --- Capture -------------------------------------------------------------------------------

const capture: SectionDef = {
  id: 'capture',
  icon: 'capture',
  title: () => tr('sectionCapture', 'Capture'),
  subtitle: () => tr('sectionCaptureSub', 'Which browser downloads Swoop takes over.'),
  render({ settings, patch, rerender }) {
    const resetTypes = textButton(tr('actionResetDefaults', 'Reset to defaults'), () => {
      patch({ intercept_extensions: defaultBrowserSettings().intercept_extensions });
      rerender();
    }, 'btn quiet');
    return [
      ...group([
        switchRow({
          label: tr('captureDownloads', 'Capture downloads'),
          hint: tr('captureDownloadsHint', 'Send matching downloads to Swoop instead of the browser’s own download list.'),
          checked: settings.intercept_downloads,
          onChange: (checked) => patch({ intercept_downloads: checked }),
        }),
        switchRow({
          label: tr('settingShowConfirmationShort', 'Confirm on the page'),
          hint: tr(
            'settingShowConfirmationHint',
            'Show a small note on the page when a download is sent, with a one-click way to keep it in the browser.',
          ),
          checked: settings.show_confirmation,
          onChange: (checked) => patch({ show_confirmation: checked }),
        }),
      ]),
      ...group(
        [
          stepperRow({
            label: tr('settingMinSizeShort', 'Minimum size'),
            hint: tr('settingMinSizeHint', 'Smaller files stay in the browser. Use 0 to capture every size.'),
            value: Math.round(settings.intercept_min_size / MB),
            unit: 'MB',
            onChange: (mb) => patch({ intercept_min_size: mb * MB }),
          }),
          chipEditor({
            label: tr('settingExtensions', 'File types to capture'),
            hint: tr('settingExtensionsHint', 'Only downloads ending in one of these are captured.'),
            values: settings.intercept_extensions,
            placeholder: tr('addExtensionPlaceholderShort', 'e.g. mkv'),
            emptyText: tr('noFileTypes', 'No file types — nothing will be captured.'),
            mono: true,
            normalize: normalizeExtension,
            onChange: (values) => patch({ intercept_extensions: values }),
            extra: resetTypes,
          }),
        ],
        tr('groupRules', 'Rules'),
      ),
    ];
  },
};

// --- Sites -----------------------------------------------------------------------------------

const sites: SectionDef = {
  id: 'sites',
  icon: 'globe',
  title: () => tr('sectionSites', 'Sites'),
  subtitle: () => tr('sectionSitesSub', 'Where Swoop should leave downloads to the browser.'),
  render({ settings, patch }) {
    return [
      ...group(
        [
          chipEditor({
            label: tr('settingExcludedDomainsShort', 'Never capture from'),
            hint: tr('settingExcludedDomainsHint', 'Includes subdomains: example.com also covers dl.example.com.'),
            values: settings.excluded_domains,
            placeholder: 'example.com',
            emptyText: tr('noSites', 'No sites yet.'),
            normalize: normalizeDomain,
            onChange: (values) => patch({ excluded_domains: values }),
          }),
          chipEditor({
            label: tr('settingAlwaysBrowserShort', 'Always use the browser for'),
            hint: tr(
              'settingAlwaysBrowserHint',
              'Sites you picked “Always use the browser” for on a download prompt. Remove one to capture from it again.',
            ),
            values: settings.always_browser_domains,
            placeholder: 'example.com',
            emptyText: tr('noSites', 'No sites yet.'),
            normalize: normalizeDomain,
            onChange: (values) => patch({ always_browser_domains: values }),
          }),
        ],
        tr('groupSiteLists', 'Site lists'),
      ),
      ...group(
        [
          chipEditor({
            label: tr('settingExcludedPatternsShort', 'Never capture URLs like'),
            hint: tr('urlPatternHintLong', 'Matched against the whole download URL. Use * as a wildcard.'),
            values: settings.excluded_url_patterns,
            placeholder: 'https://*/nightly/*',
            emptyText: tr('noPatterns', 'No patterns yet.'),
            mono: true,
            normalize: normalizePattern,
            onChange: (values) => patch({ excluded_url_patterns: values }),
          }),
        ],
        tr('groupPatterns', 'URL patterns'),
      ),
    ];
  },
};

// --- Media -----------------------------------------------------------------------------------

const media: SectionDef = {
  id: 'media',
  icon: 'media',
  title: () => tr('sectionMedia', 'Media'),
  subtitle: () => tr('sectionMediaSub', 'Finding video, audio and streams on the pages you visit.'),
  render({ settings, patch, rerender }) {
    return [
      ...group([
        switchRow({
          label: tr('settingDetectMedia', 'Detect media on pages'),
          hint: tr('settingDetectMediaHint', 'Notice videos, audio and streams as pages load, so the popup can list them.'),
          checked: settings.detect_media,
          onChange: (checked) => {
            patch({ detect_media: checked });
            rerender();
          },
        }),
        switchRow({
          label: tr('settingMediaButton', 'Show a download button on players'),
          hint: tr('settingMediaButtonHint', 'A small Swoop button appears when you point at a video or audio player.'),
          checked: settings.media_button,
          disabled: !settings.detect_media,
          onChange: (checked) => patch({ media_button: checked }),
        }),
      ]),
      note(
        tr(
          'mediaProtectedNote',
          'Streams protected by DRM, and players that stream from temporary blob: addresses, are listed but can’t be downloaded.',
        ),
        'lock',
      ),
    ];
  },
};

// --- Notifications ---------------------------------------------------------------------------

const notifications: SectionDef = {
  id: 'notifications',
  icon: 'bell',
  title: () => tr('sectionNotifications', 'Notifications'),
  subtitle: () => tr('sectionNotificationsSub', 'System notifications while the popup is closed.'),
  render({ settings, patch }) {
    return [
      ...group([
        switchRow({
          label: tr('settingNotifyComplete', 'When a download completes'),
          checked: settings.notify_on_complete,
          onChange: (checked) => patch({ notify_on_complete: checked }),
        }),
        switchRow({
          label: tr('settingNotifyFailure', 'When a download fails'),
          checked: settings.notify_on_failure,
          onChange: (checked) => patch({ notify_on_failure: checked }),
        }),
      ]),
    ];
  },
};

// --- Shortcuts -------------------------------------------------------------------------------

function keys(combo: string): HTMLElement {
  // "Alt+Shift+D" (Windows/Linux), "⌥⇧D" (macOS) or a single named key like "Return".
  const parts = combo.includes('+') ? combo.split('+') : /^[⌘⌥⇧⌃]/.test(combo) ? [...combo] : [combo];
  return h('kbd', {}, parts.filter(Boolean).map((part) => h('kbd', { text: part })));
}

const shortcuts: SectionDef = {
  id: 'shortcuts',
  icon: 'keyboard',
  title: () => tr('sectionShortcuts', 'Shortcuts'),
  subtitle: () => tr('sectionShortcutsSub', 'Keyboard shortcuts for sending pages and moving around.'),
  async render() {
    let commands: browser.Commands.Command[] = [];
    try {
      commands = await browser.commands.getAll();
    } catch {
      commands = [];
    }
    const commandRows = commands
      .filter((c) => c.name && !c.name.startsWith('_'))
      .map((c) =>
        valueRow(
          c.description || c.name || '',
          c.shortcut ? keys(c.shortcut) : h('span', { class: 'kbd-none', text: tr('shortcutNotSet', 'Not set') }),
        ),
      );

    const commandsApi = browser.commands as unknown as { openShortcutSettings?: () => Promise<void> };
    const target = __SWOOP_TARGET__;
    const change = textButton(tr('actionChangeShortcuts', 'Change shortcuts'), async () => {
      if (commandsApi.openShortcutSettings) {
        await commandsApi.openShortcutSettings();
        return;
      }
      const url = target === 'edge' ? 'edge://extensions/shortcuts' : 'chrome://extensions/shortcuts';
      await browser.tabs.create({ url });
    }, 'btn', 'keyboard');
    const canChange = target !== 'firefox' || commandsApi.openShortcutSettings !== undefined;

    return [
      ...group([
        ...commandRows,
        ...(canChange
          ? [valueRow(tr('shortcutChangeLabel', 'Customise'), change, tr('shortcutChangeHint', 'Pick your own keys in the browser’s shortcut settings.'))]
          : [valueRow(tr('shortcutChangeLabel', 'Customise'), '', tr('shortcutFirefoxHint', 'In Firefox: Add-ons and themes → ⚙ → Manage Extension Shortcuts.'))]),
      ], tr('groupBrowserShortcuts', 'Browser shortcuts')),
      ...group(
        [
          valueRow(tr('shortcutPopupReturn', 'Send the link you pasted'), keys('Return')),
          valueRow(tr('shortcutPopupEsc', 'Close the links list'), keys('Esc')),
          valueRow(tr('shortcutPopupSpace', 'Toggle a switch or a selected link'), keys('Space')),
        ],
        tr('groupInPopup', 'In the popup'),
      ),
      ...group(
        [
          valueRow(tr('shortcutPageEsc', 'Dismiss an Swoop prompt or media panel'), keys('Esc')),
          valueRow(tr('shortcutPageTab', 'Reach the media button of a focused player'), keys('Tab')),
        ],
        tr('groupOnPages', 'On web pages'),
      ),
    ];
  },
};

// --- About -----------------------------------------------------------------------------------

const about: SectionDef = {
  id: 'about',
  icon: 'info',
  title: () => tr('sectionAbout', 'About'),
  subtitle: () => tr('sectionAboutSub', 'Version, privacy and where things live.'),
  async render() {
    const manifest = browser.runtime.getManifest();
    const browserName = { chrome: 'Chrome', edge: 'Edge', firefox: 'Firefox' }[__SWOOP_TARGET__] ?? 'your browser';
    const appValue = h('span', { class: 'value is-muted', text: tr('statusChecking', 'Checking…') });
    void getConnectionStatus()
      .then((status) => {
        if (status.connected && status.running) {
          appValue.textContent = status.version ? `Swoop ${status.version}` : tr('statusConnectedShort', 'Connected');
          appValue.classList.remove('is-muted');
        } else {
          appValue.textContent = tr('statusNotConnected', 'Not connected');
        }
      })
      .catch(() => {
        appValue.textContent = tr('statusNotConnected', 'Not connected');
      });

    const hero = h('div', { class: 'about-hero card' }, [
      brandMark(72),
      h('div', {}, [
        h('h2', { text: tr('aboutTitle', 'Swoop for $1', browserName) }),
        h('p', { text: tr('extensionDescription', 'Send downloads, media and links straight to Swoop.') }),
        capsule(tr('versionLabel', 'Version $1', manifest.version), 'blue'),
      ]),
    ]);

    return [
      hero,
      ...group(
        [
          valueRow(tr('aboutExtensionVersion', 'Extension'), manifest.version),
          valueRow(tr('aboutAppVersion', 'Swoop app'), appValue),
          valueRow(tr('aboutBuild', 'Build'), browserName),
        ],
        tr('groupVersions', 'Versions'),
      ),
      ...group(
        [
          h('ul', { class: 'privacy' }, [
            h('li', {}, [icon('shield'), h('span', { text: tr('privacyLocal', 'Downloads, cookies and page details go only to Swoop — on this computer, or the one you paired.') })]),
            h('li', {}, [icon('shield'), h('span', { text: tr('privacyNoTracking', 'No analytics, tracking or remote code.') })]),
            h('li', {}, [icon('shield'), h('span', { text: tr('privacyStorage', 'Settings are kept in this browser’s extension storage, out of reach of web pages.') })]),
          ]),
        ],
        tr('groupPrivacy', 'Privacy'),
      ),
    ];
  },
};

export const STATIC_SECTIONS: SectionDef[] = [capture, sites, media, notifications, shortcuts, about];
