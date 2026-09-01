import ky from "ky";
import { z } from "zod";
import type { BrowserResponseOptions } from "./browser-response";
import { parseBrowserApiResponse } from "./browser-response";
import type { ApiPath, ProductMutationPath } from "./contracts";
import { AUTH_API_PATHS, csrfTokenSchema } from "./contracts";

export type MutationMethod = "DELETE" | "PATCH" | "POST" | "PUT";

export type BrowserClientOptions = BrowserResponseOptions & {
  readonly fetcher?: typeof fetch;
  readonly origin?: string;
};

export type MutationOptions = BrowserClientOptions & {
  readonly idempotencyKey?: string;
  readonly json: unknown;
  readonly method: MutationMethod;
};

function requestUrl(path: ApiPath | ProductMutationPath, origin: string | undefined): string {
  return origin === undefined ? path : new URL(path, origin).toString();
}

function browserClient(fetcher: typeof fetch | undefined): typeof ky {
  return fetcher === undefined ? ky : ky.create({ fetch: fetcher });
}

async function csrfToken(options: BrowserClientOptions): Promise<string> {
  const response = await browserClient(options.fetcher).get(
    requestUrl(AUTH_API_PATHS.csrf, options.origin),
    {
      cache: "no-store",
      credentials: "same-origin",
      retry: 0,
      throwHttpErrors: false,
      timeout: 10_000,
    },
  );
  const parserOptions = options.navigate === undefined ? {} : { navigate: options.navigate };
  const parsed = await parseBrowserApiResponse(response, csrfTokenSchema, parserOptions);
  return parsed.csrf_token;
}

export async function mutateWithCsrf(
  path: ApiPath | ProductMutationPath,
  options: MutationOptions,
): Promise<Response> {
  const token = await csrfToken(options);
  return browserClient(options.fetcher)(requestUrl(path, options.origin), {
    cache: "no-store",
    credentials: "same-origin",
    headers: {
      "Idempotency-Key": options.idempotencyKey ?? crypto.randomUUID(),
      "X-CSRF-Token": token,
    },
    json: options.json,
    method: options.method,
    retry: 0,
    throwHttpErrors: false,
    timeout: 10_000,
  });
}

export async function logout(options: BrowserClientOptions = {}): Promise<Response> {
  const response = await mutateWithCsrf(AUTH_API_PATHS.logout, {
    ...options,
    json: {},
    method: "POST",
  });
  if (!response.ok) {
    // Logout has no success body (204), so its caller cannot use the normal
    // generated payload parser. Parse only non-success responses from a
    // clone, preserving the response for callers while still recovering from
    // a session that expired between the CSRF preflight and the mutation.
    await parseBrowserApiResponse(response.clone(), z.unknown(), {
      ...(options.navigate === undefined ? {} : { navigate: options.navigate }),
    });
  }
  return response;
}
