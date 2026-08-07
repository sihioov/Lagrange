import type { KyInstance } from "ky";
import ky from "ky";
import type { ApiSession } from "./contracts";
import { AUTH_API_PATHS, apiSessionSchema } from "./contracts";
import { parseApiResponse } from "./response";

export const SESSION_COOKIE_NAME = "__Host-lagrange_session";

export type ServerApiClientOptions = {
  readonly baseUrl: string;
  readonly fetcher: typeof fetch;
  readonly sessionCookie?: string;
};

export type ServerApiClient = {
  readonly getSession: () => Promise<ApiSession>;
};

function normalizedBaseUrl(baseUrl: string): string {
  return baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
}

export function createServerTransport(options: ServerApiClientOptions): KyInstance {
  const headers = new Headers();
  if (options.sessionCookie !== undefined) {
    headers.set("Cookie", `${SESSION_COOKIE_NAME}=${options.sessionCookie}`);
  }
  return ky.create({
    cache: "no-store",
    credentials: "omit",
    fetch: options.fetcher,
    headers,
    prefix: normalizedBaseUrl(options.baseUrl),
    retry: 0,
    throwHttpErrors: false,
    timeout: 10_000,
  });
}

export function createServerApiClient(options: ServerApiClientOptions): ServerApiClient {
  const client = createServerTransport(options);

  return {
    getSession: async () => {
      const response = await client.get(AUTH_API_PATHS.session.slice(1));
      return parseApiResponse(response, apiSessionSchema);
    },
  };
}
