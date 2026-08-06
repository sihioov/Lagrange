import "server-only";

import { cookies } from "next/headers";
import type { ApiSession } from "./contracts";
import { createServerApiClient, SESSION_COOKIE_NAME } from "./server-client";

export class ServerConfigurationError extends Error {
  override readonly name = "ServerConfigurationError";
}

function internalApiOrigin(): string {
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

export async function getServerSession(): Promise<ApiSession> {
  const cookieStore = await cookies();
  const sessionCookie = cookieStore.get(SESSION_COOKIE_NAME);
  const client =
    sessionCookie === undefined
      ? createServerApiClient({ baseUrl: internalApiOrigin(), fetcher: fetch })
      : createServerApiClient({
          baseUrl: internalApiOrigin(),
          fetcher: fetch,
          sessionCookie: sessionCookie.value,
        });
  return client.getSession();
}
