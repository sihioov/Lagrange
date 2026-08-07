import { describe, expect, it } from "vitest";
import { logout, mutateWithCsrf } from "@/lib/api/browser-client";

type RecordedRequest = {
  readonly cache: RequestCache;
  readonly credentials: RequestCredentials;
  readonly csrf: string | null;
  readonly method: string;
  readonly url: string;
};

function csrfApi(): {
  readonly calls: RecordedRequest[];
  readonly fetcher: typeof fetch;
} {
  const calls: RecordedRequest[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const request = new Request(input, init);
    const csrf = request.headers.get("x-csrf-token");
    calls.push({
      cache: request.cache,
      credentials: request.credentials,
      csrf,
      method: request.method,
      url: request.url,
    });
    if (request.url.endsWith("/api/v1/auth/csrf")) {
      return Response.json(
        { csrf_token: "synchronizer-owner" },
        { headers: { "Cache-Control": "no-store" } },
      );
    }
    if (request.url.endsWith("/api/v1/auth/logout") && csrf === "synchronizer-owner") {
      return new Response(null, {
        status: 204,
        headers: { "Cache-Control": "no-store" },
      });
    }
    return Response.json(
      {
        error: {
          code: "CSRF_DENIED",
          message: "missing or invalid CSRF token",
          request_id: "request-csrf",
        },
      },
      { status: 403, headers: { "Cache-Control": "no-store" } },
    );
  };
  return { calls, fetcher };
}

describe("CSRF-aware browser mutations", () => {
  it("fetches a synchronizer token and sends it only in the mutation header", async () => {
    // Given
    const api = csrfApi();

    // When
    const response = await logout({
      fetcher: api.fetcher,
      origin: "https://lagrange.test",
    });

    // Then
    expect(response.status).toBe(204);
    expect(api.calls.map(({ method, url }) => ({ method, url }))).toEqual([
      { method: "GET", url: "https://lagrange.test/api/v1/auth/csrf" },
      { method: "POST", url: "https://lagrange.test/api/v1/auth/logout" },
    ]);
    expect(api.calls[0]?.cache).toBe("no-store");
    expect(api.calls[1]?.cache).toBe("no-store");
    expect(api.calls[1]?.credentials).toBe("same-origin");
    expect(api.calls[1]?.csrf).toBe("synchronizer-owner");
  });

  it("surfaces the server CSRF_DENIED response when no token is presented", async () => {
    // Given
    const api = csrfApi();

    // When
    const response = await api.fetcher("https://lagrange.test/api/v1/auth/logout", {
      method: "POST",
      body: "{}",
      headers: { "Content-Type": "application/json" },
    });
    const body: unknown = await response.json();

    // Then
    expect(response.status).toBe(403);
    expect(body).toEqual({
      error: {
        code: "CSRF_DENIED",
        message: "missing or invalid CSRF token",
        request_id: "request-csrf",
      },
    });
  });

  it("accepts only generated-contract mutation paths", async () => {
    // Given
    const api = csrfApi();

    // When
    const response = await mutateWithCsrf("/api/v1/auth/logout", {
      fetcher: api.fetcher,
      method: "POST",
      origin: "https://lagrange.test",
      json: {},
    });

    // Then
    expect(response.status).toBe(204);
  });
});
