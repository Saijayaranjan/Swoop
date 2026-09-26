/**
 * Connection section: how the extension reaches Swoop.
 *   - This computer (default): the `swoop native-host` relay over Native Messaging
 *     (docs/api/native-messaging.md) — no URL or token; set up from the app.
 *   - Another computer: a remote Swoop's TLS listener with a paired-device token
 *     (docs/api/rest.md "Pairing"), paired here with a one-time code or a pasted token.
 */

import browser from 'webextension-polyfill';
import {
  BackgroundError,
  forgetRemote,
  getConnectionStatus,
  launchApp,
  pairRemote,
  setRemoteToken,
} from '../shared/background-client.ts';
import type { ConnectionStatus } from '../shared/messages.ts';
import { REMOTE_TOKEN_KEY } from '../shared/remote.ts';
import { h, textButton, tr } from '../shared-ui/dom.ts';
import { icon, type IconName } from '../shared-ui/icons.ts';
import { controlRow, group, nextId, note, segmented, valueRow } from './controls.ts';
import type { SectionContext, SectionDef } from './sections.ts';

type Mode = 'native' | 'remote';
let uiMode: Mode | null = null;

function pairingError(err: unknown): string {
  const kind = err instanceof BackgroundError ? err.kind : undefined;
  switch (kind) {
    case 'invalid_url':
      return tr('errRemoteUrl', 'Enter an https:// address, like https://192.168.1.20:41780.');
    case 'invalid_code':
      return tr('errPairingCode', 'Pairing codes have 8 letters and numbers, like ABCD-EFGH.');
    case 'invalid_token':
      return tr('errToken', 'That doesn’t look like a device token.');
    case 'unauthorized':
      return tr('errPairingRefused', 'That code didn’t work — codes expire after two minutes. Create a new one and try again.');
    case 'rate_limited':
      return tr('errRateLimited', 'Too many attempts. Wait a minute, then try again.');
    case 'unreachable':
      return tr(
        'errUnreachable',
        'Couldn’t reach that address. If Swoop uses its own certificate, open the address in a tab once and accept it.',
      );
    default:
      return err instanceof Error ? err.message : String(err);
  }
}

export function connectionDot(status: ConnectionStatus | null): 'is-online' | 'is-warning' | 'is-error' | '' {
  if (!status) return '';
  if (status.connected && status.running) return 'is-online';
  if (status.connected) return 'is-warning';
  return 'is-error';
}

function heroFor(status: ConnectionStatus | null, mode: Mode, remoteUrl: string): {
  color: string;
  iconName: IconName;
  title: string;
  sub: string;
} {
  const where = mode === 'remote' ? remoteUrl.replace(/^https?:\/\//, '') : tr('statusThisComputer', 'this computer');
  if (!status) {
    return { color: 'is-grey', iconName: mode === 'remote' ? 'server' : 'laptop', title: tr('statusChecking', 'Checking…'), sub: where };
  }
  if (status.connected && status.running) {
    return {
      color: 'is-green',
      iconName: mode === 'remote' ? 'server' : 'laptop',
      title: status.version ? tr('connectedToVersion', 'Connected to Swoop $1', status.version) : tr('statusConnectedShort', 'Connected'),
      sub: mode === 'remote' ? tr('connectedRemoteSub', 'Paired with $1', where) : tr('connectedNativeSub', 'Through the browser helper on this computer'),
    };
  }
  if (mode === 'remote') {
    return status.connected
      ? { color: 'is-red', iconName: 'lock', title: tr('offlineRefusedTitle', 'Swoop refused this browser'), sub: tr('refusedSub', 'The device token was rejected. Pair again below.') }
      : { color: 'is-red', iconName: 'server', title: tr('offlineRemoteTitle', 'Can’t reach your Swoop'), sub: where };
  }
  return status.connected
    ? { color: 'is-orange', iconName: 'power', title: tr('offlineNotRunningTitle', 'Swoop isn’t running'), sub: tr('notRunningSub', 'The browser helper is installed. Start Swoop to connect.') }
    : { color: 'is-red', iconName: 'plug', title: tr('offlineNoHostTitle', 'Can’t reach Swoop'), sub: tr('noHostSub', 'Open the Swoop app once and it connects to this browser automatically.') };
}

export function connectionSection(onStatus: (status: ConnectionStatus | null) => void): SectionDef {
  return {
    id: 'connection',
    icon: 'plug',
    title: () => tr('sectionConnection', 'Connection'),
    subtitle: () => tr('sectionConnectionSub', 'How this browser reaches Swoop.'),
    async render(ctx: SectionContext) {
      const { settings, rerender } = ctx;
      const stored = await browser.storage.local.get(REMOTE_TOKEN_KEY);
      const hasToken = typeof stored[REMOTE_TOKEN_KEY] === 'string' && (stored[REMOTE_TOKEN_KEY] as string).length > 0;
      const activeMode: Mode = settings.connection_mode === 'remote' && hasToken ? 'remote' : 'native';
      const mode: Mode = uiMode ?? activeMode;

      // --- status hero ---------------------------------------------------------------------
      const heroIcon = h('span', { class: 'hero-icon', attrs: { 'aria-hidden': 'true' } });
      const heroTitle = h('p', { class: 'hero-title' });
      const heroSub = h('p', { class: 'hero-sub' });
      const hero = h('div', { class: 'hero card', attrs: { role: 'status', 'aria-live': 'polite' } }, [
        heroIcon,
        h('div', { class: 'hero-text' }, [heroTitle, heroSub]),
      ]);
      const paint = (status: ConnectionStatus | null): void => {
        const view = heroFor(status, activeMode, settings.remote_url);
        hero.className = `hero card ${view.color}`;
        heroIcon.replaceChildren(icon(view.iconName));
        heroTitle.textContent = view.title;
        heroSub.textContent = view.sub;
      };
      const test = async (btn?: HTMLButtonElement): Promise<void> => {
        if (btn) btn.disabled = true;
        paint(null);
        try {
          const status = await getConnectionStatus();
          paint(status);
          onStatus(status);
        } catch {
          paint({ connected: false, running: false, version: null, mode: activeMode });
          onStatus(null);
        } finally {
          if (btn) btn.disabled = false;
        }
      };
      const testBtn = textButton(tr('actionTestConnection', 'Test connection'), (e) => test(e.currentTarget as HTMLButtonElement), 'btn', 'retry');
      hero.append(h('div', { class: 'hero-actions' }, [testBtn]));
      paint(null);
      void test();

      // --- mode picker -----------------------------------------------------------------------
      const picker = segmented<Mode>(
        'connection-mode',
        tr('connectTo', 'Connect to'),
        [
          { value: 'native', label: tr('modeThisComputer', 'This computer'), icon: 'laptop' },
          { value: 'remote', label: tr('modeAnotherComputer', 'Another computer'), icon: 'server' },
        ],
        mode,
        (next) => {
          uiMode = next;
          if (next === 'native' && settings.connection_mode !== 'native') {
            ctx.patch({ connection_mode: 'native' });
          } else if (next === 'remote' && hasToken && settings.remote_url) {
            ctx.patch({ connection_mode: 'remote' });
          }
          rerender();
        },
      );

      const out: HTMLElement[] = [
        hero,
        ...group([controlRow(tr('connectTo', 'Connect to'), picker, tr('connectToHint', 'Swoop usually runs on this computer. Pick “Another computer” to send downloads to a paired Swoop elsewhere.'))]),
      ];

      if (mode === 'native') {
        const launch = textButton(tr('actionLaunchApp', 'Launch Swoop'), async () => {
          try {
            await launchApp();
            setTimeout(() => void test(), 2500);
          } catch (err) {
            heroSub.textContent = err instanceof Error ? err.message : String(err);
          }
        }, 'btn primary', 'power');
        out.push(
          ...group(
            [
              h('ol', { class: 'steps' }, [
                h('li', { text: tr('stepInstallApp', 'Install Swoop and open it once. It connects to this browser automatically.') }),
                h('li', { text: tr('stepTest', 'Come back here and choose Test connection.') }),
              ]),
              valueRow(tr('launchRowLabel', 'Swoop isn’t open?'), launch, tr('launchRowHint', 'Starts the app on this computer (macOS).')),
            ],
            tr('groupSetUp', 'Set up'),
            tr('groupSetUpDesc', 'The extension talks to Swoop through a small helper that the app sets up by itself. No address, password or ID needed.'),
          ),
        );
        return out;
      }

      // --- remote -----------------------------------------------------------------------------
      const urlId = nextId('remote-url');
      const urlInput = h('input', {
        class: 'field',
        attrs: { id: urlId, type: 'url', placeholder: 'https://192.168.1.20:41780', autocomplete: 'off', spellcheck: 'false' },
      });
      urlInput.value = settings.remote_url;
      const codeInput = h('input', {
        class: 'field is-code',
        attrs: { type: 'text', placeholder: 'ABCD-EFGH', autocomplete: 'one-time-code', spellcheck: 'false', maxlength: '12', 'aria-label': tr('pairingCode', 'Pairing code') },
      });
      const error = h('span', { class: 'field-error', attrs: { role: 'alert' } });
      error.hidden = true;
      const showError = (message: string | null): void => {
        error.textContent = message ?? '';
        error.hidden = !message;
      };
      const pairBtn = textButton(tr('actionPair', 'Pair'), async () => {
        showError(null);
        pairBtn.disabled = true;
        try {
          await pairRemote(urlInput.value, codeInput.value);
          uiMode = 'remote';
          rerender();
        } catch (err) {
          showError(pairingError(err));
        } finally {
          pairBtn.disabled = false;
        }
      }, 'btn primary');
      codeInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          pairBtn.click();
        }
      });

      const tokenInput = h('input', {
        class: 'field',
        attrs: { type: 'password', autocomplete: 'off', spellcheck: 'false', 'aria-label': tr('deviceToken', 'Device token') },
      });
      const saveToken = textButton(tr('actionSave', 'Save'), async () => {
        showError(null);
        try {
          await setRemoteToken(urlInput.value, tokenInput.value);
          tokenInput.value = '';
          uiMode = 'remote';
          rerender();
        } catch (err) {
          showError(pairingError(err));
        }
      }, 'btn');
      const disclosure = h('details', { class: 'disclosure' }, [
        h('summary', {}, [h('span', { text: tr('useTokenInstead', 'Use a device token instead') }), icon('chevronDown')]),
        h('div', { class: 'field-row' }, [tokenInput, saveToken]),
      ]);

      const rows: HTMLElement[] = [
        controlRow(
          tr('remoteAddress', 'Address'),
          h('div', { class: 'field-row' }, [urlInput]),
          tr('remoteAddressHint', 'Shown in Swoop on that computer under Settings → Remote.'),
          urlId,
        ),
        h('div', { class: 'opt-row' }, [
          h('div', { class: 'opt-text' }, [
            h('span', { class: 'opt-label', text: tr('pairingCode', 'Pairing code') }),
            h('span', { class: 'opt-hint', text: tr('pairingCodeHint', 'In Swoop there, choose Pair a device. Codes work once, for two minutes.') }),
            error,
          ]),
          h('div', { class: 'field-row', attrs: { style: 'width:auto' } }, [codeInput, pairBtn]),
        ]),
        disclosure,
      ];
      if (hasToken) {
        const forget = textButton(tr('actionForget', 'Forget'), async () => {
          await forgetRemote();
          uiMode = 'native';
          rerender();
        }, 'btn danger');
        rows.unshift(valueRow(tr('pairedLabel', 'Paired'), forget, tr('pairedHint', 'A device token for this browser is saved. Forget it to unpair.')));
      }
      out.push(...group(rows, tr('groupRemote', 'Remote Swoop')));
      out.push(
        note(
          tr('remotePrivacyNote', 'Captured downloads — including the cookies needed to fetch them — are sent to that computer over HTTPS.'),
          'shield',
        ),
      );
      return out;
    },
  };
}
