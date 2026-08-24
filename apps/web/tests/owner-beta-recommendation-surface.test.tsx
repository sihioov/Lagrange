import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import RecommendationsPage from "@/app/(authenticated)/recommendations/page";
import { submitOwnerBetaRun } from "@/components/recommendations/owner-beta-run-form";
import { pollOwnerBetaRun } from "@/components/recommendations/owner-beta-run-status";
import { apiErrorEnvelopeSchema } from "@/lib/api/contracts";
import {
  OWNER_BETA_PRICE_ONLY_RUNS_PATH,
  ownerBetaRunSchema,
} from "@/lib/products/owner-beta-contracts";

vi.mock("server-only", () => ({}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: (name: string) =>
      name === "__Host-lagrange_session" ? { name, value: "owner-opaque" } : undefined,
  }),
}));

const CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const RUN_ID = "00000000-0000-4000-8000-000000000201";
const JOB_ID = "00000000-0000-4000-8000-000000000301";
const SHA = `sha256:${"a".repeat(64)}`;
const ETF_IDS = [
  "069500.KRX",
  "102110.KRX",
  "114260.KRX",
  "132030.KRX",
  "138230.KRX",
  "152100.KRX",
  "148020.KRX",
  "305720.KRX",
  "278530.KRX",
  "292150.KRX",
  "360750.KRX",
] as const;

function ownerRun(overrides: Record<string, unknown> = {}) {
  return {
    id: RUN_ID,
    job_id: JOB_ID,
    strategy_config_id: CONFIG_ID,
    strategy_id: "buy_and_hold",
    strategy_version: "1.0.0",
    as_of: "2026-08-19",
    status: "SUCCEEDED",
    input_kind: "owner_beta_historical_price_only_v1",
    capability: "PRICE_RETURN_ONLY",
    audience: "OWNER_ONLY",
    vendor_snapshot: true,
    strict_pit: false,
    strategy_config_sha256: SHA,
    candidate_content_sha256: SHA,
    artifact_manifest_sha256: SHA,
    stage5_manifest_sha256: SHA,
    action_manifest_sha256: SHA,
    approval_registry_sha256: SHA,
    factor_snapshot_sha256: SHA,
    target_snapshot_sha256: SHA,
    cash_weight: "0.200000",
    created_at: "2026-08-19T06:30:00Z",
    started_at: "2026-08-19T06:30:01Z",
    finished_at: "2026-08-19T06:30:02Z",
    updated_at: "2026-08-19T06:30:02Z",
    items: ETF_IDS.map((instrument_id, index) => ({
      instrument_id,
      rank: index === 0 ? 1 : null,
      target_weight: index === 0 ? "0.800000" : null,
      excluded: index !== 0,
      exclusion_reason: index === 0 ? undefined : "NOT_SELECTED_BY_STRATEGY",
      reason_codes: index === 0 ? ["SELECTED_TOP_N"] : ["NOT_SELECTED_BY_STRATEGY"],
      factors: { momentum: index === 0 ? "0.912300" : "0.000000" },
    })),
    ...overrides,
  };
}

function ownerApi(entitlementState: "ACTIVE" | "REVOKED" = "ACTIVE"): {
  readonly paths: string[];
  readonly fetcher: typeof fetch;
} {
  const paths: string[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const request = new Request(input, init);
    const pathname = new URL(request.url).pathname;
    paths.push(pathname);
    if (pathname === "/api/v1/auth/session") {
      return Response.json({
        expires_at_secs: 2_000_000_000,
        owner_beta_access_mode: "owner_only",
        owner_beta_paper_mode: "disabled",
        role: "owner",
        user_id: "00000000-0000-4000-8000-000000000001",
      });
    }
    if (pathname === "/api/v1/licensing-status") {
      return Response.json({
        as_of: "2026-08-19",
        datasets: [
          {
            covered: true,
            dataset_id: "owner-beta-price-only",
            state: entitlementState,
            use_kind: "recommendation",
          },
        ],
      });
    }
    if (pathname === "/api/v1/strategy-configs") {
      return Response.json({
        has_more: false,
        items: [
          {
            config: {},
            created_at: "2026-08-19T06:00:00Z",
            id: CONFIG_ID,
            is_active: true,
            strategy_id: "buy_and_hold",
            strategy_version: "1.0.0",
            updated_at: "2026-08-19T06:00:00Z",
          },
        ],
        next_cursor: null,
      });
    }
    if (pathname === OWNER_BETA_PRICE_ONLY_RUNS_PATH) {
      const { items: _items, ...listItem } = ownerRun();
      return Response.json({
        has_more: false,
        items: [listItem],
        next_cursor: null,
      });
    }
    if (pathname === `${OWNER_BETA_PRICE_ONLY_RUNS_PATH}/${RUN_ID}`) {
      return Response.json(ownerRun());
    }
    throw new Error(`unexpected request path: ${pathname}`);
  };
  return { fetcher, paths };
}

function normalApi(): { readonly paths: string[]; readonly fetcher: typeof fetch } {
  const paths: string[] = [];
  const run = {
    as_of: "2026-08-19",
    created_at: "2026-08-19T06:30:00Z",
    id: RUN_ID,
    items: [
      {
        excluded: false,
        factors: { momentum: "0.912300" },
        instrument_id: "069500.KRX",
        rank: 1,
        reason_codes: ["SELECTED_TOP_N"],
        target_weight: "0.800000",
      },
    ],
    job_id: JOB_ID,
    provenance: {
      dataset_manifest_sha256: "b".repeat(64),
      dataset_version_id: CONFIG_ID,
    },
    status: "SUCCEEDED",
    strategy_config_id: CONFIG_ID,
    summary: {},
    trigger_kind: "MANUAL",
  };
  const fetcher: typeof fetch = async (input, init) => {
    const request = new Request(input, init);
    const pathname = new URL(request.url).pathname;
    paths.push(pathname);
    if (pathname === "/api/v1/auth/session") {
      return Response.json({
        expires_at_secs: 2_000_000_000,
        owner_beta_access_mode: "disabled",
        owner_beta_paper_mode: "disabled",
        role: "owner",
        user_id: "00000000-0000-4000-8000-000000000001",
      });
    }
    if (pathname === "/api/v1/licensing-status") {
      return Response.json({
        as_of: "2026-08-19",
        datasets: [
          {
            covered: true,
            dataset_id: "kr-etf-daily",
            state: "ACTIVE",
            use_kind: "recommendation",
          },
        ],
      });
    }
    if (pathname === "/api/v1/strategy-configs") {
      return Response.json({
        has_more: false,
        items: [
          {
            config: {},
            created_at: "2026-08-19T06:00:00Z",
            id: CONFIG_ID,
            is_active: true,
            strategy_id: "buy_and_hold",
            strategy_version: "1.0.0",
            updated_at: "2026-08-19T06:00:00Z",
          },
        ],
        next_cursor: null,
      });
    }
    if (pathname === "/api/v1/recommendations/latest") {
      return Response.json({ latest_run: run, run });
    }
    if (pathname === "/api/v1/recommendations/runs") {
      return Response.json({ has_more: false, items: [run], next_cursor: null });
    }
    throw new Error(`unexpected normal-mode request path: ${pathname}`);
  };
  return { fetcher, paths };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe("owner-beta recommendation surface", () => {
  it("uses only dedicated owner-beta list/detail paths and renders the fixed contract", async () => {
    const api = ownerApi();
    vi.stubGlobal("fetch", api.fetcher);
    vi.stubEnv("API_INTERNAL_URL", "https://api.internal");

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Owner-only");
    expect(markup).toContain("Price-return only");
    expect(markup).toContain("Vendor snapshot");
    expect(markup).toContain("Non-strict PIT");
    expect(markup).toContain("069500.KRX");
    expect(markup).toContain("0.912300");
    expect(markup).toContain("SELECTED_TOP_N");
    expect(api.paths).toContain(OWNER_BETA_PRICE_ONLY_RUNS_PATH);
    expect(api.paths).toContain(`${OWNER_BETA_PRICE_ONLY_RUNS_PATH}/${RUN_ID}`);
    expect(api.paths).not.toContain("/api/v1/recommendations/latest");
    expect(api.paths).not.toContain("/api/v1/recommendations/runs");
  });

  it("keeps the strict schema closed to provider or path fields", () => {
    const parsed = ownerBetaRunSchema.safeParse({
      ...ownerRun(),
      provider: "kis",
      path: "/var/lib/provider-output",
    });

    expect(parsed.success).toBe(false);
  });

  it("rejects semantically invalid nested owner-beta result items", () => {
    const valid = ownerRun();
    const items = valid.items;
    const invalidItems = [
      items.map((item, index) => (index === 0 ? { ...item, instrument_id: "000000.KRX" } : item)),
      items.map((item, index) => (index === 0 ? { ...item, rank: 12 } : item)),
      items.map((item, index) => (index === 0 ? { ...item, target_weight: "1.000001" } : item)),
      items.map((item, index) =>
        index === 0 ? { ...item, reason_codes: ["SELECTED_TOP_N", "SELECTED_TOP_N"] } : item,
      ),
      items.map((item, index) =>
        index === 0 ? { ...item, excluded: true, exclusion_reason: null } : item,
      ),
      items.map((item, index) =>
        index === 0
          ? {
              ...item,
              factors: Object.fromEntries(
                Array.from({ length: 65 }, (_, factorIndex) => [
                  `factor_${factorIndex}`,
                  "0.100000",
                ]),
              ),
            }
          : item,
      ),
    ];

    expect(ownerBetaRunSchema.safeParse(valid).success).toBe(true);
    for (const candidate of invalidItems) {
      expect(ownerBetaRunSchema.safeParse({ ...valid, items: candidate }).success).toBe(false);
    }
    expect(ownerBetaRunSchema.safeParse({ ...valid, items: undefined }).success).toBe(false);
  });

  it("accepts every documented owner-beta error envelope code", () => {
    for (const code of [
      "RECOMMENDATION_CAPACITY_EXCEEDED",
      "OWNER_BETA_PRICE_INPUT_UNAVAILABLE",
      "OWNER_BETA_STRATEGY_UNSUPPORTED",
    ]) {
      expect(
        apiErrorEnvelopeSchema.safeParse({
          error: { code, message: "static public message", request_id: "request-1" },
        }).success,
      ).toBe(true);
    }
  });

  it("keeps the owner-beta contract labels on entitlement refusal", async () => {
    const api = ownerApi("REVOKED");
    vi.stubGlobal("fetch", api.fetcher);
    vi.stubEnv("API_INTERNAL_URL", "https://api.internal");

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Owner-only");
    expect(markup).toContain("Price-return only");
    expect(markup).toContain("Vendor snapshot");
    expect(markup).toContain("Non-strict PIT");
    expect(api.paths).not.toContain(OWNER_BETA_PRICE_ONLY_RUNS_PATH);
  });

  it("fails closed on a duplicated owner-beta run query", async () => {
    const api = ownerApi();
    vi.stubGlobal("fetch", api.fetcher);
    vi.stubEnv("API_INTERNAL_URL", "https://api.internal");

    const markup = renderToStaticMarkup(
      await RecommendationsPage({
        searchParams: Promise.resolve({ run_id: [RUN_ID, RUN_ID] }),
      }),
    );

    expect(markup).toContain("Owner-only recommendations");
    expect(markup).toContain("Vendor snapshot");
    expect(markup).toContain("Recommendations unavailable");
    expect(api.paths).not.toContain(OWNER_BETA_PRICE_ONLY_RUNS_PATH);
  });

  it("keeps the exact owner-beta mutation route separate from generic recommendation runs", () => {
    expect(OWNER_BETA_PRICE_ONLY_RUNS_PATH).toBe(
      "/api/v1/recommendations/owner-beta/price-only/runs",
    );
  });

  it("submits the form body to the exact owner-beta POST route", async () => {
    const requests: Array<{
      readonly body: unknown;
      readonly method: string;
      readonly url: string;
    }> = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      if (request.url.endsWith("/api/v1/auth/csrf")) {
        return Response.json({ csrf_token: "owner-beta-csrf" });
      }
      requests.push({
        body: await request.json(),
        method: request.method,
        url: request.url,
      });
      return Response.json(
        {
          job_id: JOB_ID,
          run_id: RUN_ID,
          status: "PENDING",
        },
        { status: 202 },
      );
    };

    const response = await submitOwnerBetaRun(
      { as_of: "2026-08-19", strategy_config_id: CONFIG_ID },
      { fetcher, origin: "https://lagrange.test" },
    );

    expect(response.status).toBe("PENDING");
    expect(requests).toEqual([
      {
        body: { as_of: "2026-08-19", strategy_config_id: CONFIG_ID },
        method: "POST",
        url: `https://lagrange.test${OWNER_BETA_PRICE_ONLY_RUNS_PATH}`,
      },
    ]);
  });

  it("refreshes after terminal detail polling on the owner-beta route", async () => {
    const urls: string[] = [];
    const refresh = vi.fn();
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(new URL(String(input), "https://lagrange.test"), init);
      urls.push(request.url);
      return Response.json(ownerRun());
    };

    const status = await pollOwnerBetaRun(RUN_ID, { fetcher, refresh });

    expect(status).toBe("SUCCEEDED");
    expect(urls).toEqual([`https://lagrange.test${OWNER_BETA_PRICE_ONLY_RUNS_PATH}/${RUN_ID}`]);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("keeps the existing generic page in disabled mode", async () => {
    const api = normalApi();
    vi.stubGlobal("fetch", api.fetcher);
    vi.stubEnv("API_INTERNAL_URL", "https://api.internal");

    const markup = renderToStaticMarkup(await RecommendationsPage());

    expect(markup).toContain("Generate strategy proposal");
    expect(markup).toContain("069500.KRX");
    expect(api.paths).toContain("/api/v1/recommendations/latest");
    expect(api.paths).toContain("/api/v1/recommendations/runs");
    expect(api.paths).not.toContain(OWNER_BETA_PRICE_ONLY_RUNS_PATH);
  });
});
