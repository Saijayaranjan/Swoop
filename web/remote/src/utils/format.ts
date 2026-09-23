// Human-friendly formatting helpers. Pure functions so they are trivially testable.

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

/** Format a byte count as e.g. "1.5 MB". `0` renders as "0 B". Negative values clamp to 0. */
export function formatBytes(bytes: number, fractionDigits = 1): string {
  const n = Number.isFinite(bytes) && bytes > 0 ? bytes : 0;
  if (n < 1024) {
    return `${Math.round(n)} B`;
  }
  let value = n;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < BYTE_UNITS.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const digits = value >= 100 ? 0 : fractionDigits;
  return `${value.toFixed(digits)} ${BYTE_UNITS[unitIndex]}`;
}

/** Format a byte-per-second rate as e.g. "3.2 MB/s". */
export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/** Format a duration in seconds as "3d 4h", "2h 15m", "5m 30s" or "12s". `null`/negative -> "--". */
export function formatEta(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) {
    return "--";
  }
  const total = Math.floor(seconds);
  if (total === 0) {
    return "0s";
  }
  const days = Math.floor(total / 86400);
  const hours = Math.floor((total % 86400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;

  if (days > 0) {
    return `${days}d ${hours}h`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  if (minutes > 0) {
    return `${minutes}m ${secs}s`;
  }
  return `${secs}s`;
}

/** Format a percentage (0..100) with no decimals, clamped to [0, 100]. */
export function formatPercent(percent: number | null | undefined): string {
  if (percent === null || percent === undefined || !Number.isFinite(percent)) {
    return "--";
  }
  const clamped = Math.min(100, Math.max(0, percent));
  return `${Math.round(clamped)}%`;
}

/** Format a `Millis` timestamp as a locale-aware short date/time string. */
export function formatTimestamp(millis: number | null | undefined): string {
  if (millis === null || millis === undefined || !Number.isFinite(millis)) {
    return "--";
  }
  const date = new Date(millis);
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Format a relative "time ago" string for recent timestamps, falling back to a date. */
export function formatRelativeTime(millis: number | null | undefined, now = Date.now()): string {
  if (millis === null || millis === undefined || !Number.isFinite(millis)) {
    return "--";
  }
  const diffSeconds = Math.round((now - millis) / 1000);
  if (diffSeconds < 5) {
    return "just now";
  }
  if (diffSeconds < 60) {
    return `${diffSeconds}s ago`;
  }
  if (diffSeconds < 3600) {
    return `${Math.floor(diffSeconds / 60)}m ago`;
  }
  if (diffSeconds < 86400) {
    return `${Math.floor(diffSeconds / 3600)}h ago`;
  }
  return formatTimestamp(millis);
}
