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
});
