import ky from "ky";
import type { ApiPath } from "./contracts";
import { AUTH_API_PATHS, csrfTokenSchema } from "./contracts";
import { parseApiResponse } from "./response";

export type MutationMethod = "DELETE" | "PATCH" | "POST" | "PUT";

export type BrowserClientOptions = {
  readonly fetcher?: typeof fetch;
  readonly origin?: string;
};

export type MutationOptions = BrowserClientOptions & {
  readonly json: unknown;
  readonly method: MutationMethod;
};

function requestUrl(path: ApiPath, origin: string | undefined): string {
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
  const parsed = await parseApiResponse(response, csrfTokenSchema);
  return parsed.csrf_token;
}

export async function mutateWithCsrf(path: ApiPath, options: MutationOptions): Promise<Response> {
  const token = await csrfToken(options);
  return browserClient(options.fetcher)(requestUrl(path, options.origin), {
    cache: "no-store",
    credentials: "same-origin",
    headers: { "X-CSRF-Token": token },
    json: options.json,
    method: options.method,
    retry: 0,
    throwHttpErrors: false,
    timeout: 10_000,
  });
}

export function logout(options: BrowserClientOptions = {}): Promise<Response> {
  return mutateWithCsrf(AUTH_API_PATHS.logout, {
    ...options,
    json: {},
    method: "POST",
  });
}
