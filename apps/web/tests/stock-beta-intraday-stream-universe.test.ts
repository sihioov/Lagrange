import { describe, expect, it } from "vitest";
import {
  MARKET_STREAM_INSTRUMENTS,
  readyStreamIdentities,
} from "@/lib/products/intraday-stream-universe";
import { streamMembership } from "./fixtures/market-stream";

describe("stream observation universe and admission", () => {
  it("uses all 30 checked-in symbols but demands only their READY memberships", () => {
    expect(MARKET_STREAM_INSTRUMENTS).toHaveLength(30);
    const first = streamMembership(0);
    const second = streamMembership(1);
    expect(
      readyStreamIdentities([
        { ...first, instrument_id: "999999.KRX" },
        { ...second, lifecycle: "DISABLED" },
        first,
      ]),
    ).toEqual([
      { membership_id: first.id, instrument_id: first.instrument_id, generation: first.generation },
    ]);
    const members = Array.from({ length: 30 }, (_, i) => streamMembership(i));
    expect(readyStreamIdentities([...members].reverse())).toEqual(readyStreamIdentities(members));
  });
  it("does not invent an identity when none is admitted", () => {
    expect(readyStreamIdentities([])).toEqual([]);
    expect(
      readyStreamIdentities([{ ...streamMembership(0), lifecycle: "REQUESTED", generation: 0 }]),
    ).toEqual([]);
  });
  it("fails closed on duplicate admitted identities and unsafe generations", () => {
    const first = streamMembership(0);
    expect(() => readyStreamIdentities([first, first])).toThrow();
    expect(() =>
      readyStreamIdentities([
        first,
        { ...streamMembership(1), instrument_id: first.instrument_id },
      ]),
    ).toThrow();
    for (const generation of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1])
      expect(() => readyStreamIdentities([{ ...first, generation }])).toThrow();
  });
});
