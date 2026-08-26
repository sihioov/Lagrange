import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ApiSession } from "@/lib/api/contracts";

const mocks = vi.hoisted(() => ({
  cookies: vi.fn(async () => new Map()),
}));

vi.mock("server-only", () => ({}));
vi.mock("next/headers", () => ({ cookies: mocks.cookies }));

import { getServerSession } from "@/lib/api/server-session";

const SESSION = {
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "disabled",
  owner_beta_paper_mode: "disabled",
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

describe("getServerSession request memoization", () => {
  it("preserves the existing request-local client delegation", async () => {
    const fetcher: typeof fetch = vi.fn(async () => Response.json(SESSION));
    vi.stubGlobal("fetch", fetcher);

    await expect(getServerSession()).resolves.toEqual(SESSION);

    expect(mocks.cookies).toHaveBeenCalledOnce();
    expect(fetcher).toHaveBeenCalledOnce();
  });

  it("does not select a persistent or shared cache mechanism", () => {
    const source = readFileSync(resolve(process.cwd(), "lib/api/server-session.ts"), "utf8");

    // Vitest has no React Server Component request dispatcher, so this test
    // verifies the supported React.cache wiring rather than fabricating a
    // cross-request lifecycle that only Next.js supplies at render time.
    expect(source).toContain('import { cache } from "react"');
    expect(source).toContain("export const getServerSession = cache(async ()");
    expect(source).not.toMatch(
      /unstable_cache|["']use cache|cacheLife|cacheTag|new Map\(|globalThis\./,
    );
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});
