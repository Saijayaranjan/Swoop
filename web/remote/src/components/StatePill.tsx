import type { TaskState } from "../api/types.ts";
import { useT } from "../i18n/useT.ts";

const TONE: Record<TaskState, "neutral" | "active" | "good" | "bad" | "warn"> = {
  pending: "neutral",
  queued: "neutral",
  scheduled: "neutral",
  resolving: "active",
  connecting: "active",
  downloading: "active",
  paused: "warn",
  retrying: "warn",
  verifying: "active",
  processing: "active",
  completed: "good",
  failed: "bad",
  cancelled: "neutral",
  seeding: "good",
};

export function StatePill({ state }: { state: TaskState }) {
  const t = useT();
  const tone = TONE[state];
  return <span class={`pill pill-${tone}`}>{t(`state.${state}`)}</span>;
}
