/** Pure formatting helpers used across the popup/options UI. No browser globals. */

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const;

export function formatBytes(bytes: number | null | undefined, fractionDigits = 1): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return '–';
  if (bytes < 0) return '–';
  if (bytes === 0) return '0 B';
  const exponent = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    BYTE_UNITS.length - 1,
  );
  const value = bytes / Math.pow(1024, exponent);
  const digits = exponent === 0 ? 0 : fractionDigits;
  return `${value.toFixed(digits)} ${BYTE_UNITS[exponent] as string}`;
}

export function formatSpeed(bytesPerSecond: number | null | undefined): string {
  if (bytesPerSecond === null || bytesPerSecond === undefined) return '–';
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return '–';
  if (seconds < 0) return '–';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 60) return `${totalMinutes}m`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours < 24) return `${hours}h ${minutes}m`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return `${days}d ${remHours}h`;
}

export function formatPercent(downloaded: number, total: number | null | undefined): string {
  if (!total || total <= 0) return '–';
  const pct = Math.min(100, Math.max(0, (downloaded / total) * 100));
  return `${pct.toFixed(pct >= 10 ? 0 : 1)}%`;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) {
    return '–';
  }
  const s = Math.round(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number): string => n.toString().padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

export function truncateMiddle(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  const keep = maxLength - 1;
  const head = Math.ceil(keep / 2);
  const tail = Math.floor(keep / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - tail)}`;
}

/** Bytes to three significant digits, the way the desktop app shows them: `42.3 MB`, `1.15 GB`,
 *  `578 MB`. Returns the number and unit separately. */
export function bytesParts(bytes: number): { value: string; unit: string } {
  if (!Number.isFinite(bytes) || bytes <= 0) return { value: '0', unit: 'B' };
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  const scaled = bytes / Math.pow(1024, exponent);
  const value = exponent === 0 ? String(Math.round(scaled)) : String(Number(scaled.toPrecision(3)));
  return { value, unit: BYTE_UNITS[exponent] as string };
}

export function formatBytesCompact(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes) || bytes < 0) return '–';
  const { value, unit } = bytesParts(bytes);
  return `${value} ${unit}`;
}

export function formatSpeedCompact(bytesPerSecond: number | null | undefined): string {
  if (!bytesPerSecond || bytesPerSecond <= 0) return '0 KB/s';
  return `${formatBytesCompact(bytesPerSecond)}/s`;
}

/** "1.43 of 2.31 GB" when both share a unit, otherwise "578 MB of 3.4 GB". */
export function formatProgressSize(downloaded: number, total: number, of = 'of'): string {
  const a = bytesParts(downloaded);
  const b = bytesParts(total);
  if (a.unit === b.unit) return `${a.value} ${of} ${b.value} ${b.unit}`;
  return `${a.value} ${a.unit} ${of} ${b.value} ${b.unit}`;
}

/** Splits a speed into its number and unit (`{ value: "42.3", unit: "MB/s" }`) so the number can
 *  be set large and the unit small. Zero/unknown reads as `0 KB/s` rather than a dash. */
export function splitSpeed(bytesPerSecond: number | null | undefined): { value: string; unit: string } {
  if (!bytesPerSecond || bytesPerSecond < 0 || !Number.isFinite(bytesPerSecond)) {
    return { value: '0', unit: 'KB/s' };
  }
  const { value, unit } = bytesParts(bytesPerSecond);
  return { value, unit: `${unit}/s` };
}
