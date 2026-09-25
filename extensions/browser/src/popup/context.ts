/** State and helpers shared by the popup's modules. */

import type { ConnectionStatus } from '../shared/messages.ts';
import { tr } from '../shared-ui/dom.ts';

export interface PopupContext {
  tabId: number | null;
  /** Last known connection status (`null` until the first check completes). */
  connection: ConnectionStatus | null;
  toast(message: string, kind?: 'info' | 'error'): void;
}

let toastTimer: ReturnType<typeof setTimeout> | undefined;

export function showToast(message: string, kind: 'info' | 'error' = 'info'): void {
  const toast = document.getElementById('toast');
  if (!toast) return;
  toast.textContent = message;
  toast.classList.toggle('is-error', kind === 'error');
  toast.hidden = false;
  if (toastTimer !== undefined) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.hidden = true;
  }, 3200);
}

/** A friendly message for an error coming back from the background / Swoop. */
export function errorMessage(err: unknown): string {
  const kind = (err as { kind?: unknown }).kind;
  if (kind === 'unavailable' || kind === 'unreachable') {
    return tr('toastNotRunning', 'Swoop isn’t running — start it and try again.');
  }
  if (kind === 'unauthorized') return tr('toastUnauthorized', 'Swoop refused this browser. Pair it again in Settings.');
  if (err instanceof Error && err.message) return err.message;
  return tr('toastSomethingWrong', 'Something went wrong.');
}
