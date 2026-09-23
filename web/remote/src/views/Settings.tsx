import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { pushToast } from "../state/toast.ts";
import { authStore, clearAuth, setAuth } from "../state/auth.ts";
import { setHighVolumeEvents, setLocale, setTheme, settingsStore } from "../state/settings.ts";
import { useStore } from "../state/useStore.ts";
import { SegmentedControl } from "../components/SegmentedControl.tsx";
import { ConfirmDialog } from "../components/ConfirmDialog.tsx";
import { SUPPORTED_LOCALES, type Locale } from "../i18n/index.ts";
import type { EngineInfo } from "../api/types.ts";
import type { Theme } from "../state/settings.ts";

const LOCALE_LABEL: Record<Locale, string> = { en: "English", hi: "हिन्दी", ta: "தமிழ்" };

function formatUptime(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function Settings() {
  const t = useT();
  const settings = useStore(settingsStore);
  const auth = useStore(authStore);
  const [info, setInfo] = useState<EngineInfo | null>(null);
  const [deviceName, setDeviceName] = useState(auth.device?.name ?? "");
  const [confirmingForget, setConfirmingForget] = useState(false);

  useEffect(() => {
    apiClient.info().then(setInfo).catch(() => {});
  }, []);

  useEffect(() => {
    setDeviceName(auth.device?.name ?? "");
  }, [auth.device?.name]);

  async function saveDeviceName(): Promise<void> {
    if (!auth.device || deviceName.trim() === "" || deviceName === auth.device.name) return;
    try {
      await apiClient.renameDevice(auth.device.id, deviceName.trim());
      setAuth(auth.token as string, { id: auth.device.id, name: deviceName.trim() });
      pushToast(t("action.save"), "success");
    } catch (err) {
      pushToast(err instanceof ApiError ? err.message : String(err), "error");
    }
  }

  async function forgetDevice(): Promise<void> {
    setConfirmingForget(false);
    if (auth.device) {
      try {
        await apiClient.forgetDevice(auth.device.id);
      } catch {
        // best-effort: still forget locally even if the server call fails (e.g. token already revoked)
      }
    }
    clearAuth();
  }

  return (
    <div class="view view-settings">
      <h1 class="view-title">{t("settings.title")}</h1>

      <section class="settings-section">
        <h2>{t("settings.server_section")}</h2>
        {info ? (
          <dl class="detail-list">
            <div>
              <dt>{t("settings.server_version")}</dt>
              <dd>{info.version}</dd>
            </div>
            <div>
              <dt>{t("settings.server_os")}</dt>
              <dd>
                {info.os} ({info.arch})
              </dd>
            </div>
            <div>
              <dt>{t("settings.server_uptime")}</dt>
              <dd>{formatUptime(info.uptime_seconds)}</dd>
            </div>
          </dl>
        ) : (
          <p class="empty-state">{t("common.loading")}</p>
        )}
      </section>

      <section class="settings-section">
        <h2>{t("settings.device_section")}</h2>
        <label class="field">
          <span>{t("settings.device_name_label")}</span>
          <div class="limit-input">
            <input type="text" value={deviceName} onInput={(e) => setDeviceName((e.currentTarget as HTMLInputElement).value)} />
            <button type="button" class="button" onClick={saveDeviceName}>
              {t("action.save")}
            </button>
          </div>
        </label>
        <label class="checkbox-field">
          <input
            type="checkbox"
            checked={settings.highVolumeEvents}
            onChange={(e) => setHighVolumeEvents((e.currentTarget as HTMLInputElement).checked)}
          />
          {t("settings.high_volume_label")}
        </label>
        <p class="field-hint">{t("settings.high_volume_hint")}</p>
        <button type="button" class="button button-danger" onClick={() => setConfirmingForget(true)}>
          {t("action.forget_device")}
        </button>
      </section>

      <section class="settings-section">
        <h2>{t("settings.theme_label")}</h2>
        <SegmentedControl
          ariaLabel={t("settings.theme_label")}
          value={settings.theme}
          onChange={(v) => setTheme(v as Theme)}
          options={[
            { value: "system", label: t("settings.theme_system") },
            { value: "light", label: t("settings.theme_light") },
            { value: "dark", label: t("settings.theme_dark") },
          ]}
        />
      </section>

      <section class="settings-section">
        <h2>{t("settings.language_label")}</h2>
        <select value={settings.locale} onChange={(e) => setLocale((e.currentTarget as HTMLSelectElement).value as Locale)}>
          {SUPPORTED_LOCALES.map((loc) => (
            <option key={loc} value={loc}>
              {LOCALE_LABEL[loc]}
            </option>
          ))}
        </select>
      </section>

      {confirmingForget && (
        <ConfirmDialog
          title={t("action.forget_device")}
          onCancel={() => setConfirmingForget(false)}
          actions={
            <>
              <button type="button" class="button" onClick={() => setConfirmingForget(false)}>
                {t("action.cancel_dialog")}
              </button>
              <button type="button" class="button button-danger" onClick={forgetDevice}>
                {t("action.forget_device")}
              </button>
            </>
          }
        >
          <p>{t("settings.forget_device_confirm")}</p>
        </ConfirmDialog>
      )}
    </div>
  );
}
