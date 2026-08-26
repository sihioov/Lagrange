import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import DashboardPage from "@/app/(authenticated)/page";
import type { ApiSession } from "@/lib/api/contracts";

const mocks = vi.hoisted(() => ({
  getLocale: vi.fn(),
  getServerSession: vi.fn(),
}));

vi.mock("server-only", () => ({}));
vi.mock("@/lib/api/server-session", () => ({ getServerSession: mocks.getServerSession }));
vi.mock("@/lib/i18n/server", () => ({ getLocale: mocks.getLocale }));

const OWNER_ONLY_SESSION = {
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "owner_only",
  owner_beta_paper_mode: "disabled",
  role: "owner",
  user_id: "00000000-0000-4000-8000-000000000001",
} as const satisfies ApiSession;

const OWNER_BETA_MEMBER_SESSION = {
  ...OWNER_ONLY_SESSION,
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

function workspaceHrefs(markup: string): string[] {
  return Array.from(markup.matchAll(/href="([^"]+)"/g), (match) => match[1] ?? "");
}

async function renderDashboard(session: ApiSession): Promise<string> {
  mocks.getLocale.mockResolvedValueOnce("en");
  mocks.getServerSession.mockResolvedValueOnce(session);
  return renderToStaticMarkup(await DashboardPage());
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("dashboard owner-beta workspace policy", () => {
  it("keeps recommendations and backtests while hiding disabled Paper for the Owner", async () => {
    const markup = await renderDashboard(OWNER_ONLY_SESSION);

    expect(workspaceHrefs(markup)).toEqual(["/strategies", "/recommendations", "/backtests"]);
  });

  it("shows Paper when its separate Owner beta activation is enabled", async () => {
    const markup = await renderDashboard({
      ...OWNER_ONLY_SESSION,
      owner_beta_paper_mode: "enabled",
    });

    expect(workspaceHrefs(markup)).toEqual([
      "/strategies",
      "/recommendations",
      "/backtests",
      "/paper",
    ]);
  });

  it("preserves all existing beta cards in normal disabled mode", async () => {
    const markup = await renderDashboard({
      ...OWNER_ONLY_SESSION,
      owner_beta_access_mode: "disabled",
    });

    expect(workspaceHrefs(markup)).toEqual([
      "/strategies",
      "/recommendations",
      "/backtests",
      "/paper",
    ]);
  });

  it("does not expose beta destinations to an Owner-beta Member", async () => {
    const markup = await renderDashboard(OWNER_BETA_MEMBER_SESSION);

    expect(workspaceHrefs(markup)).toEqual(["/strategies"]);
  });
});
