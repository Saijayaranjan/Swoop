/**
 * "Downloads" tab: live rows fed initially by `GET /api/v1/tasks/rows?smart=active` and kept
 * current by native events relayed over the popup port (`progress`, `task_state_changed`,
 * `task_added`, `task_updated`, `task_removed` — docs/api/websocket.md). Pause/Resume/Cancel call
 * the real task-action routes.
 */

import { t } from '../../shared/i18n.ts';
import { formatBytes, formatEta, formatPercent, formatSpeed } from '../../shared/format.ts';
import { apiRequest } from '../../shared/background-client.ts';
import type {
  OspreyEvent,
  Progress,
  ProgressUpdate,
  TaskKind,
  TaskLike,
  TaskRemovedData,
  TaskRow,
  TaskState,
  TaskStateChangedData,
} from '../../shared/types.ts';
import { renderEmptyState, renderItemRow, renderList } from '../render-utils.ts';

interface DisplayRow {
  id: string;
  name: string;
  kind: TaskKind;
  state: TaskState;
  progress: Progress;
}

const PAUSABLE_STATES = new Set<TaskState>([
  'queued', 'resolving', 'connecting', 'downloading', 'retrying', 'seeding',
]);
const RESUMABLE_STATES = new Set<TaskState>(['paused', 'failed']);
const CANCELLABLE_STATES = new Set<TaskState>([
  'pending', 'queued', 'scheduled', 'resolving', 'connecting', 'downloading', 'paused',
  'retrying', 'verifying', 'processing', 'seeding',
]);

function toDisplayRow(row: TaskRow | TaskLike): DisplayRow {
  return { id: row.id, name: row.name, kind: row.kind, state: row.state, progress: row.progress };
}

function stateLabel(state: TaskState): string {
  return t(`state_${state}`) || state;
}

export interface DownloadsTabHandle {
  refresh(): Promise<void>;
  handleEvent(event: OspreyEvent): void;
}

export function initDownloadsTab(
  container: HTMLElement,
  showToast: (message: string) => void,
): DownloadsTabHandle {
  const rows = new Map<string, DisplayRow>();
  let order: string[] = [];

  function render(): void {
    if (rows.size === 0) {
      renderEmptyState(container, t('noActiveDownloads') || 'No active downloads.');
      return;
    }
    const items = order.map((id) => rows.get(id)).filter((r): r is DisplayRow => r !== undefined);
    const rendered = items.map((row) => {
      const p = row.progress;
      const meta = [
        stateLabel(row.state),
        `${formatBytes(p.downloaded)} / ${formatBytes(p.total)}`,
        formatSpeed(p.speed),
        `${t('eta') || 'ETA'} ${formatEta(p.eta_seconds)}`,
        formatPercent(p.downloaded, p.total),
      ];
      const actions = [];
      if (PAUSABLE_STATES.has(row.state)) {
        actions.push({
          label: t('actionPause') || 'Pause',
          onClick: () => runAction(row.id, 'pause'),
        });
      }
      if (RESUMABLE_STATES.has(row.state)) {
        actions.push({
          label: t('actionResume') || 'Resume',
          primary: true,
          onClick: () => runAction(row.id, 'resume'),
        });
      }
      if (CANCELLABLE_STATES.has(row.state)) {
        actions.push({
          label: t('actionCancel') || 'Cancel',
          onClick: () => runAction(row.id, 'cancel'),
        });
      }
      return renderItemRow({
        title: row.name,
        meta,
        progress: { fraction: p.total ? p.downloaded / p.total : p.fraction },
        actions,
      });
    });
    renderList(container, rendered);
  }

  async function runAction(id: string, action: 'pause' | 'resume' | 'cancel'): Promise<void> {
    try {
      await apiRequest('POST', `/api/v1/tasks/${id}/${action}`);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err));
    }
  }

  async function refresh(): Promise<void> {
    renderEmptyState(container, t('loading') || 'Loading…');
    try {
      const data = await apiRequest<TaskRow[]>('GET', '/api/v1/tasks/rows?smart=active');
      rows.clear();
      order = [];
      for (const row of data) {
        rows.set(row.id, toDisplayRow(row));
        order.push(row.id);
      }
      render();
    } catch (err) {
      renderEmptyState(
        container,
        err instanceof Error ? err.message : t('loadFailed') || 'Could not load downloads.',
      );
    }
  }

  function upsert(row: DisplayRow): void {
    if (!rows.has(row.id)) order.push(row.id);
    rows.set(row.id, row);
  }

  function handleEvent(event: OspreyEvent): void {
    switch (event.type) {
      case 'task_added':
      case 'task_updated': {
        upsert(toDisplayRow(event.data as TaskLike));
        render();
        break;
      }
      case 'task_removed': {
        const data = event.data as TaskRemovedData;
        rows.delete(data.task_id);
        order = order.filter((id) => id !== data.task_id);
        render();
        break;
      }
      case 'task_state_changed': {
        const data = event.data as TaskStateChangedData;
        const row = rows.get(data.task_id);
        if (row) {
          row.state = data.to;
          render();
        }
        break;
      }
      case 'progress': {
        const updates = event.data as ProgressUpdate[];
        let changed = false;
        for (const update of updates) {
          const row = rows.get(update.task_id);
          if (row) {
            row.progress = update.progress;
            changed = true;
          }
        }
        if (changed) render();
        break;
      }
      default:
        break;
    }
  }

  return { refresh, handleEvent };
}
