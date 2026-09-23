import { useMemo, useState } from "preact/hooks";
import { taskTableStore } from "../state/tasks.ts";
import { useStore } from "../state/useStore.ts";
import { useT } from "../i18n/useT.ts";
import { useRoute } from "../router/useRouter.ts";
import { apiClient } from "../api/singleton.ts";
import { pushToast } from "../state/toast.ts";
import { ApiError } from "../api/errors.ts";
import { TaskRowItem } from "../components/TaskRowItem.tsx";
import { WindowedList } from "../components/WindowedList.tsx";
import { Icon } from "../components/Icon.tsx";
import { ConfirmDialog } from "../components/ConfirmDialog.tsx";
import type { RowActionKind } from "../components/RowMenu.tsx";
import { filterAndSortRows, type FilterChip, type SortKey } from "../utils/filterTasks.ts";
import type { TaskRow } from "../api/types.ts";
import { removeTaskRow } from "../state/tasks.ts";

const CHIPS: FilterChip[] = ["all", "active", "queued", "completed", "failed", "torrents"];
const CHIP_LABEL: Record<FilterChip, string> = {
  all: "filter.all",
  active: "filter.active",
  queued: "filter.queued",
  completed: "filter.completed",
  failed: "filter.failed",
  torrents: "filter.torrents",
  paused: "filter.paused",
};

const SORT_OPTIONS: SortKey[] = ["position", "created_at", "name", "size", "progress", "speed", "eta", "state"];

export function Downloads() {
  const table = useStore(taskTableStore);
  const t = useT();
  const [, navigate] = useRoute();
  const [chip, setChip] = useState<FilterChip>("all");
  const [search, setSearch] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>("position");
  const [descending, setDescending] = useState(false);
  const [pendingRemove, setPendingRemove] = useState<TaskRow | null>(null);
  const [deleteFileOnRemove, setDeleteFileOnRemove] = useState(false);

  const rows = useMemo(
    () => filterAndSortRows(Array.from(table.rows.values()), chip, search, sortKey, descending),
    [table, chip, search, sortKey, descending],
  );

  async function runAction(id: string, action: RowActionKind): Promise<void> {
    try {
      switch (action) {
        case "pause":
          await apiClient.taskAction(id, "pause");
          break;
        case "resume":
          await apiClient.taskAction(id, "resume");
          break;
        case "retry":
          await apiClient.taskAction(id, "retry");
          break;
        case "cancel":
          await apiClient.taskAction(id, "cancel");
          break;
        case "details":
          navigate({ view: "detail", taskId: id });
          break;
        case "remove": {
          const row = table.rows.get(id);
          if (row) {
            setDeleteFileOnRemove(false);
            setPendingRemove(row);
          }
          break;
        }
        case "remove-delete-file": {
          const row = table.rows.get(id);
          if (row) {
            setDeleteFileOnRemove(true);
            setPendingRemove(row);
          }
          break;
        }
      }
    } catch (err) {
      pushToast(t("toast.action_failed", { message: err instanceof ApiError ? err.message : String(err) }), "error");
    }
  }

  async function confirmRemove(): Promise<void> {
    if (!pendingRemove) return;
    const id = pendingRemove.id;
    setPendingRemove(null);
    try {
      await apiClient.deleteTask(id, deleteFileOnRemove);
      removeTaskRow(id);
    } catch (err) {
      pushToast(t("toast.action_failed", { message: err instanceof ApiError ? err.message : String(err) }), "error");
    }
  }

  return (
    <div class="view view-downloads">
      <div class="view-toolbar">
        <label class="search-field">
          <Icon name="search" size={16} />
          <input
            type="search"
            placeholder={t("downloads.search_placeholder")}
            value={search}
            onInput={(e) => setSearch((e.currentTarget as HTMLInputElement).value)}
            aria-label={t("downloads.search_placeholder")}
          />
        </label>
        <select
          class="sort-select"
          aria-label={t("sort.label")}
          value={sortKey}
          onChange={(e) => setSortKey((e.currentTarget as HTMLSelectElement).value as SortKey)}
        >
          {SORT_OPTIONS.map((opt) => (
            <option key={opt} value={opt}>
              {t(`sort.${opt}` as `sort.${SortKey}`)}
            </option>
          ))}
        </select>
        <button
          type="button"
          class="icon-button"
          aria-label="Toggle sort direction"
          aria-pressed={descending}
          onClick={() => setDescending((v) => !v)}
        >
          <Icon name={descending ? "chevron-down" : "chevron-right"} />
        </button>
      </div>

      <div class="chip-row" role="tablist" aria-label={t("sort.label")}>
        {CHIPS.map((c) => (
          <button
            key={c}
            type="button"
            role="tab"
            aria-selected={chip === c}
            class={`chip${chip === c ? " is-selected" : ""}`}
            onClick={() => setChip(c)}
          >
            {t(CHIP_LABEL[c] as "filter.all")}
          </button>
        ))}
      </div>

      <WindowedList
        items={rows}
        itemHeight={92}
        height={typeof window !== "undefined" ? Math.max(240, window.innerHeight - 220) : 480}
        getKey={(r) => r.id}
        renderItem={(row) => <TaskRowItem row={row} onOpen={(id) => navigate({ view: "detail", taskId: id })} onAction={runAction} />}
        emptyState={
          <div class="empty-state">
            <p>{t(table.rows.size === 0 ? "downloads.empty" : "downloads.empty_filtered")}</p>
            {table.rows.size === 0 && <p class="empty-state-hint">{t("downloads.empty_hint")}</p>}
          </div>
        }
      />

      {pendingRemove && (
        <ConfirmDialog
          title={t("confirm.remove_title")}
          onCancel={() => setPendingRemove(null)}
          actions={
            <>
              <button type="button" class="button" onClick={() => setPendingRemove(null)}>
                {t("action.cancel_dialog")}
              </button>
              <button type="button" class="button button-danger" onClick={confirmRemove}>
                {t("confirm.remove_confirm")}
              </button>
            </>
          }
        >
          <p>{t("confirm.remove_body", { name: pendingRemove.name })}</p>
          <label class="checkbox-field">
            <input type="checkbox" checked={deleteFileOnRemove} onChange={(e) => setDeleteFileOnRemove((e.currentTarget as HTMLInputElement).checked)} />
            {t("confirm.remove_delete_file")}
          </label>
        </ConfirmDialog>
      )}
    </div>
  );
}
