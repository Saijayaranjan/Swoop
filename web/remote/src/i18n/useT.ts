import { useMemo } from "preact/hooks";
import { useStore } from "../state/useStore.ts";
import { settingsStore } from "../state/settings.ts";
import { createTranslator, type TranslationKey } from "./index.ts";

export type TFunction = (key: TranslationKey, vars?: Record<string, string | number>) => string;

/** Reactive `t()` bound to the current locale in settings. */
export function useT(): TFunction {
  const settings = useStore(settingsStore);
  return useMemo(() => createTranslator(settings.locale), [settings.locale]);
}
