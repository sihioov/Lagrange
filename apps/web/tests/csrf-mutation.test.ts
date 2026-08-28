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
    expect(api.calls[0]?.credentials).toBe("same-origin");
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

  it.each(["SESSION_UNKNOWN", "SESSION_EXPIRED"] as const)(
    "navigates to login when the CSRF session is %s",
    async (code) => {
      const calls: string[] = [];
      const navigations: string[] = [];
      const fetcher: typeof fetch = async (input, init) => {
        const request = new Request(input, init);
        calls.push(`${request.method} ${request.url}`);
        return Response.json(
          {
            error: {
              code,
              message: "session failure",
              request_id: "request-csrf-session",
            },
          },
          { status: 401 },
        );
      };

      await expect(
        logout({
          fetcher,
          navigate: (href) => navigations.push(href),
          origin: "https://lagrange.test",
        }),
      ).rejects.toMatchObject({ code });

      expect(navigations).toEqual(["/login"]);
      expect(calls).toEqual(["GET https://lagrange.test/api/v1/auth/csrf"]);
    },
  );

  it("does not navigate for an internal CSRF failure", async () => {
    const navigations: string[] = [];
    const fetcher: typeof fetch = async () =>
      Response.json(
        {
          error: {
            code: "INTERNAL",
            message: "session store unavailable",
            request_id: "request-csrf-internal",
          },
        },
        { status: 500 },
      );

    await expect(
      logout({
        fetcher,
        navigate: (href) => navigations.push(href),
        origin: "https://lagrange.test",
      }),
    ).rejects.toMatchObject({ code: "INTERNAL" });
    expect(navigations).toEqual([]);
  });

  it("navigates when the session expires after the CSRF preflight", async () => {
    const navigations: string[] = [];
    const calls: string[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      calls.push(`${request.method} ${request.url}`);
      if (request.url.endsWith("/api/v1/auth/csrf")) {
        return Response.json({ csrf_token: "synchronizer-owner" });
      }
      return Response.json(
        {
          error: {
            code: "SESSION_EXPIRED",
            message: "session expired",
            request_id: "request-logout-session-expired",
          },
        },
        { status: 401 },
      );
    };

    await expect(
      logout({
        fetcher,
        navigate: (href) => navigations.push(href),
        origin: "https://lagrange.test",
      }),
    ).rejects.toMatchObject({ code: "SESSION_EXPIRED" });
    expect(navigations).toEqual(["/login"]);
    expect(calls).toEqual([
      "GET https://lagrange.test/api/v1/auth/csrf",
      "POST https://lagrange.test/api/v1/auth/logout",
    ]);
  });
});
