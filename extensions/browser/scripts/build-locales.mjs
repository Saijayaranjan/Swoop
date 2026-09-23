#!/usr/bin/env node
// Generates `_locales/<lang>/messages.json` from one source of truth so `en`, `hi` and `ta`
// never drift out of key-sync. `hi`/`ta` are scaffolds: same keys, English text, each message's
// `description` marked `[fallback: needs hi/ta translation]` per the deliverable spec.

import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, '..');

/** @type {Record<string, {message: string, description?: string, placeholders?: Record<string, {content: string}>}>} */
const en = {
  extensionName: { message: 'Osprey' },
  extensionDescription: {
    message: 'Send downloads, media and links straight to the Osprey download manager.',
  },
  commandSendPageUrl: { message: "Send the current page's URL to Osprey" },

  tabDetectedMedia: { message: 'Detected media' },
  tabLinks: { message: 'Links on page' },
  tabDownloads: { message: 'Downloads' },
  tabSettings: { message: 'Settings' },
  optionsSubtitle: { message: 'Browser integration settings' },

  statusConnectedPrefix: { message: 'Osprey' },
  statusConnected: { message: 'connected' },
  statusNotRunning: { message: 'Osprey not running' },

  actionDownload: { message: 'Download' },
  actionQueue: { message: 'Queue' },
  actionCopyUrl: { message: 'Copy URL' },
  actionOpenSource: { message: 'Open source' },
  actionPause: { message: 'Pause' },
  actionResume: { message: 'Resume' },
  actionCancel: { message: 'Cancel' },
  actionLaunch: { message: 'Launch' },

  toastDownloadStarted: { message: 'Download started' },
  toastQueued: { message: 'Queued' },
  toastCopied: { message: 'Copied' },
  toastLaunching: { message: 'Launching Osprey…' },

  loading: { message: 'Loading…' },
  loadFailed: { message: 'Could not load downloads.' },
  scanFailed: { message: 'Could not scan this page.' },
  noActiveTab: { message: 'No active tab.' },
  noMediaDetected: { message: 'No media detected on this page yet.' },
  noLinksFound: { message: 'No links found on this page.' },
  noActiveDownloads: { message: 'No active downloads.' },
  notDownloadable: { message: 'Not downloadable' },
  likelyDownload: { message: 'Likely download' },
  quality: { message: 'Quality' },
  eta: { message: 'ETA' },
  remove: { message: 'Remove' },

  mediaKindVideo: { message: 'Video' },
  mediaKindAudio: { message: 'Audio' },
  mediaKindImage: { message: 'Image' },
  mediaKindHls: { message: 'HLS' },
  mediaKindDash: { message: 'DASH' },
  mediaKindUnknown: { message: 'Media' },

  menuDownloadLink: { message: 'Download with Osprey' },
  menuDownloadImage: { message: 'Download image with Osprey' },
  menuDownloadVideo: { message: 'Download video with Osprey' },
  menuDownloadAudio: { message: 'Download audio with Osprey' },
  menuDownloadAllLinks: { message: 'Download all links on page…' },
  menuDownloadSelectionLinks: { message: 'Download links in selection' },

  notificationCompletedTitle: { message: 'Download complete' },
  notificationFailedTitle: { message: 'Download failed' },

  settingInterceptDownloads: { message: 'Intercept browser downloads' },
  settingMinSize: { message: 'Minimum size to intercept (MB)' },
  settingExtensions: { message: 'File extensions to intercept' },
  settingExcludedDomains: { message: 'Never intercept on these domains' },
  settingExcludedPatterns: { message: 'Never intercept matching URL patterns' },
  settingAlwaysBrowser: { message: "Always use the browser's own download on these sites" },
  settingDetectMedia: { message: 'Detect playable media on pages' },
  settingShowConfirmation: { message: 'Show a confirmation toast when a download is sent' },
  settingNotifyComplete: { message: 'Notify when a download completes' },
  settingNotifyFailure: { message: 'Notify when a download fails' },
  addExtensionPlaceholder: { message: 'Add extension, press Enter' },
  addDomainPlaceholder: { message: 'example.com, press Enter' },
  addPatternPlaceholder: { message: 'https://*/ads/*, press Enter' },
  urlPatternHint: { message: 'Wildcards (*) allowed.' },

  saving: { message: 'Saving…' },
  saved: { message: 'Saved' },
  saveFailed: { message: 'Save failed' },
};

function writeLocale(lang, transform) {
  const dir = path.join(ROOT, '_locales', lang);
  mkdirSync(dir, { recursive: true });
  /** @type {Record<string, {message: string, description?: string}>} */
  const out = {};
  for (const [key, entry] of Object.entries(en)) {
    out[key] = transform(key, entry);
  }
  writeFileSync(path.join(dir, 'messages.json'), `${JSON.stringify(out, null, 2)}\n`);
  console.log(`wrote _locales/${lang}/messages.json (${Object.keys(out).length} keys)`);
}

writeLocale('en', (_key, entry) => entry);

// Scaffolds: identical keys, English fallback text, clearly marked as untranslated.
for (const lang of ['hi', 'ta']) {
  writeLocale(lang, (_key, entry) => ({
    message: entry.message,
    description: '[fallback: needs translation] ' + (entry.description ?? ''),
  }));
}
