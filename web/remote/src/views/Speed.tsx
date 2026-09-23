import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { pushToast } from "../state/toast.ts";
import { SegmentedControl } from "../components/SegmentedControl.tsx";
import { globalStatsStore } from "../state/tasks.ts";
import { useStore } from "../state/useStore.ts";
import type { TrafficMode } from "../api/types.ts";

const MODES: TrafficMode[] = ["unlimited", "balanced", "browsing", "custom"];

export function Speed() {
  const t = useT();
  const stats = useStore(globalStatsStore);
  const [mode, setMode] = useState<TrafficMode>(stats.traffic_mode);
  const [downloadLimit, setDownloadLimit] = useState(Math.round(stats.download_limit / 1024));
  const [uploadLimit, setUploadLimit] = useState(Math.round(stats.upload_limit / 1024));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setMode(stats.traffic_mode);
  }, [stats.traffic_mode]);

  async function applyMode(next: TrafficMode): Promise<void> {
    setMode(next);
    if (next === "custom") return; // wait for explicit Apply with limits
    try {
      await apiClient.setTrafficMode(next);
      pushToast(t("speed.saved"), "success");
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  async function applyCustom(): Promise<void> {
    setSaving(true);
    try {
      await apiClient.setTrafficMode("custom");
      await apiClient.setLimits(downloadLimit * 1024, uploadLimit * 1024);
      pushToast(t("speed.saved"), "success");
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div class="view view-speed">
      <h1 class="view-title">{t("speed.title")}</h1>

      <SegmentedControl
        ariaLabel={t("speed.title")}
        value={mode}
        onChange={applyMode}
        options={MODES.map((m) => ({ value: m, label: t(`speed.mode_${m}` as "speed.mode_unlimited") }))}
      />

      {mode === "custom" && (
        <div class="speed-custom">
          <label class="field">
            <span>{t("speed.download_limit")}</span>
            <div class="limit-input">
              <input
                type="number"
                min={0}
                value={downloadLimit}
                onInput={(e) => setDownloadLimit(Number((e.currentTarget as HTMLInputElement).value) || 0)}
              />
              <span>{t("speed.limit_unit")}</span>
            </div>
          </label>
          <label class="field">
            <span>{t("speed.upload_limit")}</span>
            <div class="limit-input">
              <input
                type="number"
                min={0}
                value={uploadLimit}
                onInput={(e) => setUploadLimit(Number((e.currentTarget as HTMLInputElement).value) || 0)}
              />
              <span>{t("speed.limit_unit")}</span>
            </div>
          </label>
          <p class="field-hint">0 = {t("speed.limit_unlimited")}</p>
          <button type="button" class="button button-primary" onClick={applyCustom} disabled={saving}>
            {t("speed.apply")}
          </button>
        </div>
      )}
    </div>
  );
}
