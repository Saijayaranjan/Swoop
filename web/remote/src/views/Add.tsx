import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import { useRoute } from "../router/useRouter.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { pushToast } from "../state/toast.ts";
import { Icon } from "../components/Icon.tsx";
import type { NewTaskRequest, ProbeResult, Queue } from "../api/types.ts";
import { formatBytes } from "../utils/format.ts";

function toBase64(bytes: ArrayBuffer): string {
  let binary = "";
  const view = new Uint8Array(bytes);
  for (let i = 0; i < view.length; i++) {
    binary += String.fromCharCode(view[i] as number);
  }
  return btoa(binary);
}

function splitLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

export function Add() {
  const t = useT();
  const [, navigate] = useRoute();
  const [urls, setUrls] = useState("");
  const [torrentFile, setTorrentFile] = useState<File | null>(null);
  const [queues, setQueues] = useState<Queue[]>([]);
  const [queueId, setQueueId] = useState<string>("");
  const [directory, setDirectory] = useState("");
  const [startNow, setStartNow] = useState(true);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [probing, setProbing] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    apiClient
      .listQueues()
      .then((qs) => {
        setQueues(qs);
        const def = qs.find((q) => q.builtin) ?? qs[0];
        if (def) setQueueId(def.id);
      })
      .catch(() => {});
  }, []);

  async function buildRequests(): Promise<NewTaskRequest[]> {
    const lines = splitLines(urls);
    const base: Omit<NewTaskRequest, "url" | "magnet" | "torrent_base64"> = {
      mirrors: [],
      tags: [],
      options: {},
      start: startNow,
      origin: "remote",
      directory: directory.trim() || undefined,
      queue_id: queueId || undefined,
    };
    const requests: NewTaskRequest[] = [];
    for (const line of lines) {
      if (line.startsWith("magnet:")) {
        requests.push({ ...base, magnet: line });
      } else {
        requests.push({ ...base, url: line });
      }
    }
    if (torrentFile) {
      const bytes = await torrentFile.arrayBuffer();
      requests.push({ ...base, torrent_base64: toBase64(bytes) });
    }
    return requests;
  }

  async function handleProbe(): Promise<void> {
    setError(null);
    const requests = await buildRequests();
    const first = requests[0];
    if (!first) {
      setError(t("add.error_empty"));
      return;
    }
    setProbing(true);
    setProbe(null);
    try {
      setProbe(await apiClient.probeTask(first));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : t("add.error_generic"));
    } finally {
      setProbing(false);
    }
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    setError(null);
    const requests = await buildRequests();
    if (requests.length === 0) {
      setError(t("add.error_empty"));
      return;
    }
    setSubmitting(true);
    try {
      const results = requests.length === 1 && requests[0] ? [await apiClient.addTask(requests[0])] : await apiClient.addTasksBatch(requests);
      pushToast(t("add.success", { count: results.length }), "success");
      setUrls("");
      setTorrentFile(null);
      setProbe(null);
      navigate({ view: "downloads", taskId: null });
    } catch (err) {
      setError(err instanceof ApiError ? err.message : t("add.error_generic"));
    } finally {
      setSubmitting(false);
    }
  }

  const lineCount = splitLines(urls).length + (torrentFile ? 1 : 0);

  return (
    <form class="view view-add" onSubmit={handleSubmit}>
      <h1 class="view-title">{t("add.title")}</h1>

      <label class="field">
        <span>{t("add.urls_label")}</span>
        <textarea
          class="add-textarea"
          rows={6}
          placeholder={t("add.urls_placeholder")}
          value={urls}
          onInput={(e) => setUrls((e.currentTarget as HTMLTextAreaElement).value)}
        />
      </label>

      <label class="field">
        <span>{t("add.torrent_file_label")}</span>
        <div class="file-picker">
          <input
            type="file"
            accept=".torrent"
            id="torrent-file-input"
            onChange={(e) => setTorrentFile((e.currentTarget as HTMLInputElement).files?.[0] ?? null)}
          />
          <label class="button" for="torrent-file-input">
            {t("add.torrent_file_button")}
          </label>
          {torrentFile && <span class="file-picker-name">{t("add.torrent_file_chosen", { name: torrentFile.name })}</span>}
        </div>
      </label>

      <div class="field-row">
        <label class="field">
          <span>{t("add.queue_label")}</span>
          <select value={queueId} onChange={(e) => setQueueId((e.currentTarget as HTMLSelectElement).value)}>
            {queues.map((q) => (
              <option key={q.id} value={q.id}>
                {q.name}
              </option>
            ))}
          </select>
        </label>
        <label class="field">
          <span>{t("add.directory_label")}</span>
          <input
            type="text"
            placeholder={t("add.directory_placeholder")}
            value={directory}
            onInput={(e) => setDirectory((e.currentTarget as HTMLInputElement).value)}
          />
        </label>
      </div>

      <label class="checkbox-field">
        <input type="checkbox" checked={startNow} onChange={(e) => setStartNow((e.currentTarget as HTMLInputElement).checked)} />
        {t("add.start_now_label")}
      </label>

      <div class="add-probe-row">
        <button type="button" class="button" onClick={handleProbe} disabled={probing || lineCount === 0}>
          <Icon name="search" size={16} />
          {probing ? t("add.probe_loading") : t("add.probe_button")}
        </button>
      </div>

      {probe && (
        <div class="probe-result">
          <div>
            <span class="probe-label">{t("add.probe_name")}:</span> {probe.suggested_name}
          </div>
          {probe.metadata.size !== undefined && probe.metadata.size !== null && (
            <div>
              <span class="probe-label">{t("add.probe_size")}:</span> {formatBytes(probe.metadata.size)}
            </div>
          )}
          {probe.duplicate && <div class="probe-warning">{t("add.probe_duplicate")}</div>}
          {probe.warnings.map((w) => (
            <div key={w} class="probe-warning">
              {w}
            </div>
          ))}
        </div>
      )}

      {error && (
        <div class="form-error" role="alert">
          {error}
        </div>
      )}

      <button type="submit" class="button button-primary" disabled={submitting || lineCount === 0}>
        {submitting ? t("add.submitting") : lineCount > 1 ? t("add.submit_multi", { count: lineCount }) : t("add.submit")}
      </button>
    </form>
  );
}
