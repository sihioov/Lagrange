import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import RecommendationsPage from "@/app/(authenticated)/recommendations/page";
import StrategiesPage from "@/app/(authenticated)/strategies/page";

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: () => ({ name: "__Host-lagrange_session", value: "member-opaque" }),
  }),
}));

const STRATEGY_CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const RECOMMENDATION_RUN_ID = "00000000-0000-4000-8000-000000000201";

function syntheticRecommendationApi(): typeof fetch {
  return async (input, init) => {
    const request = new Request(input, init);
    const { pathname } = new URL(request.url);
    if (pathname === "/api/v1/strategies") {
      return Response.json({
        has_more: false,
        items: [
          {
            id: "dual_momentum",
            display_name: "Dual momentum",
            description: "Ranks the governed universe with absolute and relative momentum.",
            risk_description: "May hold cash when the absolute momentum gate fails.",
            state: "Validated",
            latest_version: "2.3.1",
            parameter_schema: {
              properties: {
                lookback_months: {
                  default: 12,
                  maximum: 24,
                  minimum: 1,
                  title: "Lookback months",
                  type: "integer",
                },
                top_n: {
                  default: 4,
                  maximum: 11,
                  minimum: 1,
                  title: "Top holdings",
                  type: "integer",
                },
              },
              required: ["lookback_months", "top_n"],
              type: "object",
            },
          },
        ],
        next_cursor: null,
      });
    }
    if (pathname === "/api/v1/licensing-status") {
      return Response.json({
        as_of: "2026-01-31",
        datasets: [
          {
            covered: true,
            dataset_id: "krx-eod",
            effective_from: "2026-01-01",
            effective_until: "2026-12-31",
            state: "ACTIVE",
            use_kind: "recommendation",
          },
        ],
      });
    }
    if (pathname === "/api/v1/recommendations/latest") {
      return Response.json({ run: recommendationRun(true) });
    }
    if (pathname === "/api/v1/recommendations/runs") {
      return Response.json({
        has_more: false,
        items: [recommendationRun(false)],
        next_cursor: null,
      });
    }
    return Response.json(
      {
        error: {
          code: "RESOURCE_NOT_FOUND",
          message: `No synthetic response for ${pathname}`,
          request_id: "request-component-recommendation",
        },
      },
      { status: 404 },
    );
  };
}

function recommendationRun(includeItems: boolean) {
  return {
    as_of: "2026-01-31",
    created_at: "2026-01-31T06:30:00Z",
    id: RECOMMENDATION_RUN_ID,
    items: includeItems
      ? [
          {
            excluded: false,
            factors: { momentum_12m: "0.184200", normalized_score: "0.912300" },
            instrument_id: "069500.KRX",
            rank: 1,
            reason_codes: ["ABSOLUTE_MOMENTUM_PASS", "RELATIVE_RANK_1"],
            target_weight: "0.400000",
          },
          {
            excluded: true,
            exclusion_reason: "Inverse products are outside the governed universe.",
            factors: { normalized_score: "-0.112000" },
            instrument_id: "114800.KRX",
            rank: null,
            reason_codes: ["UNIVERSE_POLICY_EXCLUSION"],
            target_weight: null,
          },
        ]
      : undefined,
    status: "SUCCEEDED",
    strategy_config_id: STRATEGY_CONFIG_ID,
    summary: {
      data_version: "krx-eod@2026-01-31",
      engine_version: "selector@1.4.0",
      strategy_version: "dual_momentum@2.3.1",
      warnings: ["Result is two sessions old; review the as-of date before acting."],
    },
  };
}

beforeEach(() => {
  vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
  vi.stubGlobal("fetch", syntheticRecommendationApi());
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("strategy and recommendation product surfaces", () => {
  it("renders a schema-bound strategy configuration form", async () => {
    // Given
    const page = await StrategiesPage();

    // When
    const markup = renderToStaticMarkup(page);

    // Then
    expect(markup).toContain("Dual momentum");
    expect(markup).toContain("Validated");
    expect(markup).toContain("Version 2.3.1");
    expect(markup).toContain("Lookback months");
    expect(markup).toContain('min="1"');
    expect(markup).toContain('max="24"');
    expect(markup).toContain("Save strategy configuration");
    expect(markup).not.toContain("Upload strategy code");
  });

  it("renders explainable recommendations with provenance and warnings", async () => {
    // Given
    const page = await RecommendationsPage();

    // When
    const markup = renderToStaticMarkup(page);

    // Then
    expect(markup).toContain("Strategy-based proposal");
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("40.00%");
    expect(markup).toContain("ABSOLUTE_MOMENTUM_PASS");
    expect(markup).toContain("114800.KRX");
    expect(markup).toContain("Inverse products are outside the governed universe.");
    expect(markup).toContain("dual_momentum@2.3.1");
    expect(markup).toContain("krx-eod@2026-01-31");
    expect(markup).toContain("selector@1.4.0");
    expect(markup).toContain("Result is two sessions old");
    expect(markup).toContain("ACTIVE");
    expect(markup).not.toContain("guaranteed return");
  });
});
