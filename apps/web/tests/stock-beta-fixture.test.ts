import { pathToFileURL } from "node:url";
import { beforeEach, describe, expect, it } from "vitest";
import { CurrentQuoteClient } from "@/components/stock-beta/quote/current-quote-client";
import { CurrentQuoteWidget } from "@/components/stock-beta/quote/current-quote-widget";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  ownerEquityV2MembershipListSchema,
  ownerEquityV2MembershipSchema,
  ownerEquityV2MembershipStatusSchema,
  ownerEquityV2SignalDetailSchema,
} from "@/lib/products/equity-signals-contracts";

// The production-shaped browser fixture is ESM JavaScript, executed unchanged here.
const fixtureUrl = pathToFileURL(`${process.cwd()}/tests/e2e/support/stock-beta-fixture.mjs`);
const { resetStockBetaFixture, stockBetaMembershipForIntraday, stockBetaResponse } = await import(
  fixtureUrl.href
);
const intradayFixtureUrl = pathToFileURL(
  `${process.cwd()}/tests/e2e/support/stock-beta-intraday-fixture.mjs`,
);
const { resetStockBetaIntradayFixture, stockBetaIntradayResponse } = await import(
  intradayFixtureUrl.href
);
const path = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const advancePath = "/__test/stock-beta/membership-advance";
const intradayDemandPath = "/api/v1/research/owner-beta/equity-universe-v2/quote-demands";
const intradayQuotePath =
  "/api/v1/research/owner-beta/equity-universe-v2/instruments/000001.KRX/quote";
const scenario = { role: "owner", stockBetaSeed: "empty", stockBetaRows: 0 };
const controlledScenario = { ...scenario, stockBetaLifecycle: "controlled" };
const headers = { "x-csrf-token": "synthetic", "idempotency-key": "synthetic" };

type FixtureMembership = {
  readonly generation: number;
  readonly id: string;
  readonly instrument_id: string;
  readonly lifecycle: string;
};

function request(
  method: string,
  pathname: string,
  body: unknown = {},
  requestScenario: typeof scenario = scenario,
) {
  return stockBetaResponse({
    method,
    pathname,
    body,
    scenario: requestScenario,
    headers,
    query: "",
  });
}

function advanceMembership(
  membershipId: string,
  lifecycle: string,
  generation: number,
  requestScenario: typeof scenario = controlledScenario,
) {
  return request(
    "POST",
    advancePath,
    {
      membership_id: membershipId,
      expected_generation: generation,
      expected_lifecycle: lifecycle,
    },
    requestScenario,
  );
}

function membershipStatus(
  membershipId: string,
  requestScenario: typeof scenario = controlledScenario,
) {
  const response = request("GET", `${path}/${membershipId}`, {}, requestScenario);
  expect(response.status).toBe(200);
  return ownerEquityV2MembershipStatusSchema.parse(response.body).membership;
}

describe("Stock Beta browser membership fixture", () => {
  beforeEach(() => resetStockBetaFixture(scenario));

  it("publishes a READY membership with the same generation as its signal", () => {
    expect(request("POST", path, { instrument_code: "005930" }).status).toBe(202);
    for (let index = 0; index < 4; index += 1) request("GET", path);
    const membership = request("GET", path).body.memberships[0];
    const signals = request(
      "GET",
      "/api/v1/research/owner-beta/equity-universe-v2/signals/latest",
    ).body;
    expect(membership.lifecycle).toBe("READY");
    expect(membership.generation).toBeGreaterThan(0);
    expect(signals.rows[0].generation).toBe(membership.generation);
  });

  it("keeps the disabled identity rejected while re-registration gets a fresh UUID", () => {
    resetStockBetaFixture({ role: "owner", stockBetaSeed: "ready", stockBetaRows: 1 });
    resetStockBetaIntradayFixture();
    const initial = request("GET", path).body.memberships[0];
    expect(initial.lifecycle).toBe("READY");
    expect(initial.id).toBe("00000000-0000-4000-8000-000000000601");
    expect(stockBetaMembershipForIntraday(initial.id)).toEqual({
      generation: 1,
      instrument_id: "000001.KRX",
      membership_id: initial.id,
    });
    expect(
      stockBetaIntradayResponse({
        body: {},
        headers: {},
        method: "GET",
        pathname: intradayQuotePath,
        query: `membership_id=${initial.id}&generation=${initial.generation}`,
        scenario: { role: "owner", stockBetaIntradayState: "open" },
      }).status,
    ).toBe(200);

    expect(request("POST", `${path}/${initial.id}/disable`).status).toBe(202);
    expect(stockBetaMembershipForIntraday(initial.id)).toBeNull();
    const oldDemand = stockBetaIntradayResponse({
      body: {
        consumer_id: "00000000-0000-4000-8000-000000000703",
        generation: initial.generation,
        membership_id: initial.id,
        renewal_sequence: 0,
        schema_version: 1,
      },
      headers,
      method: "POST",
      pathname: intradayDemandPath,
      query: "",
      scenario: { role: "owner", stockBetaIntradayState: "open" },
    });
    expect(oldDemand.status).toBe(404);

    expect(request("POST", path, { instrument_code: "000001" }).status).toBe(202);
    for (let index = 0; index < 4; index += 1) request("GET", path);
    const memberships = request("GET", path).body.memberships as readonly FixtureMembership[];
    const replacement = memberships.find(
      (membership) => membership.instrument_id === "000001.KRX" && membership.id !== initial.id,
    );
    const disabled = memberships.find((membership) => membership.id === initial.id);
    if (replacement === undefined || disabled === undefined) {
      throw new Error("replacement membership was not READY");
    }
    expect(disabled.lifecycle).toBe("DISABLED");
    expect(replacement.lifecycle).toBe("READY");
    expect(replacement.id).not.toBe(initial.id);
    expect(replacement.generation).toBe(initial.generation);
    expect(stockBetaMembershipForIntraday(replacement.id)).toEqual({
      generation: replacement.generation,
      instrument_id: replacement.instrument_id,
      membership_id: replacement.id,
    });

    const newDemand = stockBetaIntradayResponse({
      body: {
        consumer_id: "00000000-0000-4000-8000-000000000704",
        generation: replacement.generation,
        membership_id: replacement.id,
        renewal_sequence: 0,
        schema_version: 1,
      },
      headers,
      method: "POST",
      pathname: intradayDemandPath,
      query: "",
      scenario: { role: "owner", stockBetaIntradayState: "open" },
    });
    expect(newDemand.status).toBe(200);
    expect(newDemand.body.instrument_id).toBe("000001.KRX");
    const oldQuote = stockBetaIntradayResponse({
      body: {},
      headers: {},
      method: "GET",
      pathname: intradayQuotePath,
      query: `membership_id=${initial.id}&generation=${initial.generation}`,
      scenario: { role: "owner", stockBetaIntradayState: "open" },
    });
    expect(oldQuote.status).toBe(404);
  });

  it("holds controlled lifecycle stages across list and status reads", () => {
    resetStockBetaFixture(controlledScenario);
    const admission = request("POST", path, { instrument_code: "005930" }, controlledScenario);
    expect(admission.status).toBe(202);
    const requested = ownerEquityV2MembershipSchema.parse(admission.body.resource);
    expect(requested.lifecycle).toBe("REQUESTED");
    expect(requested.generation).toBe(0);

    const stages = [
      { expected: "REQUESTED", lifecycle: "VALIDATING", observed: 20, generation: 0 },
      { expected: "VALIDATING", lifecycle: "BACKFILLING", observed: 80, generation: 0 },
      { expected: "BACKFILLING", lifecycle: "MATERIALIZING", observed: 200, generation: 0 },
      { expected: "MATERIALIZING", lifecycle: "READY", observed: 261, generation: 1 },
    ] as const;
    let current = requested;

    for (const stage of stages) {
      for (let read = 0; read < 3; read += 1) {
        const list = ownerEquityV2MembershipListSchema.parse(
          request("GET", path, {}, controlledScenario).body,
        );
        const status = membershipStatus(requested.id);
        expect(list.memberships.find((item) => item.id === requested.id)).toEqual(current);
        expect(status).toEqual(current);
      }

      expect(current.lifecycle).toBe(stage.expected);
      const response = advanceMembership(requested.id, current.lifecycle, current.generation);
      expect(response.status).toBe(200);
      const advanced = ownerEquityV2MembershipStatusSchema.parse(response.body).membership;
      expect(advanced.id).toBe(requested.id);
      expect(advanced.instrument_id).toBe(requested.instrument_id);
      expect(advanced.requested_at).toBe(requested.requested_at);
      expect(advanced.lifecycle).toBe(stage.lifecycle);
      expect(advanced.generation).toBe(stage.generation);
      expect(advanced.coverage).toMatchObject({
        minimum_observed_sessions: 121,
        observed_sessions: stage.observed,
        target_observed_sessions: 261,
      });
      current = advanced;
    }
    for (let read = 0; read < 3; read += 1) {
      const list = ownerEquityV2MembershipListSchema.parse(
        request("GET", path, {}, controlledScenario).body,
      );
      expect(list.memberships.find((item) => item.id === requested.id)).toEqual(current);
      expect(membershipStatus(requested.id)).toEqual(current);
    }
  });

  it("advances only the selected identity after same-instrument re-registration", () => {
    resetStockBetaFixture(controlledScenario);
    const firstResponse = request("POST", path, { instrument_code: "005930" }, controlledScenario);
    const first = ownerEquityV2MembershipSchema.parse(firstResponse.body.resource);
    let firstCurrent = first;
    for (const lifecycle of ["VALIDATING", "BACKFILLING", "MATERIALIZING", "READY"]) {
      const response = advanceMembership(first.id, firstCurrent.lifecycle, firstCurrent.generation);
      expect(response.status).toBe(200);
      firstCurrent = ownerEquityV2MembershipStatusSchema.parse(response.body).membership;
      expect(firstCurrent.lifecycle).toBe(lifecycle);
    }

    expect(request("POST", `${path}/${first.id}/disable`).status).toBe(202);
    const disabled = membershipStatus(first.id);
    expect(disabled.lifecycle).toBe("DISABLED");
    const disabledBefore = structuredClone(disabled);
    const disabledControl = advanceMembership(first.id, "DISABLED", disabled.generation);
    expect(disabledControl.status).toBe(409);
    expect(membershipStatus(first.id)).toEqual(disabledBefore);
    expect(stockBetaMembershipForIntraday(first.id)).toBeNull();

    const replacementResponse = request(
      "POST",
      path,
      { instrument_code: "005930" },
      controlledScenario,
    );
    expect(replacementResponse.status).toBe(202);
    expect(replacementResponse.body.duplicate_active).toBe(false);
    let replacement = ownerEquityV2MembershipSchema.parse(replacementResponse.body.resource);
    expect(replacement.id).not.toBe(first.id);
    expect(replacement.instrument_id).toBe(first.instrument_id);
    expect(replacement.generation).toBe(0);
    expect(membershipStatus(first.id)).toEqual(disabledBefore);

    for (const lifecycle of ["VALIDATING", "BACKFILLING", "MATERIALIZING", "READY"]) {
      const response = advanceMembership(
        replacement.id,
        replacement.lifecycle,
        replacement.generation,
      );
      expect(response.status).toBe(200);
      replacement = ownerEquityV2MembershipStatusSchema.parse(response.body).membership;
      expect(replacement.lifecycle).toBe(lifecycle);
      expect(membershipStatus(first.id)).toEqual(disabledBefore);
    }
    expect(replacement.generation).toBe(1);
    expect(stockBetaMembershipForIntraday(first.id)).toBeNull();
    expect(stockBetaMembershipForIntraday(replacement.id)).toEqual({
      generation: 1,
      instrument_id: first.instrument_id,
      membership_id: replacement.id,
    });
  });

  it("rejects invalid and terminal controlled advances without changing membership state", () => {
    resetStockBetaFixture(controlledScenario);
    const admission = request("POST", path, { instrument_code: "005930" }, controlledScenario);
    const requested = ownerEquityV2MembershipSchema.parse(admission.body.resource);
    const before = structuredClone(membershipStatus(requested.id));
    const validBody = {
      membership_id: requested.id,
      expected_generation: requested.generation,
      expected_lifecycle: requested.lifecycle,
    };
    const invalidBodies: unknown[] = [
      null,
      [],
      {},
      { ...validBody, extra: true },
      { ...validBody, membership_id: 7 },
      { ...validBody, expected_generation: -1 },
      { ...validBody, expected_generation: Number.MAX_SAFE_INTEGER + 1 },
      { ...validBody, expected_lifecycle: "NOT_A_LIFECYCLE" },
    ];
    for (const body of invalidBodies) {
      const response = request("POST", advancePath, body, controlledScenario);
      expect(response.status).toBe(422);
      expect(response.body.error.code).toBe("INVALID_PARAMETER");
      expect(membershipStatus(requested.id)).toEqual(before);
    }

    const nonOwner = request("POST", advancePath, validBody, {
      ...controlledScenario,
      role: "member",
    });
    expect(nonOwner.status).toBe(403);
    expect(membershipStatus(requested.id)).toEqual(before);

    const unknown = advanceMembership("00000000-0000-4000-8000-000000000999", "REQUESTED", 0);
    expect(unknown.status).toBe(404);
    expect(membershipStatus(requested.id)).toEqual(before);
    for (const staleBody of [
      { ...validBody, expected_generation: 1 },
      { ...validBody, expected_lifecycle: "VALIDATING" },
      { ...validBody, expected_lifecycle: "BACKFILLING" },
    ]) {
      const stale = request("POST", advancePath, staleBody, controlledScenario);
      expect(stale.status).toBe(409);
      expect(membershipStatus(requested.id)).toEqual(before);
    }

    resetStockBetaFixture({ ...controlledScenario, stockBetaSeed: "failed", stockBetaRows: 1 });
    const failed = ownerEquityV2MembershipListSchema.parse(
      request("GET", path, {}, controlledScenario).body,
    ).memberships[0];
    if (failed === undefined) throw new Error("failed seed missing");
    const failedBefore = structuredClone(failed);
    expect(advanceMembership(failed.id, "FAILED", failed.generation).status).toBe(409);
    expect(membershipStatus(failed.id)).toEqual(failedBefore);

    resetStockBetaFixture({ ...controlledScenario, stockBetaSeed: "ready", stockBetaRows: 1 });
    const ready = ownerEquityV2MembershipListSchema.parse(
      request("GET", path, {}, controlledScenario).body,
    ).memberships[0];
    if (ready === undefined) throw new Error("ready seed missing");
    const readyBefore = structuredClone(ready);
    expect(advanceMembership(ready.id, "READY", ready.generation).status).toBe(409);
    expect(membershipStatus(ready.id)).toEqual(readyBefore);
  });

  it("enables read-driven advancement again after reset without controlled mode", () => {
    resetStockBetaFixture(controlledScenario);
    const held = request("POST", path, { instrument_code: "005930" }, controlledScenario);
    const requested = ownerEquityV2MembershipSchema.parse(held.body.resource);
    const controlled = advanceMembership(requested.id, "REQUESTED", 0);
    expect(controlled.status).toBe(200);
    expect(membershipStatus(requested.id).lifecycle).toBe("VALIDATING");

    resetStockBetaFixture({ role: "owner", stockBetaSeed: "empty", stockBetaRows: 0 });
    const admission = request("POST", path, { instrument_code: "005930" });
    expect(admission.status).toBe(202);
    const resetRequest = ownerEquityV2MembershipSchema.parse(admission.body.resource);
    expect(advanceMembership(resetRequest.id, "REQUESTED", 0, scenario)).toBeNull();
    const list = ownerEquityV2MembershipListSchema.parse(request("GET", path).body);
    expect(list.memberships.find((item) => item.id === resetRequest.id)?.lifecycle).toBe(
      "VALIDATING",
    );
  });

  it.each(["en", "ko"] as const)("passes only cloneable detail quote props for %s", (locale) => {
    resetStockBetaFixture({ stockBetaRows: 1, stockBetaSeed: "ready" });
    const membership = ownerEquityV2MembershipSchema.parse(
      request("GET", path).body.memberships[0],
    );
    const detail = ownerEquityV2SignalDetailSchema.parse(
      request(
        "GET",
        "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/000001.KRX",
      ).body,
    );
    const copy = stockBetaDictionary[locale];
    expect(() => structuredClone(copy)).toThrow();
    const element = CurrentQuoteWidget({
      placementVisibility: { desktop: true, tablet: false, mobile: true },
      viewModel: {
        backHref: "/stock-beta",
        copy,
        detail,
        intradayEnabled: true,
        intradayMembership: membership,
        locale,
      },
    });
    expect(element.type).toBe(CurrentQuoteClient);
    expect(structuredClone(element.props)).toEqual(element.props);
    expect(element.props).not.toHaveProperty("viewModel");
    expect(element.props.copy).not.toHaveProperty("requestFailure");
    expect(element.props.copy.intradayQuoteHeading).toBe(copy.intradayQuoteHeading);
    expect(element.props.identity).toEqual({
      generation: membership.generation,
      instrument_id: membership.instrument_id,
      membership_id: membership.id,
    });
    expect(element.props.placementVisibility).toEqual({
      desktop: true,
      tablet: false,
      mobile: true,
    });
  });
});
