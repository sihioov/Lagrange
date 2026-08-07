import { describe, expect, it } from "vitest";
import { createServerApiClient, SESSION_COOKIE_NAME } from "@/lib/api/server-client";

const OWNER_USER_ID = "00000000-0000-4000-8000-000000000001";
const MEMBER_USER_ID = "00000000-0000-4000-8000-000000000002";

type RecordedRequest = {
  readonly authorization: string | null;
  readonly cache: RequestCache;
  readonly cookie: string | null;
  readonly url: string;
};

function sessionApi(): {
  readonly calls: RecordedRequest[];
  readonly fetcher: typeof fetch;
} {
  const calls: RecordedRequest[] = [];
  const sessions = new Map([
    ["owner-opaque", { user_id: OWNER_USER_ID, role: "owner", expires_at_secs: 2_000_000_000 }],
    ["member-opaque", { user_id: MEMBER_USER_ID, role: "member", expires_at_secs: 2_000_000_000 }],
  ]);
  const fetcher: typeof fetch = async (input, init) => {
    const request = new Request(input, init);
    const cookie = request.headers.get("cookie");
    calls.push({
      authorization: request.headers.get("authorization"),
      cache: request.cache,
      cookie,
      url: request.url,
    });
    const opaque = cookie?.split("=").at(1);
    const session = opaque === undefined ? undefined : sessions.get(opaque);
    if (session === undefined) {
      return Response.json(
        {
          error: {
            code: "SESSION_UNKNOWN",
            message: "session required",
            request_id: "request-test",
          },
        },
        { status: 401, headers: { "Cache-Control": "no-store" } },
      );
    }
    return Response.json(session, {
      headers: { "Cache-Control": "no-store" },
    });
  };
  return { calls, fetcher };
}

describe("server API client", () => {
  it("returns isolated payloads when two opaque sessions use one process", async () => {
    // Given
    const api = sessionApi();
    const ownerClient = createServerApiClient({
      baseUrl: "https://api.internal",
      fetcher: api.fetcher,
      sessionCookie: "owner-opaque",
    });
    const memberClient = createServerApiClient({
      baseUrl: "https://api.internal",
      fetcher: api.fetcher,
      sessionCookie: "member-opaque",
    });

    // When
    const ownerSession = await ownerClient.getSession();
    const memberSession = await memberClient.getSession();

    // Then
    expect(ownerSession).toMatchObject({ user_id: OWNER_USER_ID, role: "owner" });
    expect(memberSession).toMatchObject({ user_id: MEMBER_USER_ID, role: "member" });
    expect(JSON.stringify(ownerSession)).not.toContain(MEMBER_USER_ID);
    expect(JSON.stringify(memberSession)).not.toContain(OWNER_USER_ID);
    expect(api.calls).toHaveLength(2);
    expect(api.calls[0]?.cookie).toBe(`${SESSION_COOKIE_NAME}=owner-opaque`);
    expect(api.calls[1]?.cookie).toBe(`${SESSION_COOKIE_NAME}=member-opaque`);
  });

  it("opts every authenticated request out of caches and forwards no bearer identity", async () => {
    // Given
    const api = sessionApi();
    const client = createServerApiClient({
      baseUrl: "https://api.internal",
      fetcher: api.fetcher,
      sessionCookie: "owner-opaque",
    });

    // When
    await client.getSession();

    // Then
    expect(api.calls[0]?.cache).toBe("no-store");
    expect(api.calls[0]?.authorization).toBeNull();
    expect(api.calls[0]?.url).toBe("https://api.internal/api/v1/auth/session");
    expect(api.calls[0]?.url).not.toContain(OWNER_USER_ID);
  });
});
