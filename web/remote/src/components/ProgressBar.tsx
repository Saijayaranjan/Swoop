export interface ProgressBarProps {
  fraction: number;
  indeterminate?: boolean;
  tone?: "normal" | "error";
}

export function ProgressBar({ fraction, indeterminate = false, tone = "normal" }: ProgressBarProps) {
  const clamped = Math.min(1, Math.max(0, fraction));
  return (
    <div
      class={`progress-track${indeterminate ? " progress-indeterminate" : ""}`}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : Math.round(clamped * 100)}
    >
      {!indeterminate && (
        <div class={`progress-fill${tone === "error" ? " progress-fill-error" : ""}`} style={{ width: `${clamped * 100}%` }} />
      )}
    </div>
  );
}
