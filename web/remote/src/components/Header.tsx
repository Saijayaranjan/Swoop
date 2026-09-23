import { globalStatsStore } from "../state/tasks.ts";
import { connectionStatusStore } from "../state/ws.ts";
import { useStore } from "../state/useStore.ts";
import { useT } from "../i18n/useT.ts";
import { formatSpeed } from "../utils/format.ts";
import { Icon } from "./Icon.tsx";

export function Header() {
  const stats = useStore(globalStatsStore);
  const status = useStore(connectionStatusStore);
  const t = useT();

  return (
    <header class="app-header">
      <div class="app-header-brand">{t("app.name")}</div>
      <div class="app-header-speeds" aria-live="polite" aria-atomic="true">
        <span class="speed-chip" title={t("app.download_speed")}>
          <Icon name="download" size={16} />
          {formatSpeed(stats.download_speed)}
        </span>
        <span class="speed-chip" title={t("app.upload_speed")}>
          <Icon name="upload" size={16} />
          {formatSpeed(stats.upload_speed)}
        </span>
      </div>
      {status !== "open" && (
        <div class="app-header-status" role="status">
          <Icon name="wifi-off" size={16} />
          <span>{status === "reconnecting" || status === "connecting" ? t("app.reconnecting") : t("app.offline")}</span>
        </div>
      )}
    </header>
  );
}
