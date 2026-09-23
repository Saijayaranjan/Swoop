import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { pushToast } from "../state/toast.ts";
import { Sparkline } from "../components/Sparkline.tsx";
import { seedSpeedHistory, speedHistoryStore } from "../state/tasks.ts";
import { useStore } from "../state/useStore.ts";
import { formatBytes, formatSpeed } from "../utils/format.ts";
import type { Dashboard as DashboardData, Queue } from "../api/types.ts";

const POLL_MS = 3000;

export function Dashboard() {
  const t = useT();
  const [data, setData] = useState<DashboardData | null>(null);
  const [queues, setQueues] = useState<Map<string, Queue>>(new Map());
  const speedHistory = useStore(speedHistoryStore);

  useEffect(() => {
    let cancelled = false;
    const load = (): void => {
      apiClient
        .dashboard()
        .then((d) => {
          if (cancelled) return;
          setData(d);
          if (speedHistoryStore.getState().length === 0 && d.speed_history.length > 0) {
            seedSpeedHistory(d.speed_history.map((s) => ({ at: s.at, download: s.download, upload: s.upload })));
          }
        })
        .catch(() => {});
      apiClient
        .listQueues()
        .then((qs) => {
          if (!cancelled) setQueues(new Map(qs.map((q) => [q.id, q])));
        })
        .catch(() => {});
    };
    load();
    const interval = setInterval(load, POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  async function toggleQueue(id: string, currentlyPaused: boolean): Promise<void> {
    try {
      const updated = currentlyPaused ? await apiClient.resumeQueue(id) : await apiClient.pauseQueue(id);
      setQueues((prev) => new Map(prev).set(id, updated));
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  if (!data) {
    return (
      <div class="view view-dashboard">
        <h1 class="view-title">{t("dashboard.title")}</h1>
        <p class="empty-state">{t("common.loading")}</p>
      </div>
    );
  }

  return (
    <div class="view view-dashboard">
      <h1 class="view-title">{t("dashboard.title")}</h1>

      <div class="dashboard-speed-card">
        <div class="dashboard-speed-numbers">
          <div>
            <span class="dashboard-speed-label">{t("app.download_speed")}</span>
            <span class="dashboard-speed-value">{formatSpeed(data.stats.download_speed)}</span>
          </div>
          <div>
            <span class="dashboard-speed-label">{t("app.upload_speed")}</span>
            <span class="dashboard-speed-value">{formatSpeed(data.stats.upload_speed)}</span>
          </div>
        </div>
        <Sparkline points={speedHistory} />
      </div>

      <div class="stat-grid">
        <Stat label={t("dashboard.active")} value={data.stats.active} />
        <Stat label={t("dashboard.queued")} value={data.stats.queued} />
        <Stat label={t("dashboard.scheduled")} value={data.stats.scheduled} />
        <Stat label={t("dashboard.completed_today")} value={data.stats.completed_today} />
        <Stat label={t("dashboard.failed_today")} value={data.stats.failed_today} />
      </div>

      <section class="dashboard-section">
        <h2>{t("dashboard.disks")}</h2>
        {data.disks.length === 0 ? (
          <p class="empty-state">{t("common.none")}</p>
        ) : (
          <ul class="disk-list">
            {data.disks.map((d) => (
              <li key={d.path} class="disk-list-item">
                <span class="truncate">{d.path}</span>
                <span>{d.free !== null ? t("dashboard.disk_free", { free: formatBytes(d.free) }) : t("common.unknown")}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section class="dashboard-section">
        <h2>{t("dashboard.queues")}</h2>
        {data.queues.length === 0 ? (
          <p class="empty-state">{t("dashboard.no_queues")}</p>
        ) : (
          <ul class="queue-list">
            {data.queues.map((q) => {
              const queue = queues.get(q.queue_id);
              const paused = queue?.paused ?? false;
              return (
                <li key={q.queue_id} class="queue-list-item">
                  <div>
                    <div class="queue-list-id truncate">{queue?.name ?? q.queue_id}</div>
                    <div class="queue-list-meta">{t("dashboard.queue_active", { active: q.active, waiting: q.waiting })}</div>
                  </div>
                  <div class="queue-list-speed">{formatSpeed(q.download_speed)}</div>
                  <button type="button" class="button" onClick={() => toggleQueue(q.queue_id, paused)}>
                    {paused ? t("action.resume") : t("action.pause")}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div class="stat-tile">
      <span class="stat-value">{value}</span>
      <span class="stat-label">{label}</span>
    </div>
  );
}
