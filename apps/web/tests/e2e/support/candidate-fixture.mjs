const RUN_ID = "00000000-0000-4000-8000-000000000501";
const FEED_ID = "00000000-0000-4000-8000-000000000502";
const SCREEN_ID = "00000000-0000-4000-8000-000000000801";
const AS_OF = "2026-08-14";
const SHA = "a".repeat(64);

function userIndex(scenario) {
  const match = /^u(\d)$/.exec(scenario.user ?? "u1");
  return match ? Number(match[1]) : 1;
}

function idFor(scenario, baseId) {
  const index = userIndex(scenario);
  return index === 1 ? baseId : `${baseId.slice(0, -4)}${index}${baseId.slice(-3)}`;
}

function analysis(index) {
  const instrument = `${String(5930 + index).padStart(6, "0")}.KRX`;
  return {
    analysis_id: `00000000-0000-4000-8000-${String(600 + index).padStart(12, "0")}`,
    run_id: RUN_ID,
    instrument_id: instrument,
    name: `Synthetic ${index + 1}`,
    sector_code: index === 0 ? "G40" : "G25",
    fundamental_profile: index === 0 ? "candidate-financial-v1" : "candidate-non-financial-v1",
    eligible: true,
    exclusion_codes: [],
    scores: {
      flow: 78 - index,
      fundamental: 72 - index,
      technical: 81 - index,
      total: 77 - index,
    },
    coverage: { flow: 1, fundamental: 0.9, technical: 1 },
    evidence_strength: index < 2 ? "STRONG" : "MODERATE",
    rank: index + 1,
    normalization_scope: "UNIVERSE_FALLBACK",
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

function envelope(scenario) {
  return {
    state: scenario.candidateState === "stale" ? "STALE" : "READY",
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
    license_attributions: [
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
    })),
    disclaimer: "연구 정보이며 투자 권유, 목표가 또는 수익 확률이 아닙니다.",
  };
}

function feed(scenario) {
  return {
    ...envelope(scenario),
    feed_id: FEED_ID,
    published_at: "2026-08-14T07:05:00Z",
    computation_seq: 1,
    items: Array.from({ length: 5 }, (_, index) => analysis(index)),
  };
}

function savedScreen(scenario, name = `Private screen ${userIndex(scenario)}`) {
  return {
    id: idFor(scenario, SCREEN_ID),
    name,
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
  };
}

function error(status, code, message) {
  return {
    body: { error: { code, message, request_id: "request-synthetic-candidate" } },
    status,
  };
}

function mutationAuthorized(headers) {
  return Boolean(headers["x-csrf-token"] && headers["idempotency-key"]);
}

export function candidateResponse(request) {
  const { body, headers, method, pathname, scenario } = request;
  const isCandidatePath =
    pathname.startsWith("/api/v1/candidates/") ||
    pathname.startsWith("/api/v1/stocks/") ||
    pathname.startsWith("/api/v1/screener/");
  if (!isCandidatePath) return null;
  if (scenario.candidateState === "blocked") {
    return error(403, "DATASET_BLOCKED", "candidate source entitlement is inactive");
  }
  const currentFeed = feed(scenario);
  if (
    method === "GET" &&
    (pathname === "/api/v1/candidates/feed/latest" ||
      /^\/api\/v1\/candidates\/feed\/\d{4}-\d{2}-\d{2}$/.test(pathname))
  ) {
    return { body: currentFeed, status: 200 };
  }
  const stock = /^\/api\/v1\/stocks\/([^/]+)\/analysis$/.exec(pathname);
  if (method === "GET" && stock !== null) {
    const item = currentFeed.items.find((candidate) => candidate.instrument_id === stock[1]);
    return item === undefined
      ? error(404, "RESOURCE_NOT_FOUND", "candidate analysis does not exist")
      : { body: { ...envelope(scenario), analysis: item }, status: 200 };
  }
  if (method === "POST" && pathname === "/api/v1/screener/query") {
    const minimum = Number(body?.criteria?.min_total_score ?? 0);
    const sectors = Array.isArray(body?.criteria?.sectors) ? body.criteria.sectors : [];
    const items = currentFeed.items.filter(
      (item) =>
        item.scores.total >= minimum &&
        (sectors.length === 0 || sectors.includes(item.sector_code)),
    );
    return {
      body: { ...envelope(scenario), run_id: RUN_ID, items, next_cursor: null },
      status: 200,
    };
  }
  if (method === "GET" && pathname === "/api/v1/screener/screens") {
    return { body: { items: [savedScreen(scenario)] }, status: 200 };
  }
  if (method === "POST" && pathname === "/api/v1/screener/screens") {
    return mutationAuthorized(headers)
      ? { body: savedScreen(scenario, body?.name), status: 201 }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  const screen = /^\/api\/v1\/screener\/screens\/([^/]+)$/.exec(pathname);
  if (method === "DELETE" && screen !== null) {
    return mutationAuthorized(headers)
      ? { body: { id: screen[1], deleted: true }, status: 200 }
      : error(403, "CSRF_DENIED", "CSRF and idempotency headers are required");
  }
  return null;
}
