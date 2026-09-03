import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const NAVIGATION_ROUTE_FILES = [
  "app/(authenticated)/page.tsx",
  "app/(authenticated)/strategies/page.tsx",
  "app/(authenticated)/recommendations/page.tsx",
  "app/(authenticated)/backtests/page.tsx",
  "app/(authenticated)/paper/page.tsx",
  "app/(authenticated)/admin/page.tsx",
  "app/(authenticated)/live/page.tsx",
] as const;

function source(relativePath: string): string {
  const path = resolve(process.cwd(), relativePath);
  return existsSync(path) ? readFileSync(path, "utf8") : "";
}

function cssToken(styles: string, name: string): string {
  const value = styles.match(new RegExp(`${name}:\\s*(#[0-9a-f]{6})`, "i"))?.[1];
  if (value === undefined) {
    throw new Error(`Missing CSS token ${name}`);
  }
  return value;
}

function relativeLuminance(hex: string): number {
  const channel = (offset: number) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrastRatio(foreground: string, background: string): number {
  const lighter = Math.max(relativeLuminance(foreground), relativeLuminance(background));
  const darker = Math.min(relativeLuminance(foreground), relativeLuminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

describe("authenticated shell runtime", () => {
  it("loads tokenized styles with bounded responsive scroll mechanics", () => {
    // Given
    const rootLayout = source("app/layout.tsx");
    const styles = source("app/globals.css");

    // When
    const styleContract = {
      focusVisible: styles.includes(":focus-visible"),
      globalImport: rootLayout.includes('import "./globals.css"'),
      reducedMotion: styles.includes("prefers-reduced-motion"),
      scrollChildCanShrink: styles.includes("min-block-size: 0"),
      surfaceToken: styles.includes("--surface-canvas"),
      usesDynamicViewport: styles.includes("100dvb"),
    };

    // Then
    expect(styleContract).toEqual({
      focusVisible: true,
      globalImport: true,
      reducedMotion: true,
      scrollChildCanShrink: true,
      surfaceToken: true,
      usesDynamicViewport: true,
    });
  });

  it("keeps trust-critical light-theme text at WCAG AA contrast", () => {
    // Given
    const styles = source("app/globals.css");
    const tertiary = cssToken(styles, "--text-tertiary");
    const error = cssToken(styles, "--status-error");

    // When
    const ratios = [
      contrastRatio(tertiary, cssToken(styles, "--surface-canvas")),
      contrastRatio(tertiary, cssToken(styles, "--surface-panel")),
      contrastRatio(tertiary, cssToken(styles, "--surface-muted")),
      contrastRatio(error, cssToken(styles, "--surface-muted")),
    ];

    // Then
    expect(ratios.every((ratio) => ratio >= 4.5)).toBe(true);
  });

  it("uses Pretendard for interface copy and Geist Mono only through the data token", () => {
    const rootLayout = source("app/layout.tsx");
    const globalStyles = source("app/globals.css");
    const productStyles = source("app/product.css");
    const dashboardStyles = source("components/stock-beta/dashboard/dashboard.module.css");
    const detailStyles = source("components/stock-beta/detail/detail.module.css");

    expect(rootLayout).toContain(
      'import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css"',
    );
    expect(globalStyles).toContain('"Pretendard Variable", Pretendard');
    expect(globalStyles).toContain("--font-display: var(--font-body)");
    expect(globalStyles).toContain("--font-data: var(--font-geist-mono)");
    expect(globalStyles).toMatch(/body\s*\{[^}]*font-family:\s*var\(--font-body\)/s);
    expect(productStyles).not.toContain("var(--font-geist-mono)");
    expect(dashboardStyles).not.toContain("var(--font-geist-mono)");
    expect(detailStyles).not.toContain("var(--font-geist-mono)");
  });

  it("keeps the skip link hidden until focus when motion is reduced", () => {
    // Given
    const styles = source("app/globals.css");

    // When
    const reducedMotion = styles.slice(
      styles.indexOf("@media (prefers-reduced-motion: reduce)"),
      styles.indexOf("@media (forced-colors: active)"),
    );

    // Then
    expect(reducedMotion).toMatch(
      /\.skip-link\s*\{[^}]*transition:\s*none;[^}]*transform:\s*translateY\(-200%\);/s,
    );
  });

  it("removes browser-default fieldset chrome from workflow forms", () => {
    // Given
    const styles = source("app/product.css");

    // When
    const fieldsetReset =
      /\.config-form fieldset,\s*\.workflow-form fieldset\s*\{[^}]*border:\s*0;/s;
    const legendReset = /\.config-form legend,\s*\.workflow-form legend\s*\{/s;

    // Then
    expect(styles).toMatch(fieldsetReset);
    expect(styles).toMatch(legendReset);
  });

  it("contains long provenance identifiers inside their grid cells", () => {
    // Given
    const styles = source("app/product.css");

    // When
    const provenanceValues = /\.provenance-grid dd\s*\{[^}]*overflow-wrap:\s*anywhere;/s;

    // Then
    expect(styles).toMatch(provenanceValues);
  });

  it("provides a server page for every visible navigation destination", () => {
    // Given
    const routeFiles = NAVIGATION_ROUTE_FILES;

    // When
    const missingRoutes = routeFiles.filter((route) => !existsSync(resolve(process.cwd(), route)));

    // Then
    expect(missingRoutes).toEqual([]);
  });

  it("provides loading and error boundaries through the shared state panel", () => {
    // Given
    const loading = source("app/(authenticated)/loading.tsx");
    const error = source("app/(authenticated)/error.tsx");

    // When
    const boundaryContract = {
      errorPanel: error.includes("<StatePanel") && error.includes('kind="error"'),
      loadingPanel: loading.includes("<StatePanel") && loading.includes('kind="loading"'),
    };

    // Then
    expect(boundaryContract).toEqual({ errorPanel: true, loadingPanel: true });
  });

  it("uses one terminal shell across authenticated routes without global CSS hiding", () => {
    const appShell = source("components/shell/app-shell.tsx");
    const routeShell = source("components/stock-beta/terminal/route-aware-shell.tsx");
    const stockBetaDetail = source("components/stock-beta/stock-beta-detail.tsx");
    const stockBetaTheme = source("components/stock-beta/stock-beta-theme.module.css");
    const stockBetaWorkspace = source("components/stock-beta/stock-beta-workspace.tsx");
    const terminalPage = source("components/stock-beta/terminal/terminal-page.tsx");
    const utilitySlot = source("components/shell/terminal-utility-slot.tsx");

    expect(appShell).toContain("<RouteAwareShell");
    expect(routeShell).toContain("<ResearchTerminalShell");
    expect(routeShell).toContain('"research-terminal"');
    expect(routeShell).not.toContain('data-shell="general"');
    expect(stockBetaTheme).not.toContain(":has(");
    expect(stockBetaTheme).not.toContain(":global(.app-shell)");
    expect(terminalPage).toContain("<StockBetaTerminalUtilitySlot>");
    expect(terminalPage).not.toContain("pageUtility");
    expect(utilitySlot).toContain("createPortal(children, host)");
    expect(utilitySlot).not.toContain("querySelector");
    expect(stockBetaWorkspace.indexOf("<StockBetaSelectionProvider")).toBeLessThan(
      stockBetaWorkspace.indexOf("<StockBetaTerminalPage"),
    );
    expect(stockBetaWorkspace).toContain("search={<StockBetaInstrumentSearch");
    expect(stockBetaDetail).toContain("<StockBetaTerminalPage");
    expect(stockBetaDetail).toContain("asOf={");
  });
});
