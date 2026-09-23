import { useEffect, useRef } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import type { TFunction } from "../i18n/useT.ts";
import type { TaskState } from "../api/types.ts";
import { canCancel, canPause, canResume, canRetry } from "../utils/taskState.ts";
import { Icon } from "./Icon.tsx";

export type RowActionKind = "pause" | "resume" | "retry" | "cancel" | "remove" | "remove-delete-file" | "details";

interface MenuItem {
  action: RowActionKind;
  labelKey: Parameters<TFunction>[0];
  destructive?: boolean;
}

function itemsFor(state: TaskState): MenuItem[] {
  const items: MenuItem[] = [{ action: "details", labelKey: "action.details" }];
  if (canPause(state)) items.push({ action: "pause", labelKey: "action.pause" });
  if (canResume(state)) items.push({ action: "resume", labelKey: "action.resume" });
  if (canRetry(state)) items.push({ action: "retry", labelKey: "action.retry" });
  if (canCancel(state)) items.push({ action: "cancel", labelKey: "action.cancel" });
  items.push({ action: "remove", labelKey: "action.remove" });
  items.push({ action: "remove-delete-file", labelKey: "action.remove_and_delete", destructive: true });
  return items;
}

export interface RowMenuProps {
  state: TaskState;
  onAction: (action: RowActionKind) => void;
  onClose: () => void;
}

export function RowMenu({ state, onAction, onClose }: RowMenuProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onDocClick = (e: MouseEvent): void => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("click", onDocClick, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("click", onDocClick, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return (
    <div class="row-menu" role="menu" ref={ref}>
      {itemsFor(state).map((item) => (
        <button
          key={item.action}
          type="button"
          role="menuitem"
          class={`row-menu-item${item.destructive ? " is-destructive" : ""}`}
          onClick={() => {
            onAction(item.action);
            onClose();
          }}
        >
          {item.action === "remove-delete-file" ? <Icon name="trash" size={16} /> : null}
          {t(item.labelKey)}
        </button>
      ))}
    </div>
  );
}
