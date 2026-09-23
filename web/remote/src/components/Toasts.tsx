import { dismissToast, toastStore } from "../state/toast.ts";
import { useStore } from "../state/useStore.ts";
import { Icon } from "./Icon.tsx";

export function Toasts() {
  const toasts = useStore(toastStore);
  return (
    <div class="toast-stack" aria-live="polite" aria-atomic="false">
      {toasts.map((toast) => (
        <div key={toast.id} class={`toast toast-${toast.kind}`} role="status">
          <span class="toast-message">{toast.message}</span>
          <button
            type="button"
            class="toast-dismiss"
            aria-label="Dismiss notification"
            onClick={() => dismissToast(toast.id)}
          >
            <Icon name="close" size={16} />
          </button>
        </div>
      ))}
    </div>
  );
}
