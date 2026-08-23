import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import BacktestsPage from "@/app/(authenticated)/backtests/page";
import PaperPage from "@/app/(authenticated)/paper/page";
import RecommendationsPage from "@/app/(authenticated)/recommendations/page";
import { OwnerBetaProductRoute } from "@/components/pages/owner-beta-product-route";
import type { ApiSession } from "@/lib/api/contracts";

const mocks = vi.hoisted(() => ({
  getProductApi: vi.fn(),
  getServerSession: vi.fn(),
}));

vi.mock("server-only", () => ({}));
vi.mock("@/lib/api/server-products", () => ({ getProductApi: mocks.getProductApi }));
vi.mock("@/lib/api/server-session", () => ({ getServerSession: mocks.getServerSession }));
vi.mock("@/lib/i18n/server", () => ({ getLocale: async () => "en" }));

const MEMBER_OWNER_ONLY = {
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "owner_only",
  role: "member",
  user_id: "00000000-0000-4000-8000-000000000002",
} as const satisfies ApiSession;

const OWNER_OWNER_ONLY = {
  ...MEMBER_OWNER_ONLY,
  role: "owner",
  user_id: "00000000-0000-4000-8000-000000000001",
} as const satisfies ApiSession;

afterEach(() => {
  vi.clearAllMocks();
});

describe("owner-beta product page boundary", () => {
  it("blocks all three direct Member pages before a product client is created", async () => {
    mocks.getServerSession.mockResolvedValue(MEMBER_OWNER_ONLY);
    mocks.getProductApi.mockImplementation(() => {
      throw new Error("product API must not be constructed for a refused Member");
    });

    for (const page of [RecommendationsPage, BacktestsPage, PaperPage]) {
      const markup = renderToStaticMarkup(await page());
      expect(markup).toContain("Owner access required");
      expect(markup).toContain("This area is restricted to the Owner.");
      expect(markup).toContain('href="/"');
      expect(markup).toContain('role="alert"');
      expect(markup).toContain('aria-live="assertive"');
    }

    expect(mocks.getProductApi).not.toHaveBeenCalled();
    expect(mocks.getServerSession).toHaveBeenCalledTimes(3);
  });

  it("renders lazily for the Owner and for the normal disabled mode", async () => {
    const renderProduct = vi.fn(() => <p>product content</p>);

    mocks.getServerSession.mockResolvedValueOnce(OWNER_OWNER_ONLY).mockResolvedValueOnce({
      ...MEMBER_OWNER_ONLY,
      owner_beta_access_mode: "disabled",
    });

    const owner = await OwnerBetaProductRoute({ renderProduct, title: "Protected product" });
    const normal = await OwnerBetaProductRoute({ renderProduct, title: "Normal product" });

    expect(renderToStaticMarkup(owner)).toContain("product content");
    expect(renderToStaticMarkup(normal)).toContain("product content");
    expect(renderProduct).toHaveBeenCalledTimes(2);
  });
});
