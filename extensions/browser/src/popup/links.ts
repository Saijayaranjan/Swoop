/**
 * "Links on this page" sheet: the content script's `<a href>` scan, or a pre-loaded set when the
 * popup was opened by the "Download all links on page…" / "…in selection" context-menu actions
 * (`show(pending)`, fed by `state-bulk.ts`). Pick links, then Download (start now) or Queue.
 */

import browser from 'webextension-polyfill';
import { hostnameOf } from '../shared/url-utils.ts';
import { quickDownload } from '../shared/background-client.ts';
import type { CandidateLink, ScanResultMessage } from '../shared/messages.ts';
import type { AddTaskResult } from '../shared/types.ts';
import { fileTileFor, h, tr } from '../shared-ui/dom.ts';
import { extensionOf } from '../shared-ui/file-kind.ts';
import { errorMessage, type PopupContext } from './context.ts';

export interface LinksHandle {
  open(pending?: CandidateLink[]): Promise<void>;
  close(): void;
}

export function initLinks(ctx: PopupContext): LinksHandle {
  const sheet = document.getElementById('links-view') as HTMLElement;
  const body = document.getElementById('links-body') as HTMLElement;
  const count = document.getElementById('links-count') as HTMLElement;
  const selectAll = document.getElementById('links-all') as HTMLInputElement;
  const likelyOnly = document.getElementById('links-likely') as HTMLInputElement;
  const selectedNote = document.getElementById('links-selected') as HTMLElement;
  const downloadBtn = document.getElementById('links-download') as HTMLButtonElement;
  const downloadLabel = document.getElementById('links-download-label') as HTMLElement;
  const queueBtn = document.getElementById('links-queue') as HTMLButtonElement;
  const backBtn = document.getElementById('links-back') as HTMLButtonElement;
  let returnFocus: HTMLElement | null = null;

  let links: CandidateLink[] = [];
  let pageUrl = '';
  const selected = new Set<string>();

  function visibleLinks(): CandidateLink[] {
    return likelyOnly.checked ? links.filter((l) => l.looksDownloadable) : links;
  }

  function updateFooter(): void {
    const visible = visibleLinks();
    const chosen = visible.filter((l) => selected.has(l.url)).length;
    selectedNote.textContent = tr('selectedCount', '$1 of $2 selected', [String(chosen), String(visible.length)]);
    downloadLabel.textContent = chosen > 0 ? tr('downloadCount', 'Download $1', String(chosen)) : tr('actionDownload', 'Download');
    downloadBtn.disabled = chosen === 0;
    queueBtn.disabled = chosen === 0;
    selectAll.checked = visible.length > 0 && chosen === visible.length;
    selectAll.indeterminate = chosen > 0 && chosen < visible.length;
  }

  function render(): void {
    const visible = visibleLinks();
    count.textContent = String(links.length);
    if (visible.length === 0) {
      body.replaceChildren(
        h('div', { class: 'empty' }, [
          h('h3', { text: links.length === 0 ? tr('noLinksFound', 'No links found on this page.') : tr('noLikelyLinks', 'No obvious downloads here') }),
          h('p', {
            text: links.length === 0 ? '' : tr('noLikelyLinksBody', 'Turn off “Likely downloads” to see every link.'),
          }),
        ]),
      );
      updateFooter();
      return;
    }
    const rows = visible.map((link) => {
      const input = h('input', { attrs: { type: 'checkbox' } });
      input.checked = selected.has(link.url);
      input.addEventListener('change', () => {
        if (input.checked) selected.add(link.url);
        else selected.delete(link.url);
        updateFooter();
      });
      const ext = extensionOf(link.url);
      const meta = [hostnameOf(link.url), ext ? ext.toUpperCase() : null].filter(Boolean).join(' · ');
      return h('label', { class: 'link-row', title: link.url }, [
        input,
        fileTileFor(link.url),
        h('span', { class: 'link-text' }, [
          h('span', { class: 'row-title', text: link.text }),
          h('span', { class: 'row-meta', text: meta }),
        ]),
      ]);
    });
    body.replaceChildren(...rows);
    updateFooter();
  }

  function setLinks(next: CandidateLink[]): void {
    links = next;
    selected.clear();
    const likely = next.filter((l) => l.looksDownloadable);
    likelyOnly.checked = likely.length > 0;
    for (const link of likely) selected.add(link.url);
    render();
  }

  async function send(start: boolean): Promise<void> {
    const chosen = visibleLinks().filter((l) => selected.has(l.url));
    if (chosen.length === 0) return;
    downloadBtn.disabled = true;
    queueBtn.disabled = true;
    const results = await Promise.allSettled(
      chosen.map((link) =>
        quickDownload<AddTaskResult>({ url: link.url, start, origin: 'browser', referer_page: pageUrl || undefined }),
      ),
    );
    const failed = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
    if (failed.length === 0) {
      ctx.toast(tr('toastLinksSent', 'Sent $1 to Osprey', String(chosen.length)));
      close();
    } else {
      ctx.toast(errorMessage(failed[0]?.reason), 'error');
      updateFooter();
    }
  }

  selectAll.addEventListener('change', () => {
    for (const link of visibleLinks()) {
      if (selectAll.checked) selected.add(link.url);
      else selected.delete(link.url);
    }
    render();
  });
  likelyOnly.addEventListener('change', render);
  downloadBtn.addEventListener('click', () => void send(true));
  queueBtn.addEventListener('click', () => void send(false));
  backBtn.addEventListener('click', () => close());
  sheet.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    }
  });

  async function open(pending?: CandidateLink[]): Promise<void> {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    sheet.hidden = false;
    backBtn.focus();
    if (pending && pending.length > 0) {
      setLinks(pending);
      return;
    }
    body.replaceChildren(h('div', { class: 'skeleton' }), h('div', { class: 'skeleton' }));
    if (ctx.tabId === null) {
      setLinks([]);
      return;
    }
    try {
      const result = (await browser.tabs.sendMessage(ctx.tabId, { type: 'scan' })) as ScanResultMessage | undefined;
      pageUrl = result?.pageUrl ?? '';
      setLinks(result?.links ?? []);
    } catch {
      body.replaceChildren(h('div', { class: 'empty' }, [h('h3', { text: tr('scanFailed', 'Could not scan this page.') })]));
      count.textContent = '';
    }
  }

  function close(): void {
    sheet.hidden = true;
    returnFocus?.focus();
  }

  return { open, close };
}
