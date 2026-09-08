import { describe, expect, it, vi } from "vitest";

vi.mock("server-only", () => ({}));

import { isStockBetaIntradayQuotesEnabled } from "@/components/stock-beta/quote/intraday-quotes-mode";
import { matchReadyIntradayQuoteMembership } from "@/components/stock-beta/quote/membership";
import { ownerEquityV2MembershipSchema } from "@/lib/products/equity-signals-contracts";

function membership(lifecycle: "READY" | "DISABLED", instrumentId = "069500.KRX") {
  return ownerEquityV2MembershipSchema.parse({
    coverage: {
      first_session: "2025-01-01",
      last_session: "2026-09-07",
      minimum_observed_sessions: 121,
      observed_sessions: 261,
      target_observed_sessions: 261,
    },
    disabled_at: lifecycle === "DISABLED" ? "2026-09-07T06:00:00Z" : undefined,
    generation: 7,
    id: "00000000-0000-4000-8000-000000000001",
    instrument_id: instrumentId,
    lifecycle,
    requested_at: "2026-09-01T06:00:00Z",
    updated_at: "2026-09-07T06:00:00Z",
  });
}

describe("Stock Beta intraday server flag and membership seam", () => {
  it("enables only the exact owner_only server value", () => {
    for (const value of [undefined, "", "off", "owner-only", "OWNER_ONLY"]) {
      if (value === undefined) vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", undefined);
      else vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", value);
      expect(isStockBetaIntradayQuotesEnabled()).toBe(false);
    }
    vi.stubEnv("OWNER_INTRADAY_QUOTES_MODE", "owner_only");
    expect(isStockBetaIntradayQuotesEnabled()).toBe(true);
  });

  it("requires READY plus exact instrument and generation, so disabled/replaced records do not demand quotes", () => {
    const ready = membership("READY");
    const disabled = membership("DISABLED");
    expect(
      matchReadyIntradayQuoteMembership([disabled], {
        instrument_id: ready.instrument_id,
        generation: ready.generation,
      }),
    ).toBeNull();
    expect(
      matchReadyIntradayQuoteMembership([ready], {
        instrument_id: ready.instrument_id,
        generation: ready.generation + 1,
      }),
    ).toBeNull();
    expect(
      matchReadyIntradayQuoteMembership([ready], {
        instrument_id: ready.instrument_id,
        generation: ready.generation,
      }),
    ).toEqual(ready);
  });
});
