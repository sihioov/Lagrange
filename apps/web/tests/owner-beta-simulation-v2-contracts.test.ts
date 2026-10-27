import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import {
  eligibleOwnerBetaSimulationConfigs,
  OWNER_BETA_SIMULATION_CONTRACT_VERSION,
  ownerBetaSimulationPairMatchesRun,
  ownerBetaSimulationPairSchema,
  ownerBetaSimulationRunBodySchema,
  ownerBetaSimulationRunBodyV1Schema,
  ownerBetaSimulationRunMatchesRequest,
  ownerBetaSimulationVersionedRunSchema,
} from "@/lib/products/owner-beta-simulation-contracts";
import {
  OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256,
  OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
  OWNER_BETA_SIMULATION_V2_INSTRUMENTS,
  OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION,
  OWNER_BETA_SIMULATION_V2_STRATEGY_IDS,
  type OwnerBetaSimulationStrategyId,
  ownerBetaSimulationResultV2Schema,
  ownerBetaSimulationRunBodyV2Schema,
  ownerBetaSimulationRunV2Schema,
} from "@/lib/products/owner-beta-simulation-v2-contracts";

const fixtureUrl = pathToFileURL(
  `${process.cwd()}/tests/e2e/support/owner-beta-simulation-fixture.mjs`,
);
const {
  OWNER_BETA_SIMULATION_CONFIG_ID,
  OWNER_BETA_SIMULATION_RUNS_PATH,
  ownerBetaSimulationCanonicalConfigs,
  ownerBetaSimulationPair,
  ownerBetaSimulationResponse,
  ownerBetaSimulationV2Pair,
  resetOwnerBetaSimulationFixture,
} = await import(fixtureUrl.href);

const CONFIG_IDS = [
  "00000000-0000-4000-8000-000000000801",
  "00000000-0000-4000-8000-000000000802",
  "00000000-0000-4000-8000-000000000803",
  "00000000-0000-4000-8000-000000000804",
  "00000000-0000-4000-8000-000000000805",
] as const;
const RUN_ID = "00000000-0000-4000-8000-000000000811";
const JOB_ID = "00000000-0000-4000-8000-000000000821";
const FACTOR_SHA256 = `sha256:${"a".repeat(64)}`;

function runV2(overrides: Record<string, unknown> = {}) {
  return {
    audience: "OWNER_ONLY",
    capability: "PRICE_RETURN_ONLY",
    contract_document_sha256: OWNER_BETA_SIMULATION_V2_CONTRACT_SHA256,
    contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
    created_at: "2026-08-29T06:00:00Z",
    end_date: "2026-08-28",
    error_code: null,
    evaluation_session_count: 2,
    finished_at: null,
    id: RUN_ID,
    job_id: JOB_ID,
    result_schema_version: OWNER_BETA_SIMULATION_V2_RESULT_SCHEMA_VERSION,
    started_at: null,
    start_date: "2026-08-27",
    status: "PENDING",
    strategy_config_id: CONFIG_IDS[0],
    strategy_id: "trend_following",
    strategy_version: "1.0.0",
    strict_pit: false,
    updated_at: "2026-08-29T06:00:00Z",
    vendor_snapshot: true,
    ...overrides,
  };
}

function relativeMomentumMonthEndResult() {
  const result = structuredClone(
    ownerBetaSimulationV2Pair("2026-08-28", "2026-10-01").illustrative_10bp_per_side.result,
  );
  const initialDecision = result.schedule.decisions[0];
  if (initialDecision === undefined) throw new Error("synthetic initial decision is missing");
  const decisions = [
    initialDecision,
    {
      execution_date: "2026-09-01",
      kind: "MONTH_END_SIGNAL",
      signal_date: "2026-08-31",
    },
    {
      execution_date: "2026-10-01",
      kind: "MONTH_END_SIGNAL",
      signal_date: "2026-09-30",
    },
  ];
  const target = result.ledger.target_observations[0]?.target;
  const rebalanceTemplate = result.ledger.rebalances[0];
  if (target === undefined || rebalanceTemplate === undefined) {
    throw new Error("synthetic target or rebalance is missing");
  }

  result.schedule.strategy = "relative_momentum";
  result.schedule.decisions = decisions;
  result.strategy = {
    canonical_config_json: '{"lookback_months":12,"top_n":3}',
    factors: [{ name: "momentum_12_1", sha256: FACTOR_SHA256, version: "factor-v1" }],
    snapshot_sha256: "sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07",
    strategy: "relative_momentum",
    version: "1.0.0",
  };
  result.ledger.target_observations = decisions.map((decision) => ({
    decision_kind: decision.kind,
    execution_date: decision.execution_date,
    signal_date: decision.signal_date,
    target: structuredClone(target),
  }));
  result.ledger.rebalances = decisions.map((decision) => {
    const market = result.ledger.market_sessions.find(
      (session: { date: string; prices: unknown[] }) => session.date === decision.execution_date,
    );
    if (market === undefined) throw new Error("synthetic scheduled market session is missing");
    return {
      ...structuredClone(rebalanceTemplate),
      decision_kind: decision.kind,
      execution_date: decision.execution_date,
      open_prices: structuredClone(market.prices),
      signal_date: decision.signal_date,
      target: structuredClone(target),
    };
  });
  return result;
}

function trendHoldResult() {
  const result = structuredClone(
    ownerBetaSimulationV2Pair("2026-08-27", "2026-08-31").illustrative_10bp_per_side.result,
  );
  const initialDecision = result.schedule.decisions[0];
  if (initialDecision === undefined) throw new Error("synthetic initial decision is missing");
  const decisions = [
    initialDecision,
    ...result.schedule.evaluation_sessions
      .slice(0, -1)
      .map((signalDate: string, index: number) => ({
        execution_date: result.schedule.evaluation_sessions[index + 1],
        kind: "DAILY_TREND_TRANSITION_CHECK",
        signal_date: signalDate,
      })),
  ];
  const target = result.ledger.target_observations[0]?.target;
  if (target === undefined) throw new Error("synthetic target is missing");

  result.schedule.strategy = "trend_following";
  result.schedule.decisions = decisions;
  result.strategy = {
    canonical_config_json: '{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}',
    factors: [
      { name: "trend_50", sha256: FACTOR_SHA256, version: "factor-v1" },
      { name: "trend_200", sha256: FACTOR_SHA256, version: "factor-v1" },
    ],
    snapshot_sha256: "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0",
    strategy: "trend_following",
    version: "1.0.0",
  };
  result.ledger.target_observations = decisions.map((decision) => ({
    decision_kind: decision.kind,
    execution_date: decision.execution_date,
    signal_date: decision.signal_date,
    target: structuredClone(target),
  }));
  return result;
}

describe("owner-beta simulation v2 contracts", () => {
  it("keeps missing and explicit v1 selectors on the original v1 request shape", () => {
    const withoutSelector = ownerBetaSimulationRunBodySchema.parse({
      strategy_config_id: CONFIG_IDS[0],
    });
    const explicitV1 = ownerBetaSimulationRunBodySchema.parse({
      contract_version: OWNER_BETA_SIMULATION_CONTRACT_VERSION,
      strategy_config_id: CONFIG_IDS[0],
    });

    expect(withoutSelector).toEqual({ strategy_config_id: CONFIG_IDS[0] });
    expect(explicitV1).toEqual(withoutSelector);
    expect(
      ownerBetaSimulationRunBodyV1Schema.safeParse({
        contract_version: null,
        strategy_config_id: CONFIG_IDS[0],
      }).success,
    ).toBe(false);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        contract_version: "unknown",
        strategy_config_id: CONFIG_IDS[0],
      }).success,
    ).toBe(false);
    expect(
      ownerBetaSimulationRunBodySchema.safeParse({
        contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
        strategy_config_id: CONFIG_IDS[0],
        unsupported: true,
      }).success,
    ).toBe(false);
  });

  it("accepts the explicit v2 form body with independently optional dates", () => {
    expect(
      ownerBetaSimulationRunBodySchema.parse({
        contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
        strategy_config_id: CONFIG_IDS[0],
      }),
    ).toEqual({
      contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
      strategy_config_id: CONFIG_IDS[0],
    });
    expect(
      ownerBetaSimulationRunBodyV2Schema.safeParse({
        contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
        strategy_config_id: CONFIG_IDS[0],
        start_date: "2026-08-29",
        end_date: "2026-08-28",
      }).success,
    ).toBe(false);
  });

  it("rejects custom request fields in the synthetic enqueue route", () => {
    resetOwnerBetaSimulationFixture();
    const response = ownerBetaSimulationResponse({
      body: {
        contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
        custom: true,
        strategy_config_id: OWNER_BETA_SIMULATION_CONFIG_ID,
      },
      headers: { "idempotency-key": "fixture-run", "x-csrf-token": "fixture-csrf" },
      method: "POST",
      pathname: OWNER_BETA_SIMULATION_RUNS_PATH,
      query: "",
      scenario: { ownerBetaSimulation: "enabled" },
    });
    expect(response?.status).toBe(422);
    resetOwnerBetaSimulationFixture();
  });

  it("keeps v2_result list, metadata, and pair on the same checked run contract", () => {
    resetOwnerBetaSimulationFixture();
    try {
      const scenario = { ownerBetaSimulation: "v2_result" };
      const listResponse = ownerBetaSimulationResponse({
        headers: {},
        method: "GET",
        pathname: OWNER_BETA_SIMULATION_RUNS_PATH,
        query: "",
        scenario,
      });
      const listedRun = ownerBetaSimulationRunV2Schema.parse(listResponse?.body.items[0]);

      const metadataResponse = ownerBetaSimulationResponse({
        headers: {},
        method: "GET",
        pathname: `${OWNER_BETA_SIMULATION_RUNS_PATH}/${RUN_ID}`,
        query: "",
        scenario,
      });
      const metadataRun = ownerBetaSimulationRunV2Schema.parse(metadataResponse?.body);

      const pairResponse = ownerBetaSimulationResponse({
        headers: {},
        method: "GET",
        pathname: `${OWNER_BETA_SIMULATION_RUNS_PATH}/${RUN_ID}/pair`,
        query: "",
        scenario,
      });
      const pair = ownerBetaSimulationPairSchema.parse(pairResponse?.body);
      const illustrativeResult = pair.illustrative_10bp_per_side.result;
      if (!("schema" in illustrativeResult)) {
        throw new Error("v2_result fixture returned a v1 pair");
      }

      expect(listResponse?.status).toBe(200);
      expect(metadataResponse?.status).toBe(200);
      expect(pairResponse?.status).toBe(200);
      expect(metadataRun).toEqual(listedRun);
      expect(metadataRun.contract_version).toBe(OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION);
      expect(metadataRun.strategy_id).toBe(illustrativeResult.strategy.strategy);
      expect(metadataRun.start_date).toBe(illustrativeResult.schedule.evaluation_sessions[0]);
      expect(metadataRun.end_date).toBe(illustrativeResult.schedule.evaluation_sessions.at(-1));
      expect(metadataRun.evaluation_session_count).toBe(
        illustrativeResult.schedule.evaluation_sessions.length,
      );
      expect(ownerBetaSimulationPairMatchesRun(pair, metadataRun)).toBe(true);
    } finally {
      resetOwnerBetaSimulationFixture();
    }
  });

  it("filters to all five active canonical version-one defaults and rejects custom keys", () => {
    const defaults = ownerBetaSimulationCanonicalConfigs();
    const eligible = eligibleOwnerBetaSimulationConfigs(defaults);
    expect(eligible).toHaveLength(5);
    expect(eligible.map((config) => config.simulationStrategyId)).toEqual(
      OWNER_BETA_SIMULATION_V2_STRATEGY_IDS,
    );
    expect(eligible.map((config) => config.id)).toEqual([
      CONFIG_IDS[1],
      CONFIG_IDS[0],
      CONFIG_IDS[2],
      CONFIG_IDS[3],
      CONFIG_IDS[4],
    ]);
    expect(
      eligibleOwnerBetaSimulationConfigs([
        { ...defaults[0], config: { ...defaults[0].config, extra: true } },
        { ...defaults[1], is_active: false },
        { ...defaults[2], strategy_version: "2.0.0" },
      ]),
    ).toEqual([]);
  });

  it("validates the synthetic v2 wire result and rejects schedule, period, and market tampering", () => {
    const pair = ownerBetaSimulationV2Pair();
    const result = pair.illustrative_10bp_per_side.result;
    expect(ownerBetaSimulationResultV2Schema.safeParse(result).success).toBe(true);
    expect(ownerBetaSimulationPairSchema.safeParse(pair).success).toBe(true);

    const changedDailyDate = structuredClone(result);
    changedDailyDate.ledger.metrics.daily_returns[0].session_date = "2026-08-28";
    expect(ownerBetaSimulationResultV2Schema.safeParse(changedDailyDate).success).toBe(false);

    const changedDecision = structuredClone(result);
    changedDecision.schedule.decisions[0].signal_date = "2026-08-24";
    expect(ownerBetaSimulationResultV2Schema.safeParse(changedDecision).success).toBe(false);

    const changedMarketPath = structuredClone(result);
    changedMarketPath.benchmark.market_sessions[0].prices[0].close = "1001.0000";
    expect(ownerBetaSimulationResultV2Schema.safeParse(changedMarketPath).success).toBe(false);
  });

  it("accepts Rust-shaped zero targets for both cost profiles and benchmark ledgers", () => {
    const pair = ownerBetaSimulationV2Pair();
    const expectedTargets = OWNER_BETA_SIMULATION_V2_INSTRUMENTS.map((instrument_id, index) => ({
      instrument_id,
      quantity: index === 0 ? 10_000 : 0,
    }));

    for (const profile of [pair.illustrative_10bp_per_side, pair.zero_cost_reference]) {
      expect(profile.result.ledger.rebalances[0].target_quantities).toEqual(expectedTargets);
      expect(profile.result.benchmark.rebalances[0].target_quantities).toEqual(expectedTargets);
      expect(ownerBetaSimulationResultV2Schema.safeParse(profile.result).success).toBe(true);
    }
    expect(ownerBetaSimulationPairSchema.safeParse(pair).success).toBe(true);
  });

  it("rejects missing, duplicate, unsorted, or unknown target instruments", () => {
    const result = ownerBetaSimulationV2Pair().zero_cost_reference.result;
    const originalTargets = result.ledger.rebalances[0].target_quantities;
    const malformedTargets = [
      originalTargets.slice(1),
      [originalTargets[0], { ...originalTargets[0] }, ...originalTargets.slice(2)],
      [originalTargets[1], originalTargets[0], ...originalTargets.slice(2)],
      [{ ...originalTargets[0], instrument_id: "000000.KRX" }, ...originalTargets.slice(1)],
    ];

    for (const target_quantities of malformedTargets) {
      const candidate = structuredClone(result);
      candidate.ledger.rebalances[0].target_quantities = target_quantities;
      expect(ownerBetaSimulationResultV2Schema.safeParse(candidate).success).toBe(false);
    }
  });

  it("rejects invalid target quantities and still rejects zero holdings", () => {
    const result = ownerBetaSimulationV2Pair().zero_cost_reference.result;
    for (const quantity of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, -0]) {
      const candidate = structuredClone(result);
      candidate.ledger.rebalances[0].target_quantities[0].quantity = quantity;
      expect(ownerBetaSimulationResultV2Schema.safeParse(candidate).success).toBe(false);
    }

    const zeroHolding = structuredClone(result);
    zeroHolding.ledger.rebalances[0].positions_after[0].quantity = 0;
    expect(ownerBetaSimulationResultV2Schema.safeParse(zeroHolding).success).toBe(false);

    const zeroOpeningHolding = structuredClone(result);
    zeroOpeningHolding.ledger.daily_records[0].opening_positions[0].quantity = 0;
    expect(ownerBetaSimulationResultV2Schema.safeParse(zeroOpeningHolding).success).toBe(false);
  });

  it("rejects an off-period rebalance even when all daily target lineage is absent", () => {
    const result = relativeMomentumMonthEndResult();
    expect(ownerBetaSimulationResultV2Schema.safeParse(result).success).toBe(true);

    const movedOutsideEvaluation = structuredClone(result);
    movedOutsideEvaluation.ledger.rebalances[0].execution_date = "2026-10-02";
    for (const dailyRecord of movedOutsideEvaluation.ledger.daily_records) {
      dailyRecord.target_lineage = null;
    }
    expect(ownerBetaSimulationResultV2Schema.safeParse(movedOutsideEvaluation).success).toBe(false);
  });

  it("rejects shifted or reordered monthly rebalance lineage", () => {
    const result = relativeMomentumMonthEndResult();
    expect(ownerBetaSimulationResultV2Schema.safeParse(result).success).toBe(true);

    const shifted = structuredClone(result);
    shifted.ledger.rebalances[1].execution_date = "2026-09-02";
    expect(ownerBetaSimulationResultV2Schema.safeParse(shifted).success).toBe(false);

    const reordered = structuredClone(result);
    [reordered.ledger.rebalances[1], reordered.ledger.rebalances[2]] = [
      reordered.ledger.rebalances[2],
      reordered.ledger.rebalances[1],
    ];
    expect(ownerBetaSimulationResultV2Schema.safeParse(reordered).success).toBe(false);
  });

  it("keeps trend hold lineage on the active allocation between scheduled trades", () => {
    const result = trendHoldResult();
    const activeTarget = result.ledger.target_observations[0]?.target;
    expect(activeTarget).toBeDefined();
    expect(result.ledger.rebalances).toHaveLength(1);
    expect(
      result.ledger.daily_records.map(
        (record: { target_lineage: unknown }) => record.target_lineage,
      ),
    ).toEqual([activeTarget, activeTarget, activeTarget]);
    expect(ownerBetaSimulationResultV2Schema.safeParse(result).success).toBe(true);

    const missingHoldLineage = structuredClone(result);
    missingHoldLineage.ledger.daily_records[1].target_lineage = null;
    expect(ownerBetaSimulationResultV2Schema.safeParse(missingHoldLineage).success).toBe(false);
  });

  it("rejects mixed versions, incorrect profile slots, and mismatched pair paths", () => {
    const v2Pair = ownerBetaSimulationV2Pair();
    const v1Pair = ownerBetaSimulationPair();
    expect(
      ownerBetaSimulationPairSchema.safeParse({
        ...v2Pair,
        zero_cost_reference: v1Pair.zero_cost_reference,
      }).success,
    ).toBe(false);

    const wrongProfile = structuredClone(v2Pair);
    wrongProfile.zero_cost_reference.result.cost_profile = "ILLUSTRATIVE_10BP_PER_SIDE_V1";
    expect(ownerBetaSimulationPairSchema.safeParse(wrongProfile).success).toBe(false);

    const wrongEngine = structuredClone(v2Pair);
    wrongEngine.zero_cost_reference.result.engine.binary_sha256 = `sha256:${"c".repeat(64)}`;
    expect(ownerBetaSimulationPairSchema.safeParse(wrongEngine).success).toBe(false);

    const wrongTargetPath = structuredClone(v2Pair);
    wrongTargetPath.zero_cost_reference.result.ledger.target_observations[0].target.target_sha256 = `sha256:${"d".repeat(64)}`;
    expect(ownerBetaSimulationPairSchema.safeParse(wrongTargetPath).success).toBe(false);

    const wrongPairPeriod = structuredClone(v2Pair);
    wrongPairPeriod.zero_cost_reference = ownerBetaSimulationV2Pair(
      "2026-08-26",
      "2026-08-27",
    ).zero_cost_reference;
    expect(ownerBetaSimulationPairSchema.safeParse(wrongPairPeriod).success).toBe(false);
  });

  it("binds submitted v2 dates when present and accepts only the actual checked window when omitted", () => {
    const parsedRun = ownerBetaSimulationRunV2Schema.parse(
      runV2({
        start_date: "2026-08-20",
        end_date: "2026-08-21",
        evaluation_session_count: 2,
      }),
    );
    const omittedDates = {
      contract_version: OWNER_BETA_SIMULATION_V2_CONTRACT_VERSION,
      strategy_config_id: CONFIG_IDS[0],
    };
    expect(
      ownerBetaSimulationRunMatchesRequest(parsedRun, omittedDates, RUN_ID, "trend_following"),
    ).toBe(true);
    expect(
      ownerBetaSimulationRunMatchesRequest(
        parsedRun,
        { ...omittedDates, start_date: "2026-08-27" },
        RUN_ID,
        "trend_following",
      ),
    ).toBe(false);
    expect(
      ownerBetaSimulationRunMatchesRequest(
        parsedRun,
        { ...omittedDates, end_date: "2026-08-22" },
        RUN_ID,
        "trend_following",
      ),
    ).toBe(false);
    expect(
      ownerBetaSimulationRunMatchesRequest(parsedRun, omittedDates, RUN_ID, "buy_and_hold"),
    ).toBe(false);
    expect(
      ownerBetaSimulationRunV2Schema.safeParse(
        runV2({ contract_document_sha256: `sha256:${"e".repeat(64)}` }),
      ).success,
    ).toBe(false);
  });

  it("keeps historical v1 rows and pairs on their v1 validators", () => {
    const v1Pair = ownerBetaSimulationPair();
    expect(ownerBetaSimulationPairSchema.safeParse(v1Pair).success).toBe(true);
    const v2Run = runV2();
    expect(ownerBetaSimulationVersionedRunSchema.safeParse(v2Run).success).toBe(true);
    const v1Run = {
      ...v2Run,
      contract_document_sha256:
        "sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd",
      contract_version: OWNER_BETA_SIMULATION_CONTRACT_VERSION,
      result_schema_version: "owner-beta-price-only-result-v1",
    };
    expect(ownerBetaSimulationVersionedRunSchema.safeParse(v1Run).success).toBe(true);
    expect("schema" in v1Pair.illustrative_10bp_per_side.result).toBe(false);
  });

  it("binds validated v2 result pairs to the selected run's strategy and common dates", () => {
    const pair = ownerBetaSimulationV2Pair();
    const compatibleRun = ownerBetaSimulationRunV2Schema.parse(
      runV2({
        status: "SUCCEEDED",
        strategy_config_id: CONFIG_IDS[1],
        strategy_id: "buy_and_hold",
      }),
    );
    expect(ownerBetaSimulationPairMatchesRun(pair, compatibleRun)).toBe(true);

    const wrongStrategy = ownerBetaSimulationRunV2Schema.parse(
      runV2({ status: "SUCCEEDED", strategy_config_id: CONFIG_IDS[0] }),
    );
    expect(ownerBetaSimulationPairMatchesRun(pair, wrongStrategy)).toBe(false);

    const wrongPeriod = ownerBetaSimulationRunV2Schema.parse(
      runV2({
        end_date: "2026-08-27",
        evaluation_session_count: 2,
        start_date: "2026-08-26",
        status: "SUCCEEDED",
        strategy_config_id: CONFIG_IDS[1],
        strategy_id: "buy_and_hold",
      }),
    );
    expect(ownerBetaSimulationPairMatchesRun(pair, wrongPeriod)).toBe(false);
  });
});

const _strategyTypeCheck: OwnerBetaSimulationStrategyId = "trend_following";
void _strategyTypeCheck;
