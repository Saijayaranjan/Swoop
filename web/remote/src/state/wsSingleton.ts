// Wires the WebSocket EventsClient's lifecycle to auth (connect once paired, disconnect on
// forget/401) and to the "high-volume events" setting (reconnect so the server-side filter
// takes effect immediately when the user flips it in Settings).

import { apiClient } from "../api/singleton.ts";
import { authStore } from "./auth.ts";
import { settingsStore } from "./settings.ts";
import { EventsClient } from "./ws.ts";

export const eventsClient = new EventsClient(
  apiClient,
  () => authStore.getState().token,
  () => settingsStore.getState().highVolumeEvents,
);

let started = false;
let lastHighVolume = settingsStore.getState().highVolumeEvents;

export function initEventsLifecycle(): void {
  authStore.subscribe((state) => {
    if (state.token && !started) {
      started = true;
      eventsClient.start();
    } else if (!state.token && started) {
      started = false;
      eventsClient.stop();
    }
  });

  settingsStore.subscribe((state) => {
    if (state.highVolumeEvents !== lastHighVolume) {
      lastHighVolume = state.highVolumeEvents;
      if (started) {
        eventsClient.stop();
        started = true;
        eventsClient.start();
      }
    }
  });

  if (authStore.getState().token) {
    started = true;
    eventsClient.start();
  }
}
