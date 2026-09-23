// A minimal external store (no state-management dependency): plain get/set/subscribe, paired
// with `useStore` (preact/hooks) for components to read it reactively.

export type Listener<T> = (state: T) => void;
export type Updater<T> = T | ((prev: T) => T);

export interface Store<T> {
  getState: () => T;
  setState: (updater: Updater<T>) => void;
  subscribe: (listener: Listener<T>) => () => void;
}

export function createStore<T>(initial: T): Store<T> {
  let state = initial;
  const listeners = new Set<Listener<T>>();

  const getState = (): T => state;

  const setState = (updater: Updater<T>): void => {
    const next = typeof updater === "function" ? (updater as (prev: T) => T)(state) : updater;
    if (Object.is(next, state)) {
      return;
    }
    state = next;
    for (const listener of listeners) {
      listener(state);
    }
  };

  const subscribe = (listener: Listener<T>): (() => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  };

  return { getState, setState, subscribe };
}
