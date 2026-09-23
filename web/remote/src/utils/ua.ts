// Best-effort default device name derived from the user agent, used to pre-fill the pairing
// form's "Device name" field (the user can always override it).

export function defaultDeviceName(userAgent: string): string {
  const ua = userAgent;
  if (/iPad/.test(ua)) return "iPad";
  if (/iPhone/.test(ua)) return "iPhone";
  if (/Android/.test(ua)) {
    const model = /Android [^;]+;\s*([^)]+?)(?:\s+Build|\))/.exec(ua)?.[1];
    return model ? `Android (${model.trim()})` : "Android device";
  }
  if (/Macintosh/.test(ua)) return "Mac";
  if (/Windows/.test(ua)) return "Windows PC";
  if (/CrOS/.test(ua)) return "Chromebook";
  if (/Linux/.test(ua)) return "Linux PC";
  return "Browser";
}

export function defaultDeviceKind(userAgent: string): string {
  if (/iPad|Android(?!.*Mobile)|Tablet/.test(userAgent)) return "tablet";
  if (/iPhone|Android.*Mobile|Mobile/.test(userAgent)) return "phone";
  return "computer";
}
