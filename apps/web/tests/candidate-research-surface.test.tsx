import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import CandidatesPage from "@/app/(authenticated)/candidates/page";
import ScreenerPage from "@/app/(authenticated)/screener/page";
import StockPage from "@/app/(authenticated)/stocks/[instrument]/page";
import { candidateFeedSchema } from "@/lib/products/candidate-contracts";

vi.mock("server-only", () => ({}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

vi.mock("next/headers", () => ({
  cookies: async () => ({
    get: () => ({ name: "__Host-lagrange_session", value: "member-opaque" }),
  }),
}));

const RUN_ID = "00000000-0000-4000-8000-000000000501";
const KOSDAQ_RUN_ID = "00000000-0000-4000-8000-000000000511";
const FEED_ID = "00000000-0000-4000-8000-000000000502";
const KOSDAQ_FEED_ID = "00000000-0000-4000-8000-000000000512";
const AS_OF = "2026-08-14";
const SHA = "a".repeat(64);
const LICENSES = [
  ["price", "krx_eod_bars", "00000000-0000-4000-8000-000000000901"],
  ["universe", "krx_kospi200_membership", "00000000-0000-4000-8000-000000000902"],
  ["market_status", "krx_market_status", "00000000-0000-4000-8000-000000000903"],
  ["flow", "krx_investor_flows", "00000000-0000-4000-8000-000000000904"],
  ["fundamental", "krx_fundamentals", "00000000-0000-4000-8000-000000000905"],
  ["sector", "krx_sector_classification", "00000000-0000-4000-8000-000000000906"],
].map(([source, dataset_id, entitlement_id]) => ({
  source,
  dataset_id,
  license_ref: `fixture://${source}`,
  entitlement_id,
  contract_reference: `vault://candidate/${source}`,
  contract_document_sha256: SHA,
}));

function analysis(index: number, universe: "kospi200" | "kosdaq150" = "kospi200") {
  const instrument = `${String(5930 + index).padStart(6, "0")}.KRX`;
  return {
    analysis_id: `00000000-0000-4000-8000-${String((universe === "kospi200" ? 600 : 700) + index).padStart(12, "0")}`,
    run_id: universe === "kospi200" ? RUN_ID : KOSDAQ_RUN_ID,
    universe,
    instrument_id: instrument,
    name: `Synthetic ${index + 1}`,
    sector_code: index === 0 ? "G40" : "G25",
    fundamental_profile:
      index === 0 ? ("candidate-financial-v1" as const) : ("candidate-non-financial-v1" as const),
    eligible: true,
    exclusion_codes: [],
    scores: {
      flow: 78 - index,
      fundamental: 72 - index,
      technical: 81 - index,
      total: 77 - index,
    },
    coverage: { flow: 1, fundamental: 0.9, technical: 1 },
    evidence_strength: index < 2 ? ("STRONG" as const) : ("MODERATE" as const),
    rank: index + 1,
    normalization_scope: "UNIVERSE_FALLBACK" as const,
    factors: {
      foreign_intensity_20: {
        normalization_scope: "UNIVERSE_FALLBACK",
        normalized: 1.2,
        raw: 0.04,
        weight: 0.18,
      },
    },
    scenarios: {
      bullish: {
        evidence_refs: ["foreign_intensity_20"],
        label: "BULLISH",
        title: "상승 경로",
        trigger_expression: "flow_score > 55 AND technical_score > 55",
      },
      neutral: {
        evidence_refs: ["foreign_intensity_20"],
        label: "NEUTRAL",
        title: "중립 경로",
        trigger_expression: "ABS(composite_score - 50) <= 5",
      },
      bearish: {
        evidence_refs: ["foreign_intensity_20"],
        label: "BEARISH",
        title: "하락 경로",
        trigger_expression: "technical_score < 45 OR flow_score < 45",
      },
    },
    provenance: { input_identity_sha256: SHA },
    content_sha256: SHA,
  };
}

const envelope = {
  universe: "kospi200" as const,
  state: "READY" as const,
  as_of: AS_OF,
  cutoff_at: "2026-08-14T07:00:00Z",
  scoring_config: { version: "candidate-score-v1", sha256: SHA },
  dataset_pins: {
    universe_snapshot_id: "00000000-0000-4000-8000-000000000701",
    price: {
      dataset_version_id: "00000000-0000-4000-8000-000000000702",
      curated_version: 2,
      manifest_sha256: SHA,
    },
    market_status: {
      dataset_version_id: "00000000-0000-4000-8000-000000000703",
      manifest_sha256: SHA,
    },
    flow: {
      dataset_version_id: "00000000-0000-4000-8000-000000000704",
      manifest_sha256: SHA,
    },
    fundamental: {
      dataset_version_id: "00000000-0000-4000-8000-000000000705",
      manifest_sha256: SHA,
    },
    sector_version_id: "00000000-0000-4000-8000-000000000706",
    input_identity_sha256: SHA,
  },
  license_attributions: LICENSES,
  disclaimer: "연구 정보이며 투자 권유, 목표가 또는 수익 확률이 아닙니다.",
};

const feed = {
  ...envelope,
  feed_id: FEED_ID,
  published_at: "2026-08-14T07:05:00Z",
  computation_seq: 1,
  items: Array.from({ length: 5 }, (_, index) => analysis(index)),
};

const kosdaqFeed = {
  ...envelope,
  universe: "kosdaq150" as const,
  feed_id: KOSDAQ_FEED_ID,
  published_at: "2026-08-14T07:05:00Z",
  computation_seq: 1,
  items: Array.from({ length: 5 }, (_, index) => analysis(index, "kosdaq150")),
};

function api(
  options: {
    readonly blocked?: boolean;
    readonly stale?: boolean;
    readonly notFound?: boolean;
  } = {},
): {
  readonly calls: Request[];
  readonly fetcher: typeof fetch;
} {
  const calls: Request[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const request = new Request(input, init);
    calls.push(request);
    const { pathname } = new URL(request.url);
    if (pathname.startsWith("/api/v1/candidates/")) {
      if (options.blocked) {
        return Response.json(
          {
            error: {
              code: "DATASET_BLOCKED",
              message: "candidate entitlement is inactive",
              request_id: "request-candidate",
            },
          },
          { status: 403 },
        );
      }
      if (options.notFound) {
        return Response.json(
          {
            error: {
              code: "RESOURCE_NOT_FOUND",
              message: "candidate feed does not exist",
              request_id: "request-candidate",
            },
          },
          { status: 404 },
        );
      }
      const universe = new URL(request.url).searchParams.get("universe");
      const selectedFeed = universe === "kosdaq150" ? kosdaqFeed : feed;
      return Response.json(options.stale ? { ...selectedFeed, state: "STALE" } : selectedFeed, {
        headers: { "Cache-Control": "no-store" },
      });
    }
    if (pathname === `/api/v1/stocks/${feed.items[0]?.instrument_id}/analysis`) {
      const universe = new URL(request.url).searchParams.get("universe");
      const selectedFeed = universe === "kosdaq150" ? kosdaqFeed : feed;
      return Response.json(
        {
          ...envelope,
          universe: selectedFeed.universe,
          analysis: selectedFeed.items[0],
        },
        { headers: { "Cache-Control": "no-store" } },
      );
    }
    if (pathname === "/api/v1/screener/query") {
      const body = (await request.clone().json()) as {
        readonly criteria?: { readonly universes?: readonly string[] };
      };
      const universes = body.criteria?.universes ?? ["kospi200"];
      const items = universes.flatMap((universe) =>
        universe === "kosdaq150" ? [kosdaqFeed.items[0]] : [feed.items[0]],
      );
      return Response.json(
        {
          ...envelope,
          universe: universes.length === 1 ? universes[0] : null,
          universes,
          run_id: universes.length === 1 ? RUN_ID : null,
          run_ids: universes.map((universe) => ({
            universe,
            run_id: universe === "kosdaq150" ? KOSDAQ_RUN_ID : RUN_ID,
          })),
          items,
          next_cursor: "signed-next",
        },
        { headers: { "Cache-Control": "no-store" } },
      );
    }
    if (pathname === "/api/v1/screener/screens") {
      return Response.json({
        items: [
          {
            id: "00000000-0000-4000-8000-000000000801",
            name: "Strong financial flow",
            criteria_schema_version: 1,
            criteria: {
              sectors: ["G40"],
              evidence_strength: ["STRONG"],
              min_total_score: 70,
              min_flow_score: null,
              min_fundamental_score: null,
              min_technical_score: null,
            },
            created_at: "2026-08-14T08:00:00Z",
            updated_at: "2026-08-14T08:00:00Z",
          },
          {
            id: "00000000-0000-4000-8000-000000000802",
            name: "KOSDAQ technical screen",
            criteria_schema_version: 2,
            criteria: {
              universes: ["kosdaq150"],
              sectors: ["G25"],
              evidence_strength: ["MODERATE"],
              min_total_score: 60,
            },
            created_at: "2026-08-14T08:00:00Z",
            updated_at: "2026-08-14T08:00:00Z",
          },
        ],
      });
    }
    return Response.json(
      {
        error: {
          code: "RESOURCE_NOT_FOUND",
          message: `No response for ${pathname}`,
          request_id: "request-candidate",
        },
      },
      { status: 404 },
    );
  };
  return { calls, fetcher };
}

beforeEach(() => {
  vi.stubEnv("API_INTERNAL_URL", "https://api.internal");
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("candidate research surfaces", () => {
  it("renders an exact Top 5 with immutable lineage and deep-analysis links", async () => {
    const server = api();
    vi.stubGlobal("fetch", server.fetcher);

    const markup = renderToStaticMarkup(await CandidatesPage());

    expect(markup.match(/Synthetic \d/g)).toHaveLength(5);
    expect(markup).toContain("Common daily Top 5");
    expect(markup).toContain("KOSPI 200");
    expect(markup).toContain(`/stocks/${feed.items[0]?.instrument_id}?date=${AS_OF}`);
    expect(markup).toContain(`universe=kospi200`);
    expect(markup).toContain("Exact dataset pins");
    expect(markup).toContain("not probabilities or target prices");
    expect(server.calls.every((request) => request.cache === "no-store")).toBe(true);
  });

  it("uses the selected KOSDAQ tab for the feed and keeps its universe in links", async () => {
    const server = api();
    vi.stubGlobal("fetch", server.fetcher);

    const markup = renderToStaticMarkup(
      await CandidatesPage({ searchParams: Promise.resolve({ universe: "kosdaq150" }) }),
    );

    expect(markup).toContain("KOSDAQ 150");
    expect(markup).toContain("Synthetic 1");
    expect(
      server.calls.some(
        (request) => new URL(request.url).searchParams.get("universe") === "kosdaq150",
      ),
    ).toBe(true);
    expect(markup).toContain("universe=kosdaq150");
  });

  it("renders financial-profile evidence and three deterministic scenarios", async () => {
    vi.stubGlobal("fetch", api().fetcher);

    const markup = renderToStaticMarkup(
      await StockPage({
        params: Promise.resolve({ instrument: feed.items[0]?.instrument_id ?? "" }),
        searchParams: Promise.resolve({ date: AS_OF }),
      }),
    );

    expect(markup).toContain("Financial-company profile");
    expect(markup).toContain("Foreign &amp; institution flow");
    expect(markup).toContain("상승 경로");
    expect(markup).toContain("중립 경로");
    expect(markup).toContain("하락 경로");
    expect(markup).toContain("flow_score &gt; 55 AND technical_score &gt; 55");
    expect(markup).toContain("KOSPI 200");
    expect(markup).toContain("Rank");
    expect(markup).not.toMatch(/\b\d+(?:\.\d+)?% chance\b/i);
  });

  it("groups both-universe screener rows and keeps duplicate instruments separate", async () => {
    const server = api();
    vi.stubGlobal("fetch", server.fetcher);

    const markup = renderToStaticMarkup(
      await ScreenerPage({
        searchParams: Promise.resolve({
          as_of: AS_OF,
          universes: ["kospi200", "kosdaq150"],
        }),
      }),
    );
    const query = server.calls.find(
      (request) => new URL(request.url).pathname === "/api/v1/screener/query",
    );

    await expect(query?.clone().json()).resolves.toMatchObject({
      criteria: { universes: ["kospi200", "kosdaq150"] },
    });
    expect(markup).toContain("KOSPI 200");
    expect(markup).toContain("KOSDAQ 150");
    expect(markup.match(/Synthetic 1/g)).toHaveLength(2);
    expect(markup).not.toContain("Global rank");
  });

  it("restores v1 saved screens as KOSPI and preserves explicit v2 universes", async () => {
    const server = api();
    vi.stubGlobal("fetch", server.fetcher);

    const markup = renderToStaticMarkup(
      await ScreenerPage({ searchParams: Promise.resolve({ as_of: AS_OF }) }),
    );

    expect(markup).toContain("Strong financial flow");
    expect(markup).toContain("KOSDAQ technical screen");
    expect(markup).toContain("universes=kospi200");
    expect(markup).toContain("universes=kosdaq150");
  });

  it("queries the exact published run and preserves private saved screens", async () => {
    const server = api();
    vi.stubGlobal("fetch", server.fetcher);

    const markup = renderToStaticMarkup(
      await ScreenerPage({
        searchParams: Promise.resolve({
          as_of: AS_OF,
          sectors: "G40",
          evidence: ["STRONG"],
          min_total_score: "70",
        }),
      }),
    );
    const query = server.calls.find(
      (request) => new URL(request.url).pathname === "/api/v1/screener/query",
    );

    expect(query?.method).toBe("POST");
    await expect(query?.clone().json()).resolves.toMatchObject({
      run_id: RUN_ID,
      criteria: { sectors: ["G40"], evidence_strength: ["STRONG"], min_total_score: 70 },
    });
    expect(markup).toContain("Strong financial flow");
    expect(markup).toContain("signed-next");
    expect(markup).toContain(feed.items[0]?.instrument_id);
  });

  it("fails closed without leaking candidate rows when one source is blocked", async () => {
    vi.stubGlobal("fetch", api({ blocked: true }).fetcher);

    const markup = renderToStaticMarkup(await CandidatesPage());

    expect(markup).toContain("Candidate research is blocked");
    expect(markup).not.toContain("Synthetic 1");
    expect(markup).not.toContain(SHA);
  });

  it("keeps stale and not-found errors distinct from blocked access", async () => {
    vi.stubGlobal("fetch", api({ stale: true }).fetcher);
    const stale = renderToStaticMarkup(await CandidatesPage());
    expect(stale).toContain("Stale research snapshot");
    expect(stale).not.toContain("Candidate research is blocked");
    expect(stale).toContain("Synthetic 1");

    vi.stubGlobal("fetch", api({ notFound: true }).fetcher);
    const notFound = renderToStaticMarkup(await CandidatesPage());
    expect(notFound).toContain("No candidate snapshot");
    expect(notFound).not.toContain("Candidate research is blocked");
  });

  it("rejects a malicious BLOCKED success payload before any proprietary row can render", () => {
    const malicious = {
      ...feed,
      state: "BLOCKED",
      items: [feed.items[0]],
    };
    expect(() => candidateFeedSchema.parse(malicious)).toThrow();
  });
});
