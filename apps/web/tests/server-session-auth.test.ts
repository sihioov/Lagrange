import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  cookies: vi.fn(async () => ({ get: () => undefined })),
  redirect: vi.fn((destination: string) => {
    throw Object.assign(new Error("NEXT_REDIRECT"), { destination });
  }),
}));

vi.mock("server-only", () => ({}));
vi.mock("next/headers", () => ({ cookies: mocks.cookies }));
vi.mock("next/navigation", () => ({ redirect: mocks.redirect }));

import { getServerSession } from "@/lib/api/server-session";

describe("getServerSession authentication boundary", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
  });

  it.each(["SESSION_UNKNOWN", "SESSION_EXPIRED"] as const)(
    "redirects %s to the canonical login route",
    async (code) => {
      vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
      vi.stubGlobal(
        "fetch",
        vi.fn(async () =>
          Response.json(
            {
              error: {
                code,
                message: "static test message",
                request_id: "request-test",
              },
            },
            { status: 401 },
          ),
        ),
      );

      await expect(getServerSession()).rejects.toMatchObject({
        destination: "/login",
        message: "NEXT_REDIRECT",
      });
      expect(mocks.redirect).toHaveBeenCalledOnce();
      expect(mocks.redirect).toHaveBeenCalledWith("/login");
    },
  );

  it("preserves an internal session-store failure instead of misreporting login expiry", async () => {
    vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json(
          {
            error: {
              code: "INTERNAL",
              message: "session store unavailable",
              request_id: "request-test-internal",
            },
          },
          { status: 500 },
        ),
      ),
    );

    await expect(getServerSession()).rejects.toMatchObject({
      code: "INTERNAL",
      name: "ApiProblem",
      status: 500,
    });
    expect(mocks.redirect).not.toHaveBeenCalled();
  });
});
