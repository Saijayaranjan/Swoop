// Single shared ApiClient wired to the auth store: every request carries the current token, and
// any 401 clears auth so the app falls back to the Pair screen (the "on 401 anywhere" rule).

import { ApiClient } from "./client.ts";
import { authStore, clearAuth } from "../state/auth.ts";

export const apiClient = new ApiClient({
  getToken: () => authStore.getState().token,
  onUnauthorized: () => clearAuth(),
});
