/**
 * Downloads list + speed card. Rows come from `GET /api/v1/tasks/rows?smart=active` (everything in
 * flight) plus the most recent tasks (`sort=created_at&desc=true`) so paused, failed and just
 * finished downloads stay visible, and are kept live by native events relayed over the popup port
 * (`progress`, `task_state_changed`, `task_added`, `task_updated`, `task_removed`,
 * `global_stats` — docs/api/websocket.md). Pause/Resume/Retry/Cancel call the real task-action
 * routes. Rows are updated in place so keyboard focus and bar animations survive live updates.
 */

import {
  formatBytesCompact,
  formatEta,
  formatPercent,
  formatProgressSize,
  formatSpeed,
  formatSpeedCompact,
  splitSpeed,
} from '../shared/format.ts';
import { apiRequest, launchApp } from '../shared/background-client.ts';
import type {
  GlobalStats,
  SwoopEvent,
  Progress,
  ProgressUpdate,
  TaskKind,
  TaskLike,
  TaskRemovedData,
  TaskRow,
  TaskState,
  TaskStateChangedData,
} from '../shared/types.ts';
import { featherIllustration } from '../shared-ui/brand.ts';
import { capsule, fileTileFor, h, iconButton, progressBar, setProgress, tr, type StatusColor } from '../shared-ui/dom.ts';
import type { IconName } from '../shared-ui/icons.ts';
import { errorMessage, type PopupContext } from './context.ts';

interface DisplayRow {
  id: string;
  name: string;
  kind: TaskKind;
  state: TaskState;
  progress: Progress;
  domain: string | null;
}

const PAUSABLE_STATES = new Set<TaskState>(['queued', 'resolving', 'connecting', 'downloading', 'retrying', 'seeding']);
const CANCELLABLE_STATES = new Set<TaskState>([
  'pending', 'queued', 'scheduled', 'resolving', 'connecting', 'downloading', 'paused',
  'retrying', 'verifying', 'processing',
]);
const MOVING_STATES = new Set<TaskState>([
  'resolving', 'connecting', 'downloading', 'retrying', 'verifying', 'processing', 'seeding',
]);
const MAX_ROWS = 12;

function stateColor(state: TaskState): StatusColor {
  switch (state) {
    case 'completed':
    case 'seeding':
      return 'green';
    case 'paused':
      return 'orange';
    case 'failed':
      return 'red';
    case 'pending':
    case 'queued':
    case 'scheduled':
    case 'cancelled':
      return 'grey';
    default:
      return 'blue';
  }
}

function stateIcon(state: TaskState): IconName | undefined {
  switch (state) {
    case 'completed':
      return 'check';
    case 'paused':
      return 'pause';
    case 'failed':
      return 'warning';
    case 'queued':
    case 'pending':
    case 'scheduled':
      return 'hourglass';
    case 'seeding':
      return 'arrowUp';
    default:
      return 'arrowDown';
  }
}

const STATE_FALLBACK: Record<TaskState, string> = {
  pending: 'Pending',
  queued: 'Queued',
  scheduled: 'Scheduled',
  resolving: 'Resolving',
  connecting: 'Connecting',
  downloading: 'Downloading',
  paused: 'Paused',
  retrying: 'Retrying',
  verifying: 'Verifying',
  processing: 'Processing',
  completed: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
  seeding: 'Seeding',
};

function stateLabel(state: TaskState): string {
  return tr(`state_${state}`, STATE_FALLBACK[state] ?? state);
}

function fractionOf(p: Progress): number {
  if (p.total && p.total > 0) return Math.min(1, p.downloaded / p.total);
  return p.fraction ?? 0;
}

function sizeText(p: Progress, state: TaskState): string {
  if (state === 'completed') return formatBytesCompact(p.total ?? p.downloaded);
  if (p.total && p.total > 0) return formatProgressSize(p.downloaded, p.total, tr('sizeOfWord', 'of'));
  return p.downloaded > 0 ? formatBytesCompact(p.downloaded) : '';
}

interface RowView {
  li: HTMLLIElement;
  title: HTMLElement;
  actions: HTMLElement;
  progressLine: HTMLElement;
  bar: HTMLElement;
  pct: HTMLElement;
  stats: HTMLElement;
  statsLeft: HTMLElement;
  statsRight: HTMLElement;
  renderedState: TaskState | null;
  renderedName: string;
}

export interface DownloadsHandle {
  refresh(): Promise<void>;
  handleEvent(event: SwoopEvent): void;
  setUsable(usable: boolean): void;
}

export function initDownloads(ctx: PopupContext): DownloadsHandle {
  const section = document.getElementById('downloads-section') as HTMLElement;
  const list = document.getElementById('downloads-list') as HTMLUListElement;
  const empty = document.getElementById('downloads-empty') as HTMLElement;
  const count = document.getElementById('downloads-count') as HTMLElement;
  const speedCard = document.getElementById('speed') as HTMLElement;
  const speedDown = document.getElementById('speed-down') as HTMLElement;
  const speedDownUnit = document.getElementById('speed-down-unit') as HTMLElement;
  const speedUp = document.getElementById('speed-up') as HTMLElement;
  const activeChip = document.getElementById('active-chip') as HTMLElement;
  const activeCount = document.getElementById('active-count') as HTMLElement;

  const rows = new Map<string, DisplayRow>();
  const views = new Map<string, RowView>();
  let order: string[] = [];
  let loaded = false;
  let usable = true;
  let haveGlobalStats = false;
  let lastLiveUpdate = 0;

  // --- speed card ------------------------------------------------------------------------------

  function renderSpeed(down: number, up: number, active: number): void {
    const { value, unit } = splitSpeed(down);
    speedDown.textContent = value;
    speedDownUnit.textContent = unit;
    speedUp.textContent = formatSpeedCompact(up);
    activeCount.textContent = tr('activeCount', '$1 active', String(active));
    speedCard.classList.toggle('is-idle', down <= 0);
    activeChip.classList.toggle('is-active', active > 0);
    speedCard.setAttribute(
      'aria-label',
      tr('speedSummary', 'Downloading at $1, uploading at $2, $3 active', [formatSpeed(down), formatSpeed(up), String(active)]),
    );
  }

  function renderSpeedFromRows(): void {
    if (haveGlobalStats) return;
    let down = 0;
    let up = 0;
    let active = 0;
    for (const row of rows.values()) {
      if (MOVING_STATES.has(row.state) || row.state === 'queued') active += 1;
      if (MOVING_STATES.has(row.state)) {
        down += row.progress.speed || 0;
        up += row.progress.upload_speed || 0;
      }
    }
    renderSpeed(down, up, active);
  }

  // --- rows ------------------------------------------------------------------------------------

  async function runAction(id: string, action: 'pause' | 'resume' | 'retry' | 'cancel'): Promise<void> {
    try {
      await apiRequest('POST', `/api/v1/tasks/${encodeURIComponent(id)}/${action}`);
    } catch (err) {
      ctx.toast(errorMessage(err), 'error');
    }
  }

  function actionButtons(row: DisplayRow): HTMLElement[] {
    const out: HTMLElement[] = [];
    const name = row.name;
    const add = (key: string, iconName: IconName, label: string, fn: () => void | Promise<void>): void => {
      const btn = iconButton(iconName, `${label}: ${name}`, fn);
      btn.title = label;
      btn.dataset['action'] = key;
      out.push(btn);
    };
    if (PAUSABLE_STATES.has(row.state)) add('pause', 'pause', tr('actionPause', 'Pause'), () => runAction(row.id, 'pause'));
    if (row.state === 'paused') add('resume', 'play', tr('actionResume', 'Resume'), () => runAction(row.id, 'resume'));
    if (row.state === 'failed') add('retry', 'retry', tr('actionRetry', 'Retry'), () => runAction(row.id, 'retry'));
    if (row.state === 'completed' && ctx.connection?.mode !== 'remote') {
      add('open', 'open', tr('actionShowInApp', 'Show in Swoop'), async () => {
        try {
          await launchApp();
        } catch (err) {
          ctx.toast(errorMessage(err), 'error');
        }
      });
    }
    if (CANCELLABLE_STATES.has(row.state)) add('cancel', 'close', tr('actionCancel', 'Cancel'), () => runAction(row.id, 'cancel'));
    return out;
  }

  function createView(row: DisplayRow): RowView {
    const title = h('span', { class: 'row-title' });
    const actions = h('div', { class: 'row-actions' });
    const bar = progressBar(0, 'blue', row.name);
    const pct = h('span', { class: 'pct' });
    const progressLine = h('div', { class: 'progress-line' }, [bar, pct]);
    const statsLeft = h('span', { class: 'grow' });
    const statsRight = h('span', { class: 'strong' });
    const stats = h('div', { class: 'row-meta' }, [statsLeft, statsRight]);
    const li = h('li', { class: 'row', attrs: { 'data-id': row.id } }, [
      fileTileFor(row.name, row.kind),
      h('div', { class: 'row-body' }, [h('div', { class: 'row-top' }, [title, actions]), progressLine, stats]),
    ]);
    return {
      li, title, actions, progressLine, bar, pct, stats, statsLeft, statsRight,
      renderedState: null, renderedName: '',
    };
  }

  function updateView(view: RowView, row: DisplayRow): void {
    const p = row.progress;
    const moving = MOVING_STATES.has(row.state);
    const color = stateColor(row.state);

    if (view.renderedName !== row.name) {
      view.title.textContent = row.name;
      view.title.title = row.name;
      view.bar.setAttribute('aria-label', row.name);
      view.renderedName = row.name;
    }

    if (view.renderedState !== row.state) {
      // Keep keyboard focus on the equivalent control when the action set changes.
      const focused = view.actions.contains(document.activeElement)
        ? (document.activeElement as HTMLElement).dataset['action']
        : undefined;
      view.actions.replaceChildren(...actionButtons(row));
      if (focused) {
        const next = view.actions.querySelector<HTMLElement>(`[data-action="${focused}"]`) ?? view.actions.querySelector('button');
        next?.focus();
      }
      view.bar.className = `bar is-${color}`;
      view.renderedState = row.state;
    }

    const fraction = fractionOf(p);
    const indeterminate = moving && !(p.total && p.total > 0) && fraction === 0;
    view.bar.classList.toggle('is-indeterminate', indeterminate);
    setProgress(view.bar, indeterminate ? null : fraction);
    view.pct.textContent = p.total ? formatPercent(p.downloaded, p.total) : fraction > 0 ? `${Math.round(fraction * 100)}%` : '';

    const size = sizeText(p, row.state);
    view.progressLine.hidden = row.state === 'completed' || row.state === 'cancelled';
    view.statsLeft.textContent = [size, row.domain].filter(Boolean).join(' · ');
    if (row.state === 'downloading' || row.state === 'seeding') {
      const speed = row.state === 'seeding' ? p.upload_speed : p.speed;
      const eta = row.state === 'downloading' && p.eta_seconds !== null ? ` · ${formatEta(p.eta_seconds)}` : '';
      view.statsRight.className = 'strong';
      view.statsRight.textContent = `${formatSpeedCompact(speed)}${eta}`;
    } else if (view.statsRight.dataset['state'] !== row.state) {
      view.statsRight.className = '';
      view.statsRight.replaceChildren(capsule(stateLabel(row.state), color, stateIcon(row.state)));
    }
    view.statsRight.dataset['state'] = row.state === 'downloading' || row.state === 'seeding' ? '' : row.state;
  }

  function render(): void {
    if (!usable) {
      section.hidden = true;
      return;
    }
    section.hidden = false;
    if (!loaded) {
      list.hidden = false;
      empty.hidden = true;
      if (list.childElementCount === 0) {
        list.replaceChildren(h('li', { class: 'skeleton' }), h('li', { class: 'skeleton' }));
      }
      return;
    }
    const ids = order.filter((id) => rows.has(id)).slice(0, MAX_ROWS);
    count.hidden = ids.length === 0;
    count.textContent = String(rows.size);
    if (ids.length === 0) {
      list.hidden = true;
      list.replaceChildren();
      views.clear();
      if (empty.childElementCount === 0) {
        empty.append(
          featherIllustration({ width: 150 }),
          h('h3', { text: tr('emptyTitle', 'Nothing downloading') }),
          h('p', {
            text: tr('emptyBody', 'Paste a link above, or just click a download link — Swoop will pick it up.'),
          }),
        );
      }
      empty.hidden = false;
      renderSpeedFromRows();
      return;
    }
    empty.hidden = true;
    list.hidden = false;
    for (const id of [...views.keys()]) {
      if (!ids.includes(id)) {
        views.get(id)?.li.remove();
        views.delete(id);
      }
    }
    list.querySelectorAll('.skeleton').forEach((node) => node.remove());
    ids.forEach((id, index) => {
      const row = rows.get(id) as DisplayRow;
      let view = views.get(id);
      if (!view) {
        view = createView(row);
        views.set(id, view);
      }
      updateView(view, row);
      if (list.children[index] !== view.li) list.insertBefore(view.li, list.children[index] ?? null);
    });
    renderSpeedFromRows();
  }

  function toDisplayRow(row: TaskRow | TaskLike): DisplayRow {
    const existing = rows.get(row.id);
    const domain = typeof (row as TaskRow).domain === 'string' ? (row as TaskRow).domain : (existing?.domain ?? null);
    return { id: row.id, name: row.name, kind: row.kind, state: row.state, progress: row.progress, domain };
  }

  async function refresh(): Promise<void> {
    try {
      const [active, recent] = await Promise.all([
        apiRequest<TaskRow[]>('GET', '/api/v1/tasks/rows?smart=active'),
        apiRequest<TaskRow[]>('GET', '/api/v1/tasks/rows?sort=created_at&desc=true&limit=10').catch(() => [] as TaskRow[]),
      ]);
      rows.clear();
      order = [];
      for (const row of [...active, ...recent]) {
        if (rows.has(row.id)) continue;
        rows.set(row.id, toDisplayRow(row));
        order.push(row.id);
      }
      loaded = true;
      render();
    } catch {
      // The status card explains what's wrong; keep whatever we had.
      loaded = true;
      render();
    }
  }

  function upsert(row: DisplayRow, atTop: boolean): void {
    if (!rows.has(row.id)) {
      if (atTop) order.unshift(row.id);
      else order.push(row.id);
    }
    rows.set(row.id, row);
  }

  function handleEvent(event: SwoopEvent): void {
    switch (event.type) {
      case 'task_added':
        upsert(toDisplayRow(event.data as TaskLike), true);
        render();
        break;
      case 'task_updated':
        upsert(toDisplayRow(event.data as TaskLike), false);
        render();
        break;
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
        if (changed) {
          lastLiveUpdate = Date.now();
          render();
        }
        break;
      }
      case 'global_stats': {
        const stats = event.data as GlobalStats;
        haveGlobalStats = true;
        renderSpeed(stats.download_speed, stats.upload_speed, stats.active);
        break;
      }
      default:
        break;
    }
  }

  // Safety net: if live progress isn't arriving (older host, remote without high-volume
  // events), quietly re-read the list every couple of seconds while something is moving.
  setInterval(() => {
    if (!usable || !loaded) return;
    const moving = [...rows.values()].some((row) => MOVING_STATES.has(row.state));
    if (moving && Date.now() - lastLiveUpdate > 2500) void refresh();
  }, 2000);

  function setUsable(next: boolean): void {
    usable = next;
    speedCard.hidden = !next;
    render();
  }

  renderSpeed(0, 0, 0);
  render();
  return { refresh, handleEvent, setUsable };
}
