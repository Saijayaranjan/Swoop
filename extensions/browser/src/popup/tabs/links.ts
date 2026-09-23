/**
 * "Links on page" tab: the content script's `<a href>` scan, plus a pre-loaded set when opened
 * via the "Download all links on page…" / "…in selection" context menu actions
 * (`showPending`, fed by `state-bulk.ts`).
 */

import browser from 'webextension-polyfill';
import { t } from '../../shared/i18n.ts';
import { hostnameOf } from '../../shared/url-utils.ts';
import { quickDownload } from '../../shared/background-client.ts';
import type { CandidateLink, ScanResultMessage } from '../../shared/messages.ts';
import type { AddTaskResult, NewTaskRequest } from '../../shared/types.ts';
import { renderEmptyState, renderItemRow, renderList } from '../render-utils.ts';

export interface LinksTabHandle {
  refresh(): Promise<void>;
  showPending(links: CandidateLink[]): void;
}

export function initLinksTab(
  container: HTMLElement,
  tabId: number | null,
  showToast: (message: string) => void,
): LinksTabHandle {
  let pageUrl = '';

  function renderLinks(links: CandidateLink[]): void {
    if (links.length === 0) {
      renderEmptyState(container, t('noLinksFound') || 'No links found on this page.');
      return;
    }
    const rows = links.map((link) => {
      const domain = hostnameOf(link.url) ?? '';
      const request = (start: boolean): NewTaskRequest => ({
        url: link.url,
        start,
        origin: 'browser',
        referer_page: pageUrl || undefined,
      });
      return renderItemRow({
        title: link.text,
        meta: [domain].filter(Boolean),
        badges: link.looksDownloadable ? [{ text: t('likelyDownload') || 'Likely download' }] : [],
        actions: [
          {
            label: t('actionDownload') || 'Download',
            primary: true,
            onClick: async () => {
              try {
                await quickDownload<AddTaskResult>(request(true));
                showToast(t('toastDownloadStarted') || 'Download started');
              } catch (err) {
                showToast(err instanceof Error ? err.message : String(err));
              }
            },
          },
          {
            label: t('actionQueue') || 'Queue',
            onClick: async () => {
              try {
                await quickDownload<AddTaskResult>(request(false));
                showToast(t('toastQueued') || 'Queued');
              } catch (err) {
                showToast(err instanceof Error ? err.message : String(err));
              }
            },
          },
          {
            label: t('actionCopyUrl') || 'Copy URL',
            onClick: async () => {
              await navigator.clipboard.writeText(link.url);
              showToast(t('toastCopied') || 'Copied');
            },
          },
          {
            label: t('actionOpenSource') || 'Open source',
            onClick: async () => {
              await browser.tabs.create({ url: link.url });
            },
          },
        ],
      });
    });
    renderList(container, rows);
  }

  async function refresh(): Promise<void> {
    if (tabId === null) {
      renderEmptyState(container, t('noActiveTab') || 'No active tab.');
      return;
    }
    renderEmptyState(container, t('loading') || 'Loading…');
    try {
      const result = (await browser.tabs.sendMessage(tabId, { type: 'scan' })) as
        | ScanResultMessage
        | undefined;
      pageUrl = result?.pageUrl ?? '';
      renderLinks(result?.links ?? []);
    } catch {
      renderEmptyState(container, t('scanFailed') || 'Could not scan this page.');
    }
  }

  function showPending(links: CandidateLink[]): void {
    renderLinks(links);
  }

  return { refresh, showPending };
}
