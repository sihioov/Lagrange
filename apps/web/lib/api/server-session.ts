import "server-only";

import { cookies } from "next/headers";
import type { ApiSession } from "./contracts";
import {
  createServerApiClient,
  SESSION_COOKIE_NAME,
  type ServerApiClientOptions,
} from "./server-client";

export class ServerConfigurationError extends Error {
  override readonly name = "ServerConfigurationError";
}

export function internalApiOrigin(): string {
  const { API_INTERNAL_URL: configuredApiUrl } = process.env;
  const configured = configuredApiUrl ?? "http://127.0.0.1:8080";
  if (!URL.canParse(configured)) {
    throw new ServerConfigurationError("API_INTERNAL_URL must be an absolute URL");
  }
  const url = new URL(configured);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new ServerConfigurationError("API_INTERNAL_URL must use HTTP or HTTPS");
  }
  return url.origin;
}

export async function serverApiClientOptions(): Promise<ServerApiClientOptions> {
  const cookieStore = await cookies();
  const sessionCookie = cookieStore.get(SESSION_COOKIE_NAME);
  return sessionCookie === undefined
    ? { baseUrl: internalApiOrigin(), fetcher: fetch }
    : {
        baseUrl: internalApiOrigin(),
        fetcher: fetch,
        sessionCookie: sessionCookie.value,
      };
}

export async function getServerSession(): Promise<ApiSession> {
  const client = createServerApiClient(await serverApiClientOptions());
  return client.getSession();
}
