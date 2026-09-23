import { useEffect, useState } from "preact/hooks";
import type { Store } from "./store.ts";

/** Subscribe a component to a `Store`, re-rendering whenever its state changes. */
export function useStore<T>(store: Store<T>): T {
  const [state, setState] = useState<T>(store.getState());
  useEffect(() => {
    // State may have changed between render and effect commit; sync once, then subscribe.
    setState(store.getState());
    return store.subscribe(setState);
  }, [store]);
  return state;
}
