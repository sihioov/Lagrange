import { pathToFileURL } from "node:url";
import { beforeEach, describe, expect, it } from "vitest";
import { CurrentQuoteClient } from "@/components/stock-beta/quote/current-quote-client";
import { CurrentQuoteWidget } from "@/components/stock-beta/quote/current-quote-widget";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  ownerEquityV2MembershipSchema,
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
const intradayDemandPath = "/api/v1/research/owner-beta/equity-universe-v2/quote-demands";
const intradayQuotePath =
  "/api/v1/research/owner-beta/equity-universe-v2/instruments/000001.KRX/quote";
const scenario = { role: "owner", stockBetaSeed: "empty", stockBetaRows: 0 };
const headers = { "x-csrf-token": "synthetic", "idempotency-key": "synthetic" };

type FixtureMembership = {
  readonly generation: number;
  readonly id: string;
  readonly instrument_id: string;
  readonly lifecycle: string;
};

function request(method: string, pathname: string, body = {}) {
  return stockBetaResponse({ method, pathname, body, scenario, headers, query: "" });
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
