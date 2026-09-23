// API error mapping. The server returns `{"error": {"type": ..., "message": ...}}` alongside a
// matching HTTP status (docs/api/rest.md). This module turns any failure -- a well-formed error
// envelope, an unparsable body, or a network failure -- into one `ApiError` shape the UI can
// branch on.

import type { ApiErrorBody } from "./types.ts";

export type ApiErrorKind = ApiErrorBody["error"]["type"] | "network" | "parse";

export class ApiError extends Error {
  readonly status: number;
  readonly kind: ApiErrorKind;

  constructor(status: number, kind: ApiErrorKind, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.kind = kind;
  }

  get isUnauthorized(): boolean {
    return this.kind === "unauthorized" || this.status === 401;
  }
}

function isApiErrorBody(value: unknown): value is ApiErrorBody {
  if (typeof value !== "object" || value === null || !("error" in value)) {
    return false;
  }
  const err = (value as { error: unknown }).error;
  return (
    typeof err === "object" &&
    err !== null &&
    "type" in err &&
    "message" in err &&
    typeof (err as { type: unknown }).type === "string" &&
    typeof (err as { message: unknown }).message === "string"
  );
}

/** Map a non-OK HTTP response's already-parsed JSON body (or `undefined` if parsing failed) to an ApiError. */
export function mapResponseError(status: number, body: unknown, parseFailed: boolean): ApiError {
  if (parseFailed) {
    return new ApiError(status, "parse", `Request failed with status ${status} (unparsable response body)`);
  }
  if (isApiErrorBody(body)) {
    return new ApiError(status, body.error.type, body.error.message);
  }
  return new ApiError(status, "internal", `Request failed with status ${status}`);
}

/** Map a thrown error from `fetch` itself (offline, DNS failure, aborted, CORS, ...) to an ApiError. */
export function mapNetworkError(cause: unknown): ApiError {
  const message = cause instanceof Error ? cause.message : String(cause);
  return new ApiError(0, "network", message || "Network request failed");
}
