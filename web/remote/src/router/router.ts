// Minimal hash-based router. Pure parsing/building functions are exported separately from the
// browser-integrated `Router` class so route logic is testable without a DOM.

export type ViewName = "pair" | "downloads" | "detail" | "add" | "dashboard" | "speed" | "settings";

export interface Route {
  view: ViewName;
  taskId: string | null;
}

const KNOWN_VIEWS: readonly ViewName[] = ["pair", "downloads", "detail", "add", "dashboard", "speed", "settings"];

export const DEFAULT_ROUTE: Route = { view: "downloads", taskId: null };

function isViewName(s: string): s is ViewName {
  return (KNOWN_VIEWS as readonly string[]).includes(s);
}

/**
 * Parse a location hash into a `Route`. Accepted shapes:
 *   "#/downloads"          -> { view: "downloads", taskId: null }
 *   "#/downloads/abc-123"  -> { view: "detail", taskId: "abc-123" }
 *   "#/add", "#/dashboard", "#/speed", "#/settings", "#/pair"
 *   ""  or an unknown path -> DEFAULT_ROUTE
 */
export function parseHash(hash: string): Route {
  const trimmed = hash.replace(/^#/, "");
  const parts = trimmed.split("/").filter((p) => p.length > 0);
  if (parts.length === 0) {
    return DEFAULT_ROUTE;
  }
  const [first, second] = parts;
  if (first === undefined || !isViewName(first)) {
    return DEFAULT_ROUTE;
  }
  if (first === "downloads" && second !== undefined) {
    return { view: "detail", taskId: decodeURIComponent(second) };
  }
  return { view: first, taskId: null };
}

/** Build the location hash for a route (inverse of `parseHash`). */
export function buildHash(route: Route): string {
  if (route.view === "detail" && route.taskId !== null) {
    return `#/downloads/${encodeURIComponent(route.taskId)}`;
  }
  return `#/${route.view}`;
}

type Listener = (route: Route) => void;

/** Browser-integrated router: reads/writes `window.location.hash` and notifies subscribers. */
export class Router {
  private listeners = new Set<Listener>();
  private current: Route;

  constructor() {
    this.current = typeof window === "undefined" ? DEFAULT_ROUTE : parseHash(window.location.hash);
    if (typeof window !== "undefined") {
      window.addEventListener("hashchange", this.handleHashChange);
    }
  }

  private handleHashChange = (): void => {
    this.current = parseHash(window.location.hash);
    for (const listener of this.listeners) {
      listener(this.current);
    }
  };

  get route(): Route {
    return this.current;
  }

  navigate(route: Route): void {
    const hash = buildHash(route);
    if (typeof window === "undefined") {
      this.current = route;
      return;
    }
    if (window.location.hash === hash) {
      // Same route: still notify (e.g. re-opening the same detail sheet).
      this.current = route;
      for (const listener of this.listeners) {
        listener(this.current);
      }
      return;
    }
    window.location.hash = hash;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  dispose(): void {
    if (typeof window !== "undefined") {
      window.removeEventListener("hashchange", this.handleHashChange);
    }
    this.listeners.clear();
  }
}
