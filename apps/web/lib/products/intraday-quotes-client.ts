import { z } from "zod";
import { type BrowserClientOptions, mutateWithCsrf } from "@/lib/api/browser-client";
import type { ProductMutationPath } from "@/lib/api/contracts";
import { ApiContractError } from "@/lib/api/response";
import {
  INTRADAY_QUOTE_DEMAND_PATH,
  type IntradayQuoteDemandRequest,
  type IntradayQuoteDemandResponse,
  type IntradayQuoteIdentity,
  type IntradayQuoteReleaseRequest,
  type IntradayQuoteResponse,
  intradayQuoteDemandPath,
  intradayQuoteDemandRequestSchema,
  intradayQuoteDemandResponseSchema,
  intradayQuoteIdentitySchema,
  intradayQuotePath,
  intradayQuoteReleaseRequestSchema,
  parseIntradayQuoteResponse,
} from "./intraday-quotes-contracts";

export const INTRADAY_QUOTE_API_ERROR_CODES = [
  "SESSION_UNKNOWN",
  "SESSION_EXPIRED",
  "FORBIDDEN",
  "CSRF_DENIED",
  "RESOURCE_NOT_FOUND",
  "INVALID_PARAMETER",
  "IDEMPOTENCY_MISMATCH",
  "QUOTE_DEMAND_SEQUENCE_CONFLICT",
  "QUOTE_DEMAND_CAPACITY",
  "QUOTE_CACHE_UNAVAILABLE",
] as const;

const INTRADAY_QUOTE_ERROR_STATUSES: Readonly<
  Record<IntradayQuoteApiErrorCode, readonly number[]>
> = {
  CSRF_DENIED: [403],
  FORBIDDEN: [403],
  IDEMPOTENCY_MISMATCH: [409],
  INVALID_PARAMETER: [400],
  QUOTE_CACHE_UNAVAILABLE: [503],
  QUOTE_DEMAND_CAPACITY: [429],
  QUOTE_DEMAND_SEQUENCE_CONFLICT: [409],
  RESOURCE_NOT_FOUND: [404],
  SESSION_EXPIRED: [401],
  SESSION_UNKNOWN: [401],
};

export type IntradayQuoteApiErrorCode = (typeof INTRADAY_QUOTE_API_ERROR_CODES)[number];

const intradayQuoteErrorEnvelopeSchema = z
  .object({
    error: z
      .object({
        code: z.enum(INTRADAY_QUOTE_API_ERROR_CODES),
        // The provider/API message is intentionally parsed only to validate the
        // envelope and is never copied into an exception or rendered UI.
        message: z.string(),
        request_id: z.string(),
        details: z.record(z.string(), z.unknown()).exactOptional(),
      })
      .strict(),
  })
  .strict();

export class IntradayQuoteApiError extends Error {
  override readonly name = "IntradayQuoteApiError";
  readonly code: IntradayQuoteApiErrorCode;
  readonly requestId: string;
  readonly status: number;
  readonly retryAfterMs: number | null;

  constructor(
    status: number,
    code: IntradayQuoteApiErrorCode,
    requestId: string,
    retryAfterMs: number | null,
  ) {
    super(`Intraday quote request failed with typed code ${code}`);
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.retryAfterMs = retryAfterMs;
  }
}

export type IntradayQuoteClientOptions = BrowserClientOptions & {
  readonly fetcher?: typeof fetch;
  readonly nowMs?: number;
  readonly origin?: string;
  readonly signal?: AbortSignal;
};

export type IntradayQuoteMutationOptions = IntradayQuoteClientOptions & {
  readonly idempotencyKey: string;
};

function requestUrl(path: string, origin: string | undefined): string {
  return origin === undefined ? path : new URL(path, origin).toString();
}

function isValidIdempotencyKey(value: string): boolean {
  return (
    value.length >= 1 && value.length <= 128 && /^[\x21-\x7e]+$/.test(value) && !/[:\\]/.test(value)
  );
}

export function assertIntradayQuoteIdempotencyKey(value: string): string {
  if (!isValidIdempotencyKey(value)) {
    throw new ApiContractError(400, "Invalid intraday quote idempotency key");
  }
  return value;
}

function retryAfterMs(response: Response): number | null {
  const value = response.headers.get("Retry-After");
  if (value === null || !/^\d+$/.test(value.trim())) return null;
  const seconds = Number.parseInt(value.trim(), 10);
  return Number.isSafeInteger(seconds) ? seconds * 1_000 : null;
}

function navigateToLogin(options: BrowserResponseOptions): void {
  if (options.navigate !== undefined) {
    options.navigate("/login");
    return;
  }
  if (typeof window !== "undefined") window.location.replace("/login");
}

type BrowserResponseOptions = Pick<BrowserClientOptions, "navigate">;

async function jsonBody(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    throw new ApiContractError(response.status, "Intraday quote response was not valid JSON");
  }
}

async function throwIntradayQuoteApiError(
  response: Response,
  options: IntradayQuoteClientOptions,
): Promise<never> {
  const body = await response
    .clone()
    .json()
    .catch(() => undefined);
  const parsed = intradayQuoteErrorEnvelopeSchema.safeParse(body);
  if (!parsed.success) {
    throw new ApiContractError(response.status, "Intraday quote error envelope was invalid");
  }
  const { code, request_id: requestId } = parsed.data.error;
  if (!INTRADAY_QUOTE_ERROR_STATUSES[code].includes(response.status)) {
    throw new ApiContractError(
      response.status,
      "Intraday quote error status did not match its code",
    );
  }
  if (code === "SESSION_UNKNOWN" || code === "SESSION_EXPIRED") navigateToLogin(options);
  throw new IntradayQuoteApiError(response.status, code, requestId, retryAfterMs(response));
}

async function parseMutationResponse<Output>(
  response: Response,
  status: number,
  schema: z.ZodType<Output>,
  options: IntradayQuoteClientOptions,
): Promise<Output> {
  if (response.status !== status) {
    if (!response.ok || response.status >= 400) await throwIntradayQuoteApiError(response, options);
    throw new ApiContractError(
      response.status,
      "Intraday quote mutation returned an unexpected status",
    );
  }
  if (status === 204) {
    if ((await response.text()) !== "") {
      throw new ApiContractError(response.status, "Intraday quote DELETE response was not empty");
    }
    return undefined as Output;
  }
  const parsed = schema.safeParse(await jsonBody(response));
  if (!parsed.success) {
    throw new ApiContractError(
      response.status,
      "Intraday quote mutation did not match its contract",
    );
  }
  return parsed.data;
}

function mutationPath(path: string): ProductMutationPath {
  return path as ProductMutationPath;
}

function mutationKey(options: IntradayQuoteMutationOptions): string {
  return assertIntradayQuoteIdempotencyKey(options.idempotencyKey);
}

export async function createIntradayQuoteDemand(
  body: IntradayQuoteDemandRequest,
  options: IntradayQuoteMutationOptions,
): Promise<IntradayQuoteDemandResponse> {
  const requestBody = intradayQuoteDemandRequestSchema.parse(body);
  const { idempotencyKey, signal: _signal, nowMs: _nowMs, ...browserOptions } = options;
  const response = await mutateWithCsrf(mutationPath(INTRADAY_QUOTE_DEMAND_PATH), {
    ...browserOptions,
    idempotencyKey: mutationKey({ ...options, idempotencyKey }),
    json: requestBody,
    method: "POST",
  });
  return parseMutationResponse(response, 200, intradayQuoteDemandResponseSchema, options);
}

export async function releaseIntradayQuoteDemand(
  demandId: string,
  body: IntradayQuoteReleaseRequest,
  options: IntradayQuoteMutationOptions,
): Promise<void> {
  const requestBody = intradayQuoteReleaseRequestSchema.parse(body);
  const { idempotencyKey, signal: _signal, nowMs: _nowMs, ...browserOptions } = options;
  const response = await mutateWithCsrf(mutationPath(intradayQuoteDemandPath(demandId)), {
    ...browserOptions,
    idempotencyKey: mutationKey({ ...options, idempotencyKey }),
    json: requestBody,
    method: "DELETE",
  });
  await parseMutationResponse(response, 204, z.undefined(), options);
}

export async function getIntradayQuote(
  identity: IntradayQuoteIdentity,
  options: IntradayQuoteClientOptions = {},
): Promise<IntradayQuoteResponse> {
  const parsedIdentity = intradayQuoteIdentitySchema.parse(identity);
  const requestInit: RequestInit = {
    cache: "no-store",
    credentials: "same-origin",
    mode: "same-origin",
  };
  if (options.signal !== undefined) requestInit.signal = options.signal;
  const response = await (options.fetcher ?? fetch)(
    requestUrl(intradayQuotePath(parsedIdentity), options.origin),
    requestInit,
  );
  if (!response.ok) await throwIntradayQuoteApiError(response, options);
  if (response.status !== 200) {
    throw new ApiContractError(response.status, "Intraday quote returned an unexpected status");
  }
  return parseIntradayQuoteResponse(await jsonBody(response), {
    nowMs: options.nowMs ?? Date.now(),
  });
}

export type IntradayQuoteClient = {
  readonly createDemand: (
    body: IntradayQuoteDemandRequest,
    options: IntradayQuoteMutationOptions,
  ) => Promise<IntradayQuoteDemandResponse>;
  readonly getQuote: (
    identity: IntradayQuoteIdentity,
    options?: IntradayQuoteClientOptions,
  ) => Promise<IntradayQuoteResponse>;
  readonly releaseDemand: (
    demandId: string,
    body: IntradayQuoteReleaseRequest,
    options: IntradayQuoteMutationOptions,
  ) => Promise<void>;
};

export function createIntradayQuoteClient(
  defaults: Omit<IntradayQuoteClientOptions, "signal" | "nowMs"> = {},
): IntradayQuoteClient {
  return {
    createDemand: (body, options) => createIntradayQuoteDemand(body, { ...defaults, ...options }),
    getQuote: (identity, options = {}) => getIntradayQuote(identity, { ...defaults, ...options }),
    releaseDemand: (demandId, body, options) =>
      releaseIntradayQuoteDemand(demandId, body, { ...defaults, ...options }),
  };
}

export const createOwnerIntradayQuoteDemand = createIntradayQuoteDemand;
export const getOwnerIntradayQuote = getIntradayQuote;
export const releaseOwnerIntradayQuoteDemand = releaseIntradayQuoteDemand;
