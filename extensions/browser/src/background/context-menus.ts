/**
 * Context menus (docs/api/extension.md "Context menus"): link, image, video, audio, page
 * ("Download all links on page…"), and selection ("links in selection"). Single-target clicks
 * (link/image/video/audio) go straight to `POST /tasks`; the two bulk actions gather candidate
 * links from the content script and open the popup's links picker pre-loaded with them (see
 * `pendingBulkLinks` in `state-bulk.ts` and `src/popup/links.ts`).
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './native-port.ts';
import type { NewTaskRequest } from '../shared/types.ts';
import { setPendingBulkLinks } from './state-bulk.ts';
import type { CandidateLink, ScanResultMessage } from '../shared/messages.ts';

const MENU_LINK = 'swoop-download-link';
const MENU_IMAGE = 'swoop-download-image';
const MENU_VIDEO = 'swoop-download-video';
const MENU_AUDIO = 'swoop-download-audio';
const MENU_PAGE = 'swoop-download-page-links';
const MENU_SELECTION = 'swoop-download-selection-links';

export function createContextMenus(): void {
  browser.contextMenus.removeAll().then(() => {
    browser.contextMenus.create({
      id: MENU_LINK,
      contexts: ['link'],
      title: browser.i18n.getMessage('menuDownloadLink') || 'Download with Swoop',
    });
    browser.contextMenus.create({
      id: MENU_IMAGE,
      contexts: ['image'],
      title: browser.i18n.getMessage('menuDownloadImage') || 'Download image with Swoop',
    });
    browser.contextMenus.create({
      id: MENU_VIDEO,
      contexts: ['video'],
      title: browser.i18n.getMessage('menuDownloadVideo') || 'Download video with Swoop',
    });
    browser.contextMenus.create({
      id: MENU_AUDIO,
      contexts: ['audio'],
      title: browser.i18n.getMessage('menuDownloadAudio') || 'Download audio with Swoop',
    });
    browser.contextMenus.create({
      id: MENU_PAGE,
      contexts: ['page'],
      title: browser.i18n.getMessage('menuDownloadAllLinks') || 'Download all links on page…',
    });
    browser.contextMenus.create({
      id: MENU_SELECTION,
      contexts: ['selection'],
      title: browser.i18n.getMessage('menuDownloadSelectionLinks') || 'Download links in selection',
    });
  }).catch(() => {});
}

async function addTask(nativePort: NativePort, request: NewTaskRequest): Promise<void> {
  await nativePort.post('/api/v1/tasks', request);
}

async function openBulkPicker(
  tabId: number,
  scanType: 'scan' | 'scan-selection',
): Promise<void> {
  let links: CandidateLink[] = [];
  try {
    const response = (await browser.tabs.sendMessage(tabId, { type: scanType })) as
      | ScanResultMessage
      | undefined;
    links = response?.links ?? [];
  } catch {
    links = [];
  }
  await setPendingBulkLinks(links);
  try {
    await browser.action.openPopup();
  } catch {
    await browser.tabs.create({ url: browser.runtime.getURL('popup/index.html?tab=links') });
  }
}

export function registerContextMenuHandlers(nativePort: NativePort): void {
  browser.contextMenus.onClicked.addListener((info, tab) => {
    void handleClick(info, tab, nativePort);
  });
}

async function handleClick(
  info: browser.Menus.OnClickData,
  tab: browser.Tabs.Tab | undefined,
  nativePort: NativePort,
): Promise<void> {
  const refererPage = tab?.url;
  switch (info.menuItemId) {
    case MENU_LINK:
      if (info.linkUrl) {
        await addTask(nativePort, {
          url: info.linkUrl,
          start: true,
          origin: 'browser',
          referer_page: refererPage,
        });
      }
      break;
    case MENU_IMAGE:
    case MENU_VIDEO:
    case MENU_AUDIO:
      if (info.srcUrl) {
        await addTask(nativePort, {
          url: info.srcUrl,
          start: true,
          origin: 'browser',
          referer_page: refererPage,
        });
      }
      break;
    case MENU_PAGE:
      if (tab?.id !== undefined) await openBulkPicker(tab.id, 'scan');
      break;
    case MENU_SELECTION:
      if (tab?.id !== undefined) await openBulkPicker(tab.id, 'scan-selection');
      break;
    default:
      break;
  }
}
