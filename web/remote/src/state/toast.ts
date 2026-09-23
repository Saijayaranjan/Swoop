// Toast queue for notifications (WS `notification` events, and action errors/successes).

import { createStore } from "./store.ts";

export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: number;
  message: string;
  kind: ToastKind;
}

let nextId = 1;
const AUTO_DISMISS_MS = 5000;

export const toastStore = createStore<Toast[]>([]);

export function pushToast(message: string, kind: ToastKind = "info"): number {
  const id = nextId++;
  toastStore.setState((toasts) => [...toasts, { id, message, kind }]);
  if (typeof window !== "undefined") {
    window.setTimeout(() => dismissToast(id), AUTO_DISMISS_MS);
  }
  return id;
}

export function dismissToast(id: number): void {
  toastStore.setState((toasts) => toasts.filter((t) => t.id !== id));
}
