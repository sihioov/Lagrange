import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import RecommendationsPage from "@/app/(authenticated)/recommendations/page";
import { recommendationPollDelay } from "@/components/recommendations/recommendation-run-status";

vi.mock("server-only", () => ({}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: () => ({ name: "__Host-lagrange_session", value: "member-opaque" }),
  }),
}));

const CONFIG_A = "00000000-0000-4000-8000-000000000101";
const CONFIG_B = "00000000-0000-4000-8000-000000000102";
const SUCCESS_RUN = "00000000-0000-4000-8000-000000000201";
const PENDING_RUN = "00000000-0000-4000-8000-000000000202";

type RunFixture = ReturnType<typeof run>;
type MetadataOnlyRunFixture = Omit<RunFixture, "items"> & { readonly items?: never };

type Fixture = {
  readonly blocked?: boolean;
  readonly configs?: readonly ReturnType<typeof config>[];
  readonly history?: readonly RunFixture[];
  readonly latest?: {
    readonly latest_run: RunFixture | MetadataOnlyRunFixture;
    readonly run: RunFixture | null;
  };
  readonly selected?: RunFixture;
};

function config(id: string, updatedAt = "2026-01-31T06:00:00Z", isActive = true) {
  return {
    config: { lookback_days: 126, top_n: 4 },
    created_at: updatedAt,
    id,
    is_active: isActive,
    strategy_id: "relative_momentum",
    strategy_version: "1.0.0",
    updated_at: updatedAt,
  };
}

function run(
  id = SUCCESS_RUN,
  overrides: Partial<{
    as_of: string;
    cash_weight: string;
    created_at: string;
    id: string;
    items: readonly Record<string, unknown>[];
    status: "PENDING" | "SUCCEEDED" | "FAILED" | "BLOCKED";
    strategy_config_id: string;
  }> = {},
) {
  const { cash_weight = "0.200000", ...rest } = overrides;
  return {
    as_of: "2026-01-31",
    created_at: "2026-01-31T06:30:00Z",
    id,
    items: [
      {
        excluded: false,
        factors: { normalized_score: "0.912300", return_6m: "0.184200" },
        instrument_id: "069500.KRX",
        rank: 1,
        reason_codes: ["RELATIVE_RANK_1"],
        target_weight: "0.800000",
      },
    ],
    job_id: "00000000-0000-4000-8000-000000000301",
    provenance: {
      dataset_manifest_sha256: "a".repeat(64),
      dataset_version_id: "00000000-0000-4000-8000-000000000401",
    },
    status: "SUCCEEDED" as const,
    strategy_config_id: CONFIG_A,
    summary: {
      cash_weight,
      dataset_id: "kr-etf-daily",
      dataset_version: "phase0-v2",
      factor_snapshot_hash: `sha256:${"b".repeat(64)}`,
      manifest_sha256: "a".repeat(64),
      origin: "synthetic",
      portfolio_snapshot_id: `sha256:${"c".repeat(64)}`,
      universe_snapshot_id: `sha256:${"d".repeat(64)}`,
      warnings: [],
    },
    trigger_kind: "MANUAL" as const,
    ...rest,
  };
}

function syntheticRecommendationApi(fixture: Fixture): typeof fetch {
  return async (input, init) => {
    const request = new Request(input, init);
    const { pathname } = new URL(request.url);
    if (pathname === "/api/v1/auth/session") {
      return Response.json({
        expires_at_secs: 2_000_000_000,
        role: "member",
        user_id: "00000000-0000-4000-8000-000000000002",
      });
    }
    if (pathname === "/api/v1/licensing-status") {
      return Response.json({
        as_of: "2026-01-31",
        datasets: [
          {
            covered: !fixture.blocked,
            dataset_id: "kr-etf-daily",
            effective_from: "2026-01-01",
            effective_until: "2026-12-31",
            state: fixture.blocked ? "REVOKED" : "ACTIVE",
            use_kind: "recommendation",
          },
        ],
      });
    }
    if (pathname === "/api/v1/strategy-configs") {
      return Response.json({ has_more: false, items: fixture.configs ?? [], next_cursor: null });
    }
    if (pathname === "/api/v1/recommendations/latest") {
      if (fixture.latest === undefined) {
        return Response.json(
          {
            error: {
              code: "RESOURCE_NOT_FOUND",
              message: "no recommendation run yet",
              request_id: "test",
            },
          },
          { status: 404 },
        );
      }
      return Response.json(fixture.latest);
    }
    if (pathname === "/api/v1/recommendations/runs") {
      return Response.json({ has_more: false, items: fixture.history ?? [], next_cursor: null });
    }
    if (
      pathname === `/api/v1/recommendations/runs/${SUCCESS_RUN}` &&
      fixture.selected !== undefined
    ) {
      return Response.json(fixture.selected);
    }
    return Response.json(
      {
        error: {
          code: "RESOURCE_NOT_FOUND",
          message: `No response for ${pathname}`,
          request_id: "test",
        },
      },
      { status: 404 },
    );
  };
}

beforeEach(() => {
  vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("recommendation workflow", () => {
  it("explains that no governed configuration is available", async () => {
    vi.stubGlobal("fetch", syntheticRecommendationApi({ configs: [], history: [] }));

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("No strategy configuration is available");
    expect(markup).not.toContain('aria-label="Generate recommendation"');
  });

  it("enables a run form for each selectable strategy configuration before any run exists", async () => {
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A), config(CONFIG_B, "2026-01-31T06:01:00Z")],
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain('aria-label="Generate recommendation"');
    expect(markup).toContain("Strategy configuration");
    expect(markup).toContain(`value="${CONFIG_A}"`);
    expect(markup).toContain(`value="${CONFIG_B}"`);
    expect(markup).not.toContain("disabled");
  });

  it("withholds the run form when every governed configuration is inactive", async () => {
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({ configs: [config(CONFIG_A, "2026-01-31T06:00:00Z", false)] }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("No strategy configuration is available");
    expect(markup).not.toContain('aria-label="Generate recommendation"');
  });

  it("offers only active governed configurations", async () => {
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A), config(CONFIG_B, "2026-01-31T06:01:00Z", false)],
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain(`value="${CONFIG_A}"`);
    expect(markup).not.toContain(`value="${CONFIG_B}"`);
  });

  it("continues polling at the capped delay after the initial backoff budget", () => {
    expect(recommendationPollDelay(0)).toBe(250);
    expect(recommendationPollDelay(4)).toBe(4_000);
    expect(recommendationPollDelay(5)).toBe(4_000);
    expect(recommendationPollDelay(20)).toBe(4_000);
  });

  it("renders item-bearing run data when the latest successful run is metadata-only", async () => {
    const { items: _items, ...metadataOnlyLatest } = run(SUCCESS_RUN);
    const itemBearingRun = run(SUCCESS_RUN);
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A)],
        history: [itemBearingRun],
        latest: { latest_run: metadataOnlyLatest, run: itemBearingRun },
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("069500.KRX");
  });

  it("keeps the last successful report visible, exposes cash and provenance, and links newest history first", async () => {
    const succeeded = run(SUCCESS_RUN);
    const pending = run(PENDING_RUN, {
      created_at: "2026-01-31T07:30:00Z",
      items: [],
      status: "PENDING",
    });
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A)],
        history: [pending, succeeded],
        latest: { latest_run: pending, run: succeeded },
        selected: succeeded,
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Recommendation is in progress");
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("Cash allocation: 20.00%");
    expect(markup).toContain("Synthetic QA data");
    expect(markup).toContain("Dataset version");
    expect(markup).toContain("Universe snapshot");
    expect(markup).toContain("Factor snapshot");
    expect(markup).toContain("Portfolio snapshot");
    expect(markup).toContain(`href="/recommendations?run_id=${SUCCESS_RUN}"`);
    expect(markup.indexOf(PENDING_RUN)).toBeLessThan(markup.indexOf(SUCCESS_RUN));
  });

  it("keeps the last successful report visible when a newer run fails without rendering failed payload", async () => {
    const succeeded = run(SUCCESS_RUN);
    const failed = run(PENDING_RUN, {
      created_at: "2026-01-31T07:30:00Z",
      items: [
        {
          excluded: false,
          instrument_id: "FAILED_PAYLOAD_MUST_NOT_RENDER",
          target_weight: "1.000000",
        },
      ],
      status: "FAILED",
    });
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A)],
        history: [failed, succeeded],
        latest: { latest_run: failed, run: succeeded },
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Recommendation failed");
    expect(markup).toContain("069500.KRX");
    expect(markup).not.toContain("FAILED_PAYLOAD_MUST_NOT_RENDER");
  });

  it("labels an all-cash proposal without inventing selected instruments", async () => {
    const allCash = run(SUCCESS_RUN, { cash_weight: "1.000000", items: [] });
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        configs: [config(CONFIG_A)],
        history: [allCash],
        latest: { latest_run: allCash, run: allCash },
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("All-cash allocation");
    expect(markup).not.toContain("Selected instruments and target weights</caption><thead><tr><th");
  });

  it("fails closed when recommendation access is blocked", async () => {
    vi.stubGlobal(
      "fetch",
      syntheticRecommendationApi({
        blocked: true,
        configs: [config(CONFIG_A)],
        latest: { latest_run: run(), run: run() },
      }),
    );

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Recommendation data is blocked");
    expect(markup).not.toContain("069500.KRX");
    expect(markup).not.toContain("Synthetic QA data");
    expect(markup).not.toContain('aria-label="Generate recommendation"');
  });
});
