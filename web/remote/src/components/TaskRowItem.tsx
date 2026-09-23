import { useRef, useState } from "preact/hooks";
import type { TaskRow } from "../api/types.ts";
import { formatBytes, formatEta, formatPercent, formatSpeed } from "../utils/format.ts";
import { isActive } from "../utils/taskState.ts";
import { StatePill } from "./StatePill.tsx";
import { ProgressBar } from "./ProgressBar.tsx";
import { Icon } from "./Icon.tsx";
import { RowMenu, type RowActionKind } from "./RowMenu.tsx";

export interface TaskRowItemProps {
  row: TaskRow;
  onOpen: (id: string) => void;
  onAction: (id: string, action: RowActionKind) => void;
}

const LONG_PRESS_MS = 450;

export function TaskRowItem({ row, onOpen, onAction }: TaskRowItemProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const pressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const suppressClick = useRef(false);

  const percent =
    row.progress.total !== null && row.progress.total > 0
      ? (row.progress.downloaded / row.progress.total) * 100
      : null;

  function startPress(): void {
    suppressClick.current = false;
    pressTimer.current = setTimeout(() => {
      suppressClick.current = true;
      setMenuOpen(true);
    }, LONG_PRESS_MS);
  }
  function cancelPress(): void {
    if (pressTimer.current !== null) {
      clearTimeout(pressTimer.current);
      pressTimer.current = null;
    }
  }

  return (
    <div class="task-row" data-state={row.state}>
      <button
        type="button"
        class="task-row-main"
        onPointerDown={startPress}
        onPointerUp={cancelPress}
        onPointerLeave={cancelPress}
        onClick={() => {
          if (suppressClick.current) {
            suppressClick.current = false;
            return;
          }
          onOpen(row.id);
        }}
      >
        <div class="task-row-top">
          <span class="task-row-name" title={row.name}>
            {row.name}
          </span>
          <StatePill state={row.state} />
        </div>
        <div class="task-row-domain">{row.domain ?? ""}</div>
        <ProgressBar fraction={percent !== null ? percent / 100 : row.progress.fraction} indeterminate={percent === null && row.progress.fraction === 0 && isActive(row.state)} tone={row.state === "failed" ? "error" : "normal"} />
        <div class="task-row-meta">
          <span>{percent !== null ? formatPercent(percent) : formatBytes(row.progress.downloaded)}</span>
          {isActive(row.state) && <span>{formatSpeed(row.progress.speed)}</span>}
          {row.state === "downloading" && <span>{formatEta(row.progress.eta_seconds)}</span>}
        </div>
      </button>
      <div class="task-row-actions">
        <button type="button" class="icon-button" aria-label="More actions" onClick={() => setMenuOpen((v) => !v)}>
          <Icon name="more" />
        </button>
        {menuOpen && (
          <RowMenu
            state={row.state}
            onAction={(action) => onAction(row.id, action)}
            onClose={() => setMenuOpen(false)}
          />
        )}
      </div>
    </div>
  );
}
