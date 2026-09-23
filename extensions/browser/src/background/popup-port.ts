/**
 * The popup keeps a long-lived `runtime.connect({name: POPUP_PORT_NAME})` port open while
 * visible, so the background can fan native events out to it live (progress, state changes,
 * notifications) and so `notifications.ts` knows to stay quiet while the popup's own toast would
 * show the same thing (docs/api/extension.md: "A toast in the popup confirms").
 */

import browser from 'webextension-polyfill';
import type { NativePort } from './native-port.ts';
import { setPopupOpen } from './state.ts';
import { POPUP_PORT_NAME, type PopupPortMessage } from '../shared/messages.ts';

export function registerPopupPort(nativePort: NativePort): void {
  browser.runtime.onConnect.addListener((port) => {
    if (port.name !== POPUP_PORT_NAME) return;

    setPopupOpen(true).catch(() => {});

    const unsubscribeEvent = nativePort.onEvent((event) => {
      postSafely(port, { type: 'event', event });
    });
    const unsubscribeStatus = nativePort.onStatusChange((status) => {
      postSafely(port, {
        type: 'connection-status',
        status: { connected: status.connected, running: status.running === true, version: status.version },
      });
    });

    const initial = nativePort.getStatus();
    postSafely(port, {
      type: 'connection-status',
      status: { connected: initial.connected, running: initial.running === true, version: initial.version },
    });

    port.onDisconnect.addListener(() => {
      unsubscribeEvent();
      unsubscribeStatus();
      setPopupOpen(false).catch(() => {});
    });
  });
}

function postSafely(port: browser.Runtime.Port, message: PopupPortMessage): void {
  try {
    port.postMessage(message);
  } catch {
    // Port already closed; onDisconnect will clean up listeners shortly.
  }
}
