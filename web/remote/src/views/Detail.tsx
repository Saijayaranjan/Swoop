import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import type { TFunction } from "../i18n/useT.ts";
import { useRoute } from "../router/useRouter.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { pushToast } from "../state/toast.ts";
import { StatePill } from "../components/StatePill.tsx";
import { ProgressBar } from "../components/ProgressBar.tsx";
import { Icon } from "../components/Icon.tsx";
import { errorExplanationKey, type FileSelection, type PeerInfo, type Task, type TaskLogEntry } from "../api/types.ts";
import { formatBytes, formatEta, formatSpeed, formatTimestamp } from "../utils/format.ts";
import { canCancel, canPause, canResume, canRetry } from "../utils/taskState.ts";

type Tab = "overview" | "stats" | "files" | "trackers" | "peers" | "log";

const PEER_POLL_MS = 5000;

export function Detail() {
  const [route, navigate] = useRoute();
  const t = useT();
  const taskId = route.taskId;
  const [task, setTask] = useState<Task | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [tab, setTab] = useState<Tab>("overview");
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [log, setLog] = useState<TaskLogEntry[]>([]);
  const [trackerInput, setTrackerInput] = useState("");

  useEffect(() => {
    if (!taskId) return;
    let cancelled = false;
    setTask(null);
    setLoadError(false);
    apiClient
      .getTask(taskId)
      .then((t) => {
        if (!cancelled) setTask(t);
      })
      .catch(() => {
        if (!cancelled) setLoadError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [taskId]);

  useEffect(() => {
    if (!taskId || tab !== "peers" || task?.torrent === null || task?.torrent === undefined) return;
    let cancelled = false;
    const poll = (): void => {
      apiClient
        .torrentPeers(taskId)
        .then((p) => {
          if (!cancelled) setPeers(p);
        })
        .catch(() => {});
    };
    poll();
    const interval = setInterval(poll, PEER_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [taskId, tab, task?.torrent]);

  useEffect(() => {
    if (!taskId || tab !== "log") return;
    let cancelled = false;
    apiClient
      .taskLog(taskId, 200)
      .then((entries) => {
        if (!cancelled) setLog(entries);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [taskId, tab]);

  async function refresh(): Promise<void> {
    if (!taskId) return;
    try {
      setTask(await apiClient.getTask(taskId));
    } catch {
      // keep showing the stale task; the header status indicator covers connectivity issues
    }
  }

  async function runAction(fn: () => Promise<Task>): Promise<void> {
    try {
      setTask(await fn());
    } catch (err) {
      pushToast(t("toast.action_failed", { message: err instanceof ApiError ? err.message : String(err) }), "error");
    }
  }

  if (!taskId) {
    return null;
  }

  if (loadError) {
    return (
      <div class="view view-detail">
        <DetailHeader onBack={() => navigate({ view: "downloads", taskId: null })} title={t("detail.title")} />
        <p class="empty-state">{t("detail.not_found")}</p>
      </div>
    );
  }

  if (!task) {
    return (
      <div class="view view-detail">
        <DetailHeader onBack={() => navigate({ view: "downloads", taskId: null })} title={t("detail.title")} />
        <p class="empty-state">{t("detail.loading")}</p>
      </div>
    );
  }

  const percent = task.progress.total && task.progress.total > 0 ? (task.progress.downloaded / task.progress.total) * 100 : null;
  const isTorrent = task.torrent !== null;

  const tabs: Tab[] = ["overview", "stats", ...(isTorrent ? (["files", "trackers", "peers"] as Tab[]) : []), "log"];

  return (
    <div class="view view-detail">
      <DetailHeader onBack={() => navigate({ view: "downloads", taskId: null })} title={task.name} />

      <div class="detail-summary">
        <StatePill state={task.state} />
        <ProgressBar fraction={percent !== null ? percent / 100 : task.progress.fraction} tone={task.state === "failed" ? "error" : "normal"} />
        <div class="detail-summary-meta">
          <span>{percent !== null ? `${Math.round(percent)}%` : formatBytes(task.progress.downloaded)}</span>
          <span>{formatSpeed(task.progress.speed)}</span>
          <span>{formatEta(task.progress.eta_seconds)}</span>
        </div>
        <div class="detail-actions">
          {canPause(task.state) && (
            <button type="button" class="button" onClick={() => runAction(() => apiClient.taskAction(task.id, "pause"))}>
              {t("action.pause")}
            </button>
          )}
          {canResume(task.state) && (
            <button type="button" class="button" onClick={() => runAction(() => apiClient.taskAction(task.id, "resume"))}>
              {t("action.resume")}
            </button>
          )}
          {canRetry(task.state) && (
            <button type="button" class="button" onClick={() => runAction(() => apiClient.taskAction(task.id, "retry"))}>
              {t("action.retry")}
            </button>
          )}
          {canCancel(task.state) && (
            <button type="button" class="button" onClick={() => runAction(() => apiClient.taskAction(task.id, "cancel"))}>
              {t("action.cancel")}
            </button>
          )}
        </div>
      </div>

      {task.error && (
        <div class="detail-error">
          <Icon name="warning" size={18} />
          <div>
            <strong>{t("detail.error_title")}</strong>
            <p>{t(errorExplanationKey(task.error.kind) as "error.unknown")}</p>
            {task.error.detail && <p class="detail-error-tech">{task.error.detail}</p>}
          </div>
        </div>
      )}

      <div class="tab-row" role="tablist">
        {tabs.map((tb) => (
          <button key={tb} type="button" role="tab" aria-selected={tab === tb} class={`tab${tab === tb ? " is-selected" : ""}`} onClick={() => setTab(tb)}>
            {t(`detail.tab_${tb}` as "detail.tab_overview")}
          </button>
        ))}
      </div>

      {tab === "overview" && <OverviewTab task={task} t={t} />}
      {tab === "stats" && <StatsTab task={task} t={t} />}
      {tab === "files" && task.torrent && <FilesTab task={task} onChanged={refresh} t={t} />}
      {tab === "trackers" && task.torrent && (
        <TrackersTab
          task={task}
          t={t}
          trackerInput={trackerInput}
          setTrackerInput={setTrackerInput}
          onChanged={refresh}
        />
      )}
      {tab === "peers" && task.torrent && <PeersTab peers={peers} t={t} />}
      {tab === "log" && <LogTab log={log} t={t} />}
    </div>
  );
}

function DetailHeader({ onBack, title }: { onBack: () => void; title: string }) {
  return (
    <div class="detail-header">
      <button type="button" class="icon-button" aria-label="Back" onClick={onBack}>
        <Icon name="chevron-right" class="icon-flip" />
      </button>
      <h1 class="detail-header-title">{title}</h1>
    </div>
  );
}

function OverviewTab({ task, t }: { task: Task; t: TFunction }) {
  return (
    <dl class="detail-list">
      <div>
        <dt>{t("detail.destination")}</dt>
        <dd>{task.directory}</dd>
      </div>
      <div>
        <dt>{t("detail.source")}</dt>
        <dd class="truncate">{task.source.urls?.[0] ?? task.source.uri ?? task.source.playlist_url ?? task.source.name ?? "--"}</dd>
      </div>
      <div>
        <dt>{t("detail.queue")}</dt>
        <dd>{task.queue_id}</dd>
      </div>
      <div>
        <dt>{t("detail.added")}</dt>
        <dd>{formatTimestamp(task.created_at)}</dd>
      </div>
      {task.started_at && (
        <div>
          <dt>{t("detail.started")}</dt>
          <dd>{formatTimestamp(task.started_at)}</dd>
        </div>
      )}
      {task.completed_at && (
        <div>
          <dt>{t("detail.completed")}</dt>
          <dd>{formatTimestamp(task.completed_at)}</dd>
        </div>
      )}
    </dl>
  );
}

function StatsTab({ task, t }: { task: Task; t: TFunction }) {
  return (
    <dl class="detail-list">
      <div>
        <dt>{t("detail.connections")}</dt>
        <dd>{task.progress.active_connections}</dd>
      </div>
      {task.torrent && (
        <div>
          <dt>{t("detail.peers_seeds")}</dt>
          <dd>{t("detail.peers_seeds", { peers: task.progress.peers, seeds: task.progress.seeds })}</dd>
        </div>
      )}
      {task.torrent && (
        <div>
          <dt>{t("detail.ratio")}</dt>
          <dd>{task.progress.ratio.toFixed(2)}</dd>
        </div>
      )}
      <div>
        <dt>Retries</dt>
        <dd>{task.stats.retries}</dd>
      </div>
      <div>
        <dt>Peak speed</dt>
        <dd>{formatSpeed(task.stats.peak_speed)}</dd>
      </div>
      {task.stats.http_version && (
        <div>
          <dt>HTTP version</dt>
          <dd>{task.stats.http_version}</dd>
        </div>
      )}
      {task.stats.server && (
        <div>
          <dt>Server</dt>
          <dd>{task.stats.server}</dd>
        </div>
      )}
    </dl>
  );
}

function FilesTab({ task, onChanged, t }: { task: Task; onChanged: () => void; t: TFunction }) {
  const files = task.torrent?.files ?? [];

  async function toggle(index: number, selected: boolean): Promise<void> {
    const selection: FileSelection[] = files.map((f) => ({ index: f.index, selected: f.index === index ? selected : f.selected, priority: f.priority }));
    try {
      await apiClient.setTorrentFiles(task.id, selection);
      onChanged();
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  if (files.length === 0) {
    return <p class="empty-state">{t("detail.loading")}</p>;
  }

  return (
    <ul class="file-list">
      {files.map((f) => (
        <li key={f.index} class="file-list-item">
          <label class="checkbox-field">
            <input type="checkbox" checked={f.selected} onChange={(e) => toggle(f.index, (e.currentTarget as HTMLInputElement).checked)} />
            <span class="truncate">{f.path}</span>
          </label>
          <span class="file-list-size">{formatBytes(f.size)}</span>
        </li>
      ))}
    </ul>
  );
}

function TrackersTab({
  task,
  onChanged,
  t,
  trackerInput,
  setTrackerInput,
}: {
  task: Task;
  onChanged: () => void;
  t: TFunction;
  trackerInput: string;
  setTrackerInput: (v: string) => void;
}) {
  const trackers = task.torrent?.trackers ?? [];

  async function addTracker(): Promise<void> {
    if (!trackerInput.trim()) return;
    try {
      await apiClient.addTrackers(task.id, [trackerInput.trim()]);
      setTrackerInput("");
      onChanged();
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  async function remove(url: string): Promise<void> {
    try {
      await apiClient.removeTracker(task.id, url);
      onChanged();
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  return (
    <div>
      <div class="tracker-add">
        <input
          type="text"
          value={trackerInput}
          placeholder={t("detail.tracker_add_placeholder")}
          onInput={(e) => setTrackerInput((e.currentTarget as HTMLInputElement).value)}
        />
        <button type="button" class="button" onClick={addTracker}>
          {t("detail.tracker_add")}
        </button>
      </div>
      {trackers.length === 0 ? (
        <p class="empty-state">{t("detail.trackers_empty")}</p>
      ) : (
        <ul class="tracker-list">
          {trackers.map((tr) => (
            <li key={tr.url} class="tracker-list-item">
              <span class="truncate">{tr.url}</span>
              <span class={`tracker-health tracker-health-${tr.health}`}>{tr.health}</span>
              <button type="button" class="icon-button" aria-label={t("action.remove")} onClick={() => remove(tr.url)}>
                <Icon name="close" size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PeersTab({ peers, t }: { peers: PeerInfo[]; t: TFunction }) {
  if (peers.length === 0) {
    return <p class="empty-state">{t("detail.peers_empty")}</p>;
  }
  return (
    <ul class="peer-list">
      {peers.map((p) => (
        <li key={p.address} class="peer-list-item">
          <span class="truncate">{p.address}</span>
          <span>{p.client ?? "--"}</span>
          <span>{formatSpeed(p.download_speed)}</span>
          <span>{formatSpeed(p.upload_speed)}</span>
        </li>
      ))}
    </ul>
  );
}

function LogTab({ log, t }: { log: TaskLogEntry[]; t: TFunction }) {
  if (log.length === 0) {
    return <p class="empty-state">{t("detail.log_empty")}</p>;
  }
  return (
    <ul class="log-list">
      {log.map((entry, i) => (
        <li key={i} class={`log-entry log-level-${entry.level}`}>
          <span class="log-time">{formatTimestamp(entry.at)}</span>
          <span class="log-code">{entry.code}</span>
          <span>{entry.message}</span>
        </li>
      ))}
    </ul>
  );
}
