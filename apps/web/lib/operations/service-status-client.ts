import {
  parseServiceStatusResponse,
  type ServiceStatusResponse,
} from "@/lib/operations/service-status-contract";

export const SERVICE_STATUS_API_PATH = "/api/v1/admin/service-status";
const MAX_RESPONSE_BYTES = 256 * 1024;
const MAX_TIMER_DELAY_MS = 2_147_483_647;

export type ServiceStatusRequestErrorKind = "denied" | "throttled" | "unavailable";

export class ServiceStatusRequestError extends Error {
  readonly kind: ServiceStatusRequestErrorKind;
  readonly retryAfterMs: number | null;

  constructor(kind: ServiceStatusRequestErrorKind, retryAfterMs: number | null = null) {
    super("Service status is unavailable");
    this.name = "ServiceStatusRequestError";
    this.kind = kind;
    this.retryAfterMs = retryAfterMs;
  }
}

function invalidResponse(): ServiceStatusRequestError {
  return new ServiceStatusRequestError("unavailable");
}

export function retryAfterMilliseconds(
  value: string | null,
  serverDate: string | null,
  fallbackNow = Date.now(),
): number | null {
  if (value === null) return null;
  const trimmed = value.trim();
  if (/^\d+$/.test(trimmed)) {
    const seconds = Number(trimmed);
    if (!Number.isFinite(seconds)) return null;
    return Math.min(MAX_TIMER_DELAY_MS, seconds * 1_000);
  }

  const retryAt = Date.parse(trimmed);
  if (!Number.isFinite(retryAt)) return null;
  const serverNow = serverDate === null ? fallbackNow : Date.parse(serverDate);
  if (!Number.isFinite(serverNow)) return null;
  return Math.min(MAX_TIMER_DELAY_MS, Math.max(0, retryAt - serverNow));
}

async function readBoundedJson(response: Response): Promise<unknown> {
  const contentLength = response.headers.get("Content-Length");
  if (contentLength !== null && /^\d+$/.test(contentLength)) {
    if (Number(contentLength) > MAX_RESPONSE_BYTES) return invalidResponse();
  }
  if (response.body === null) return invalidResponse();

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (value === undefined) return invalidResponse();
      length += value.byteLength;
      if (length > MAX_RESPONSE_BYTES) {
        await reader.cancel();
        return invalidResponse();
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }

  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return invalidResponse();
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return invalidResponse();
  }
}

export async function requestServiceStatus(signal: AbortSignal): Promise<ServiceStatusResponse> {
  let response: Response;
  try {
    response = await fetch(SERVICE_STATUS_API_PATH, {
      method: "GET",
      cache: "no-store",
      credentials: "same-origin",
      headers: { Accept: "application/json" },
      signal,
    });
  } catch {
    throw new ServiceStatusRequestError("unavailable");
  }

  if (response.status === 401 || response.status === 403) {
    throw new ServiceStatusRequestError("denied");
  }
  if (response.status === 429) {
    throw new ServiceStatusRequestError(
      "throttled",
      retryAfterMilliseconds(response.headers.get("Retry-After"), response.headers.get("Date")),
    );
  }
  if (!response.ok) throw new ServiceStatusRequestError("unavailable");

  const body = await readBoundedJson(response);
  try {
    return parseServiceStatusResponse(body);
  } catch {
    throw new ServiceStatusRequestError("unavailable");
  }
}
