/**
 * Connection status: the header pill (Connected / Not running / Offline) and, when Swoop can't
 * be used, a friendly state card in place of the downloads list with the one action that fixes it
 * (launch the app, finish setting up the browser helper, or check the remote connection).
 */

import browser from 'webextension-polyfill';
import { getConnectionStatus, launchApp } from '../shared/background-client.ts';
import type { ConnectionStatus } from '../shared/messages.ts';
import { featherIllustration } from '../shared-ui/brand.ts';
import { h, textButton, tr } from '../shared-ui/dom.ts';
import { errorMessage, type PopupContext } from './context.ts';

export type ConnectionState = 'checking' | 'online' | 'not-running' | 'no-host' | 'remote-unreachable' | 'remote-refused';

export function connectionState(status: ConnectionStatus | null): ConnectionState {
  if (!status) return 'checking';
  if (status.connected && status.running) return 'online';
  if (status.mode === 'remote') return status.connected ? 'remote-refused' : 'remote-unreachable';
  return status.connected ? 'not-running' : 'no-host';
}

export function openOptions(section?: string): void {
  if (!section) {
    void browser.runtime.openOptionsPage();
    return;
  }
  void browser.tabs.create({ url: browser.runtime.getURL(`options/index.html#${section}`) });
}

export interface StatusHandle {
  refresh(): Promise<ConnectionStatus | null>;
  apply(status: ConnectionStatus): void;
}

export function initStatus(
  ctx: PopupContext,
  onChange: (state: ConnectionState, status: ConnectionStatus | null) => void,
): StatusHandle {
  const pill = document.getElementById('status') as HTMLElement;
  const card = document.getElementById('offline') as HTMLElement;
  let lastState: ConnectionState | null = null;

  async function launch(button?: HTMLButtonElement): Promise<void> {
    if (button) button.disabled = true;
    try {
      await launchApp();
      ctx.toast(tr('toastLaunching', 'Launching Swoop…'));
      // The app takes a moment to come up; check a few times.
      for (const delay of [1500, 3000, 5000]) {
        setTimeout(() => void refresh(), delay);
      }
    } catch (err) {
      ctx.toast(errorMessage(err), 'error');
    } finally {
      if (button) setTimeout(() => (button.disabled = false), 1500);
    }
  }

  function renderPill(state: ConnectionState, status: ConnectionStatus | null): void {
    pill.className = 'status-pill';
    const text = h('span', { class: 'status-text' });
    const children: Node[] = [h('span', { class: 'dot', attrs: { 'aria-hidden': 'true' } }), text];
    pill.removeAttribute('title');
    switch (state) {
      case 'checking':
        pill.classList.add('is-checking');
        text.textContent = tr('statusChecking', 'Checking…');
        break;
      case 'online': {
        pill.classList.add('is-online');
        text.textContent = tr('statusConnectedShort', 'Connected');
        const where = status?.mode === 'remote' ? tr('statusRemote', 'remote') : tr('statusThisComputer', 'this computer');
        pill.title = status?.version ? `Swoop ${status.version} · ${where}` : `Swoop · ${where}`;
        break;
      }
      case 'not-running': {
        pill.classList.add('is-warning');
        text.textContent = tr('statusNotRunningShort', 'Not running');
        const btn = h('button', { text: tr('actionLaunch', 'Launch'), attrs: { type: 'button' } });
        btn.addEventListener('click', () => void launch(btn));
        children.push(btn);
        break;
      }
      case 'no-host':
      case 'remote-unreachable':
        pill.classList.add('is-error');
        text.textContent = tr('statusOffline', 'Offline');
        break;
      case 'remote-refused':
        pill.classList.add('is-error');
        text.textContent = tr('statusRefused', 'Not paired');
        break;
    }
    pill.replaceChildren(...children);
  }

  function renderCard(state: ConnectionState): void {
    if (state === 'online' || state === 'checking') {
      card.hidden = true;
      card.replaceChildren();
      return;
    }
    let title: string;
    let body: string;
    const actions: HTMLElement[] = [];
    let glyph: 'power' | 'plug' | 'server' = 'plug';
    switch (state) {
      case 'not-running':
        glyph = 'power';
        title = tr('offlineNotRunningTitle', 'Swoop isn’t running');
        body = tr('offlineNotRunningBody', 'Start Swoop to send downloads to it and follow their progress here.');
        actions.push(
          textButton(tr('actionLaunchApp', 'Launch Swoop'), (e) => launch(e.currentTarget as HTMLButtonElement), 'btn primary', 'power'),
        );
        break;
      case 'no-host':
        title = tr('offlineNoHostTitle', 'Can’t reach Swoop');
        body = tr(
          'offlineNoHostBody',
          'The browser helper isn’t set up yet. In Swoop, open Settings → Browser and choose Install, then try again.',
        );
        actions.push(textButton(tr('actionTryAgain', 'Try again'), () => void refresh(), 'btn primary', 'retry'));
        actions.push(textButton(tr('actionHowToFix', 'How to fix'), () => openOptions('connection'), 'btn'));
        break;
      case 'remote-unreachable':
        glyph = 'server';
        title = tr('offlineRemoteTitle', 'Can’t reach your Swoop');
        body = tr('offlineRemoteBody', 'Make sure it’s switched on and reachable, and that the address in Connection settings is right.');
        actions.push(textButton(tr('actionTryAgain', 'Try again'), () => void refresh(), 'btn primary', 'retry'));
        actions.push(textButton(tr('actionConnectionSettings', 'Connection settings'), () => openOptions('connection'), 'btn'));
        break;
      case 'remote-refused':
      default:
        glyph = 'server';
        title = tr('offlineRefusedTitle', 'Swoop refused this browser');
        body = tr('offlineRefusedBody', 'The device token may have been revoked. Pair this browser again in Connection settings.');
        actions.push(
          textButton(tr('actionConnectionSettings', 'Connection settings'), () => openOptions('connection'), 'btn primary'),
        );
        break;
    }
    card.replaceChildren(
      featherIllustration({ width: 150, glyph, muted: state !== 'not-running' }),
      h('h3', { text: title }),
      h('p', { text: body }),
      h('div', { class: 'state-actions' }, actions),
    );
    card.hidden = false;
  }

  function apply(status: ConnectionStatus): void {
    ctx.connection = status;
    const state = connectionState(status);
    renderPill(state, status);
    if (state !== lastState) renderCard(state);
    lastState = state;
    onChange(state, status);
  }

  async function refresh(): Promise<ConnectionStatus | null> {
    try {
      const status = await getConnectionStatus();
      apply(status);
      return status;
    } catch {
      apply({ connected: false, running: false, version: null });
      return null;
    }
  }

  renderPill('checking', null);
  return { refresh, apply };
}
