import { useEffect, useState } from "preact/hooks";
import { Router, type Route } from "./router.ts";

export const router = new Router();

/** Reactive current `Route`, plus a stable `navigate` function. */
export function useRoute(): [Route, (route: Route) => void] {
  const [route, setRoute] = useState<Route>(router.route);
  useEffect(() => router.subscribe(setRoute), []);
  return [route, (r: Route) => router.navigate(r)];
}
