import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AppShell } from "@/components/shell/app-shell";
import { StatePanel } from "@/components/states/state-panel";
import { StockBetaInstrumentSearch } from "@/components/stock-beta/dashboard/instrument-search";
import { StockBetaSelectionProvider } from "@/components/stock-beta/dashboard/selection-provider";
import { StockBetaTerminalShell } from "@/components/stock-beta/terminal/stock-beta-terminal-shell";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { ownerEquityV2SignalSchema } from "@/lib/products/equity-signals-contracts";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
  usePathname: () => "/",
}));

const OWNER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000001",
  role: "owner",
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "disabled",
  owner_beta_paper_mode: "disabled",
} as const;

describe("application shell accessibility", () => {
  it("provides named landmarks and a labeled logout form", () => {
    // Given
    const shell = (
      <AppShell session={OWNER_SESSION}>
        <h1>Dashboard</h1>
      </AppShell>
    );

    // When
    const markup = renderToStaticMarkup(shell);

    // Then
    expect(markup).toContain("<header");
    expect(markup).toContain('<nav aria-label="Primary"');
    expect(markup).toContain("<main");
    expect(markup).toContain('<form aria-label="Sign out"');
    expect(markup).toContain("<h1>Dashboard</h1>");
  });

  it("announces actionable error content", () => {
    // Given
    const errorState = (
      <StatePanel
        action={<button type="button">Try again</button>}
        kind="error"
        message="The authenticated request could not be completed."
        title="We could not load this view"
      />
    );

    // When
    const markup = renderToStaticMarkup(errorState);
    const headingId = markup.match(/aria-labelledby="([^"]+)"/)?.[1] ?? "";

    // Then
    expect(markup).toContain('role="alert"');
    expect(headingId).not.toBe("");
    expect(markup).toContain(`<h2 id="${headingId}">We could not load this view</h2>`);
    expect(markup).toContain("We could not load this view");
    expect(markup).toContain("Try again");
  });

  it("keeps the Stock Beta terminal landmarks named without a non-functional theme control", () => {
    // Given
    const shell = (
      <StockBetaTerminalShell
        languageLabel="Change language"
        navigation={[
          { href: "/stock-beta", icon: <span aria-hidden={true}>S</span>, label: "Stock beta" },
        ]}
        productLabel="Market research"
        readOnlyLabel="Read only"
        roleLabel="Owner"
        skipToMainLabel="Skip to main content"
      >
        <h1>Stock signal beta</h1>
      </StockBetaTerminalShell>
    );

    // When
    const markup = renderToStaticMarkup(shell);

    // Then
    expect(markup).toContain('data-shell="stock-beta-terminal"');
    expect(markup).toContain('<nav aria-label="Primary"');
    expect(markup).toContain('href="#main-content"');
    expect(markup.match(/<main/g)).toHaveLength(1);
    expect(markup.match(/<h1/g)).toHaveLength(1);
    expect(markup).toContain("<main class=");
    expect(markup).not.toContain("Switch to dark theme");
    expect(markup).not.toContain("Switch to light theme");
  });

  it("gives the V2 instrument search a label, match description, and keyboard affordance", () => {
    const row = ownerEquityV2SignalSchema.parse({
      average_trading_value_20: 1_000_000,
      average_volume_20: 25_000,
      condition: "BULLISH",
      generation: 7,
      instrument_id: "000001.KRX",
      max_drawdown_120: -0.2,
      rank: 1,
      return_120: 0.4,
      return_20: 0.1,
      return_60: 0.2,
      score: 1.2,
      sma_20: 100,
      sma_60: 99,
      volatility_120: 0.3,
      volatility_20: 0.1,
      volatility_60: 0.2,
      volume_ratio_20_60: 1.1,
    });
    const markup = renderToStaticMarkup(
      <StockBetaSelectionProvider initialSelectedInstrumentId={row.instrument_id} rows={[row]}>
        <StockBetaInstrumentSearch copy={stockBetaDictionary.en} rows={[row]} />
      </StockBetaSelectionProvider>,
    );

    expect(markup).toContain('data-testid="stock-beta-instrument-search"');
    expect(markup).toContain(
      '<label for="stock-beta-instrument-search-input"><span>Search signals</span>',
    );
    expect(markup).toContain('type="search"');
    expect(markup).toContain('aria-describedby="stock-beta-search-hint"');
    expect(markup).toContain("<kbd>/</kbd>");
    expect(markup).toContain('id="stock-beta-search-hint"');
    expect(markup).toContain('list="stock-beta-instrument-options"');
    expect(markup).toContain('<option value="000001.KRX"></option>');
  });
});
