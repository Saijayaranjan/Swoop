// A tiny hand-rolled icon set (no icon font / icon library dependency). Each icon is a stroke
// path on a 0 0 24 24 canvas, rendered in `currentColor` so it follows the surrounding text/theme.

export type IconName =
  | "pause"
  | "play"
  | "retry"
  | "close"
  | "trash"
  | "more"
  | "search"
  | "chevron-right"
  | "chevron-down"
  | "plus"
  | "check"
  | "download"
  | "upload"
  | "wifi-off"
  | "settings"
  | "speed"
  | "dashboard"
  | "list"
  | "folder"
  | "file"
  | "magnet"
  | "warning"
  | "info";

const PATHS: Record<IconName, string> = {
  pause: "M8 5v14M16 5v14",
  play: "M7 4l13 8-13 8V4z",
  retry: "M4 4v6h6M20 20v-6h-6M4.5 15a8 8 0 0 0 14.5 2.5M19.5 9A8 8 0 0 0 5 6.5",
  close: "M6 6l12 12M18 6L6 18",
  trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-4.3-4.3",
  "chevron-right": "M9 5l7 7-7 7",
  "chevron-down": "M5 9l7 7 7-7",
  plus: "M12 5v14M5 12h14",
  check: "M5 13l4 4L19 7",
  download: "M12 3v12m0 0l-4-4m4 4l4-4M4 19h16",
  upload: "M12 21V9m0 0l-4 4m4-4l4 4M4 5h16",
  "wifi-off": "M2 2l20 20M8.5 16.5a5 5 0 0 1 7 0M5 12.5a10 10 0 0 1 3.5-2.3M19 12.5a10 10 0 0 0-3-2.1M12 20h.01",
  settings:
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 13.5a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6V20a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.6-1H4a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H10a1.7 1.7 0 0 0 1-1.6V4a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V10a1.7 1.7 0 0 0 1.6 1H20a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.6 1z",
  speed: "M12 12l4-4M12 21a9 9 0 1 1 9-9M12 21a9 9 0 0 1-6.4-2.6M5 12H3M12 5V3M19 12h2",
  dashboard: "M4 4h7v7H4zM13 4h7v4h-7zM13 11h7v9h-7zM4 14h7v7H4z",
  list: "M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01",
  folder: "M3 7a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7z",
  file: "M6 3h9l5 5v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5",
  magnet:
    "M6 4v7a6 6 0 0 0 12 0V4M6 4H3v7a9 9 0 0 0 18 0V4h-3M6 8h3M15 8h3",
  warning: "M12 3l10 18H2L12 3zM12 10v4M12 17h.01",
  info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v6M12 7h.01",
};

export interface IconProps {
  name: IconName;
  size?: number;
  class?: string;
  title?: string;
}

export function Icon({ name, size = 20, class: className, title }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      class={className}
      role={title ? "img" : "presentation"}
      aria-hidden={title ? undefined : "true"}
    >
      {title ? <title>{title}</title> : null}
      <path d={PATHS[name]} />
    </svg>
  );
}
