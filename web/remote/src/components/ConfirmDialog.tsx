import { useEffect, useRef } from "preact/hooks";
import type { JSX } from "preact";

export interface ConfirmDialogProps {
  title: string;
  onCancel: () => void;
  children?: JSX.Element | JSX.Element[] | string;
  actions: JSX.Element;
}

/** A simple, accessible modal: traps Escape-to-close and focuses itself on open. */
export function ConfirmDialog({ title, onCancel, children, actions }: ConfirmDialogProps) {
  const dialogRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    dialogRef.current?.focus();
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div class="dialog-overlay" onClick={onCancel}>
      <div
        class="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
        tabIndex={-1}
        ref={dialogRef}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="dialog-title" class="dialog-title">
          {title}
        </h2>
        <div class="dialog-body">{children}</div>
        <div class="dialog-actions">{actions}</div>
      </div>
    </div>
  );
}
