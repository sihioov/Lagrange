import { AUTH_API_PATHS, csrfTokenSchema } from "@/lib/api/contracts";
import { withCsrfMutationLock } from "@/lib/api/csrf-mutation-lock";
import { IntradayStreamContractError, intradayStreamUuidSchema } from "./intraday-stream-contracts";
import {
  INTRADAY_STREAM_LEASE_PATH,
  type IntradayStreamLease,
  type IntradayStreamLeaseRequest,
  type IntradayStreamReleaseRequest,
  intradayStreamLeasePath,
  intradayStreamLeaseRequestSchema,
  intradayStreamReleaseRequestSchema,
  intradayStreamReleaseResponseSchema,
  matchStreamLease,
} from "./intraday-stream-lease-contracts";

export type IntradayStreamFailure =
  | "unauthenticated"
  | "forbidden"
  | "not_found"
  | "conflict"
  | "feature_disabled"
  | "unavailable"
  | "invalid_response"
  | "timeout"
  | "aborted"
  | "network";

/** Never copy response messages, credentials, request bodies or prices into exceptions. */
export class IntradayStreamHttpError extends Error {
  override readonly name = "IntradayStreamHttpError";
  constructor(readonly kind: IntradayStreamFailure) {
    super(`Market stream request failed: ${kind}`);
  }
}

export type IntradayStreamMutationOptions = {
  readonly idempotencyKey: string;
  readonly signal: AbortSignal;
  readonly previousLeaseId?: string;
};

export type IntradayStreamClient = {
  replaceLease: (
    request: IntradayStreamLeaseRequest,
    options: IntradayStreamMutationOptions,
  ) => Promise<IntradayStreamLease>;
  releaseLease: (
    leaseId: string,
    request: IntradayStreamReleaseRequest,
    options: IntradayStreamMutationOptions,
  ) => Promise<void>;
};

const MAX_JSON_BYTES = 16 * 1024;

async function jsonBody(response: Response, signal: AbortSignal): Promise<unknown> {
  const length = response.headers.get("Content-Length");
  if (
    response.headers.get("Content-Type")?.split(";", 1)[0]?.trim() !== "application/json" ||
    (length !== null && (!/^\d+$/.test(length) || Number(length) > MAX_JSON_BYTES)) ||
    response.body === null
  ) {
    await response.body?.cancel();
    throw new IntradayStreamContractError();
  }
  const reader = response.body.getReader();
  const cancel = (): void => {
    void reader.cancel().catch(() => undefined);
  };
  signal.addEventListener("abort", cancel, { once: true });
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let bytes = 0;
  let text = "";
  try {
    while (true) {
      signal.throwIfAborted();
      const chunk = await reader.read();
      signal.throwIfAborted();
      if (chunk.done) break;
      bytes += chunk.value.byteLength;
      if (bytes > MAX_JSON_BYTES) throw new IntradayStreamContractError();
      text += decoder.decode(chunk.value, { stream: true });
    }
    text += decoder.decode();
    return JSON.parse(text) as unknown;
  } finally {
    signal.removeEventListener("abort", cancel);
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

function responseFailure(status: number, input: unknown): IntradayStreamFailure {
  if (status === 401) return "unauthenticated";
  if (status === 403) return "forbidden";
  if (status === 404) return "not_found";
  if (status === 409) return "conflict";
  if (status === 503) {
    if (
      typeof input === "object" &&
      input !== null &&
      "error" in input &&
      typeof input.error === "object" &&
      input.error !== null &&
      "code" in input.error &&
      input.error.code === "FEATURE_DISABLED"
    )
      return "feature_disabled";
    return "unavailable";
  }
  return "invalid_response";
}

/** Uses the existing cookie + CSRF protocol with one deadline for preflight, mutation and body. */
export function createIntradayStreamClient(
  options: { readonly fetcher?: typeof fetch; readonly origin?: string } = {},
): IntradayStreamClient {
  let origin: string | undefined;
  if (options.origin !== undefined) {
    const parsed = new URL(options.origin);
    if (parsed.origin !== options.origin || !["http:", "https:"].includes(parsed.protocol))
      throw new IntradayStreamContractError();
    origin = parsed.origin;
  }
  const fetcher = options.fetcher ?? fetch;
  const url = (path: string): string => (origin === undefined ? path : `${origin}${path}`);

  async function mutation(
    path: string,
    method: "POST" | "DELETE",
    input: unknown,
    mutationOptions: IntradayStreamMutationOptions,
  ): Promise<unknown> {
    if (!intradayStreamUuidSchema.safeParse(mutationOptions.idempotencyKey).success)
      throw new IntradayStreamContractError();
    const body = JSON.stringify(input);
    if (new TextEncoder().encode(body).length > MAX_JSON_BYTES)
      throw new IntradayStreamContractError();
    const abort = new AbortController();
    const stop = (): void => abort.abort();
    mutationOptions.signal.addEventListener("abort", stop, { once: true });
    if (mutationOptions.signal.aborted) abort.abort();
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      abort.abort();
    }, 5_000);
    const init = {
      credentials: "same-origin",
      mode: "same-origin",
      redirect: "error",
      cache: "no-store",
      signal: abort.signal,
    } as const;
    try {
      return await withCsrfMutationLock(abort.signal, async () => {
        abort.signal.throwIfAborted();
        const csrf = await fetcher(url(AUTH_API_PATHS.csrf), { ...init, method: "GET" });
        const csrfBody = await jsonBody(csrf, abort.signal);
        if (csrf.status !== 200)
          throw new IntradayStreamHttpError(responseFailure(csrf.status, csrfBody));
        const parsed = csrfTokenSchema.safeParse(csrfBody);
        if (!parsed.success || parsed.data.csrf_token.length > 4_096)
          throw new IntradayStreamContractError();
        abort.signal.throwIfAborted();
        const response = await fetcher(url(path), {
          ...init,
          method,
          body,
          headers: {
            "Content-Type": "application/json",
            "X-CSRF-Token": parsed.data.csrf_token,
            "Idempotency-Key": mutationOptions.idempotencyKey,
          },
        });
        const output = await jsonBody(response, abort.signal);
        if (response.status !== 200)
          throw new IntradayStreamHttpError(responseFailure(response.status, output));
        return output;
      });
    } catch (error) {
      if (timedOut) throw new IntradayStreamHttpError("timeout");
      if (mutationOptions.signal.aborted) throw new IntradayStreamHttpError("aborted");
      if (error instanceof IntradayStreamHttpError) throw error;
      if (error instanceof IntradayStreamContractError || error instanceof SyntaxError)
        throw new IntradayStreamHttpError("invalid_response");
      throw new IntradayStreamHttpError("network");
    } finally {
      clearTimeout(timer);
      mutationOptions.signal.removeEventListener("abort", stop);
    }
  }

  return {
    async replaceLease(request, mutationOptions) {
      const parsed = intradayStreamLeaseRequestSchema.safeParse(request);
      if (!parsed.success) throw new IntradayStreamContractError();
      const output = await mutation(
        INTRADAY_STREAM_LEASE_PATH,
        "POST",
        parsed.data,
        mutationOptions,
      );
      return matchStreamLease(output, parsed.data, mutationOptions.previousLeaseId);
    },
    async releaseLease(leaseId, request, mutationOptions) {
      const path = intradayStreamLeasePath(leaseId);
      const parsed = intradayStreamReleaseRequestSchema.safeParse(request);
      if (!parsed.success) throw new IntradayStreamContractError();
      const output = intradayStreamReleaseResponseSchema.safeParse(
        await mutation(path, "DELETE", parsed.data, mutationOptions),
      );
      if (!output.success || output.data.lease_id !== leaseId)
        throw new IntradayStreamContractError();
    },
  };
}
