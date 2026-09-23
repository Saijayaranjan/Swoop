import { useState } from "preact/hooks";
import { useT } from "../i18n/useT.ts";
import { apiClient } from "../api/singleton.ts";
import { ApiError } from "../api/errors.ts";
import { setAuth } from "../state/auth.ts";
import { formatPairingCode, isValidPairingCode } from "../utils/pairingCode.ts";
import { defaultDeviceKind, defaultDeviceName } from "../utils/ua.ts";
import { Icon } from "../components/Icon.tsx";

export function Pair() {
  const t = useT();
  const [code, setCode] = useState("");
  const [deviceName, setDeviceName] = useState(() => (typeof navigator !== "undefined" ? defaultDeviceName(navigator.userAgent) : "Browser"));
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: Event): Promise<void> {
    e.preventDefault();
    if (!isValidPairingCode(code)) {
      setError(t("pair.error.generic"));
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const kind = typeof navigator !== "undefined" ? defaultDeviceKind(navigator.userAgent) : "computer";
      const result = await apiClient.pair(code, deviceName.trim() || "Remote device", kind);
      setAuth(result.token, { id: result.device.id, name: result.device.name });
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.isUnauthorized) setError(t("pair.error.unauthorized"));
        else if (err.kind === "rate_limited") setError(t("pair.error.rate_limited"));
        else setError(err.message);
      } else {
        setError(t("pair.error.generic"));
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div class="pair-screen">
      <form class="pair-card" onSubmit={submit}>
        <h1>{t("pair.title")}</h1>
        <p class="pair-subtitle">{t("pair.subtitle")}</p>

        <label class="field">
          <span>{t("pair.code_label")}</span>
          <input
            type="text"
            inputMode="text"
            autoCapitalize="characters"
            autoComplete="off"
            spellcheck={false}
            placeholder={t("pair.code_placeholder")}
            value={code}
            maxLength={9}
            class="pair-code-input"
            onInput={(e) => setCode(formatPairingCode((e.currentTarget as HTMLInputElement).value))}
          />
        </label>

        <label class="field">
          <span>{t("pair.device_name_label")}</span>
          <input type="text" value={deviceName} onInput={(e) => setDeviceName((e.currentTarget as HTMLInputElement).value)} />
        </label>

        {error && (
          <div class="form-error" role="alert">
            {error}
          </div>
        )}

        <button type="submit" class="button button-primary" disabled={submitting || !isValidPairingCode(code)}>
          {submitting ? t("pair.pairing") : t("pair.submit")}
        </button>

        <p class="pair-hint">
          <Icon name="info" size={14} />
          {t("pair.scan_hint")}
        </p>
        <p class="pair-hint pair-fingerprint-hint">{t("pair.fingerprint_hint")}</p>
      </form>
    </div>
  );
}
