import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  IntradayStreamBoard,
  StreamSelectedQuote,
} from "@/components/stock-beta/quote/intraday-stream-board";
import type { IntradayStreamControllerView } from "@/components/stock-beta/quote/intraday-stream-controller";
import {
  MARKET_STREAM_INSTRUMENTS,
  readyStreamIdentities,
} from "@/lib/products/intraday-stream-universe";
import { streamMembership, streamRow } from "./fixtures/market-stream";

const mock = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/components/stock-beta/quote/intraday-stream-provider", () => ({
  usePageIntradayStream: mock.read,
}));
const identity = readyStreamIdentities([streamMembership(0)])[0];
if (!identity) throw new Error("Missing test identity");
let view: IntradayStreamControllerView;

beforeEach(() => {
  view = {
    lifecycle: "active",
    failure: null,
    phase: "ready",
    terminalReason: null,
    rows: [streamRow()],
  };
  mock.read.mockImplementation(() => ({ view, identities: [identity] }));
});

const board = () =>
  renderToStaticMarkup(
    <IntradayStreamBoard
      locale="en"
      selectedInstrumentId={identity.instrument_id}
      onSelect={() => undefined}
    />,
  );
const selected = () =>
  renderToStaticMarkup(<StreamSelectedQuote locale="en" identity={identity} />);

describe("shared stream presentation", () => {
  it("renders exactly 30 instrument rows, preserving precise prices and observation times", () => {
    const markup = board();
    expect(markup.match(/data-stream-instrument=/g)).toHaveLength(30);
    for (const instrument of MARKET_STREAM_INSTRUMENTS) expect(markup).toContain(instrument.id);
    expect(markup).toContain("100,123,456,789.12345678");
    expect(markup).toContain("-1.25%");
    expect(markup).toContain("Awaiting admission");
    expect(markup).toContain("2026-10-03T03:00:00.100Z");
    expect(markup).toContain("2026-10-03T12:00:00+09:00");
    expect(markup).toContain('aria-pressed="true"');
    const detail = selected();
    expect(detail).toContain('data-quote-value="100123456789.12345678"');
    expect(detail).toContain("Not provided by this channel");
    expect(detail).toContain("100 / 1,234,567");
  });
  it("shows halt observations beside stale, closed and gap state", () => {
    const row = streamRow();
    view.rows = [{ ...row, freshness: "STALE", availability: "LAST_KNOWN", session_has_gap: true }];
    expect(board()).toContain("Stale");
    expect(selected()).toContain("Gap in this session");
    view.rows = [{ ...row, market_state: "CLOSED", availability: "LAST_KNOWN" }];
    expect(selected()).toContain("Market closed");
    expect(board()).not.toContain(">Live<");
    if (!row.quote) throw new Error("Missing quote fixture");
    view.rows = [
      {
        ...row,
        market_state: "CLOSED",
        availability: "LAST_KNOWN",
        quote: { ...row.quote, halted: true },
      },
    ];
    expect(board()).toContain("Market closed");
    expect(board()).toContain("Halt observed");
    expect(selected()).toContain("Market closed");
    expect(selected()).toContain("Halt observed");
    view.rows = [
      {
        ...row,
        freshness: "STALE",
        availability: "LAST_KNOWN",
        quote: { ...row.quote, halted: true },
      },
    ];
    expect(board()).toContain("Stale");
    expect(board()).toContain("Halt observed");
    expect(selected()).toContain("Stale");
    expect(selected()).toContain("Halt observed");
    view.rows = [{ ...row, quote: { ...row.quote, halted: false } }];
    expect(board()).not.toContain("Halt observed");
    view.rows = [{ ...row, quote: { ...row.quote, halted: true } }];
    const liveWithHalt = board();
    expect(liveWithHalt).toContain(">Live<");
    expect(liveWithHalt.match(/Halt observed/g)).toHaveLength(1);
    expect(selected().match(/Halt observed/g)).toHaveLength(1);
  });
  it("never reuses another membership generation's price for the selected instrument", () => {
    const markup = renderToStaticMarkup(
      <StreamSelectedQuote
        locale="ko"
        identity={{ ...identity, generation: identity.generation + 1 }}
      />,
    );
    expect(markup).not.toContain("100123456789");
    expect(markup).toContain("첫 체결 대기");
    const pending = renderToStaticMarkup(
      <StreamSelectedQuote locale="en" identity={null} instrumentId="005930.KRX" />,
    );
    expect(pending).toContain("005930.KRX");
    expect(pending).not.toContain("100123456789");
  });
});
