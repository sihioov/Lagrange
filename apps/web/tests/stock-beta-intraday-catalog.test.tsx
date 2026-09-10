import { pathToFileURL } from "node:url";
import { Children, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { StockBetaSelectionProvider } from "@/components/stock-beta/dashboard/selection-provider";
import { renderStockBetaDashboardGrid } from "@/components/stock-beta/dashboard/stock-beta-dashboard";
import type { StockBetaDashboardViewModel } from "@/components/stock-beta/dashboard/types";
import { stockBetaDashboardCatalog } from "@/components/stock-beta/dashboard/widget-registry";
import { renderStockBetaDetailGrid } from "@/components/stock-beta/detail/stock-beta-detail-layout";
import type { StockBetaDetailViewModel } from "@/components/stock-beta/detail/types";
import { stockBetaDetailCatalog } from "@/components/stock-beta/detail/widget-registry";
import { CurrentQuoteWidget } from "@/components/stock-beta/quote/current-quote-widget";
import { defineStockBetaWidgetArchitecture } from "@/components/stock-beta/shared/widget-types";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import {
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2SignalDetailSchema,
} from "@/lib/products/equity-signals-contracts";

const fixtureUrl = pathToFileURL(`${process.cwd()}/tests/e2e/support/stock-beta-fixture.mjs`);
const { resetStockBetaFixture, stockBetaResponse } = await import(fixtureUrl.href);
const scenario = { role: "owner", stockBetaSeed: "ready", stockBetaRows: 1 };
const prefix = "/api/v1/research/owner-beta/equity-universe-v2";

function body(path: string) {
  return stockBetaResponse({
    method: "GET",
    pathname: `${prefix}/${path}`,
    query: "",
    headers: {},
    scenario,
  }).body;
}

function models(): { dashboard: StockBetaDashboardViewModel; detail: StockBetaDetailViewModel } {
  const list = ownerEquityV2MembershipListSchema.parse(body("memberships"));
  const signals = ownerEquityV2LatestSignalsSchema.parse(body("signals/latest"));
  const detail = ownerEquityV2SignalDetailSchema.parse(body("signals/instruments/000001.KRX"));
  return {
    dashboard: {
      actionError: null,
      actionMessage: null,
      busy: false,
      copy: stockBetaDictionary.en,
      disableId: null,
      inputError: null,
      instrumentCode: "",
      locale: "en",
      memberships: list.memberships,
      mutationPending: false,
      onAdd: async () => undefined,
      onCancelDisable: () => undefined,
      onConfirmDisable: async () => undefined,
      onInstrumentCodeChange: () => undefined,
      onRequestDisable: () => undefined,
      onRetry: async () => undefined,
      pendingMembershipId: null,
      policy: list.policy,
      pollError: false,
      signalState: { kind: "ready" },
      signals,
      intradayEnabled: true,
      selectedInstrumentId: "000001.KRX",
    },
    detail: {
      backHref: "/stock-beta",
      copy: stockBetaDictionary.en,
      detail,
      locale: "en",
      intradayEnabled: true,
      intradayMembership: list.memberships[0] ?? null,
    },
  };
}

function ids(markup: string): string[] {
  return [...markup.matchAll(/data-widget-id="([^"]+)"/g)].map((match) => match[1] ?? "");
}

function quoteTag(markup: string): string {
  return markup.match(/<div[^>]*data-widget-id="current-quote"[^>]*>/)?.[0] ?? "";
}

function quotePlacement(node: ReactNode): unknown {
  for (const child of Children.toArray(node)) {
    if (!isValidElement<{ children?: ReactNode; "data-widget-id"?: string }>(child)) continue;
    if (child.props["data-widget-id"] === "current-quote") {
      const widget = child.props.children;
      return isValidElement<{ placementVisibility?: unknown }>(widget)
        ? widget.props.placementVisibility
        : undefined;
    }
    const found = quotePlacement(child.props.children);
    if (found !== null) return found;
  }
  return null;
}

describe("real current-quote catalog composition (no customization UI)", () => {
  beforeEach(() => resetStockBetaFixture(scenario));

  it("inserts, removes and reorders the real dashboard entry without changing other widgets", () => {
    const viewModel = models().dashboard;
    const entry = stockBetaDashboardCatalog.find((item) => item.id === "current-quote");
    if (entry === undefined) throw new Error("missing current-quote catalog entry");
    expect(entry.component).toBe(CurrentQuoteWidget);
    const remaining = stockBetaDashboardCatalog.filter((item) => item.id !== "current-quote");
    const render = (catalog: readonly (typeof stockBetaDashboardCatalog)[number][]) =>
      renderToStaticMarkup(
        <StockBetaSelectionProvider rows={viewModel.signals?.rows ?? []}>
          {renderStockBetaDashboardGrid(defineStockBetaWidgetArchitecture(catalog), viewModel)}
        </StockBetaSelectionProvider>,
      );
    const absent = render(remaining);
    const inserted = render([...remaining, entry]);
    const reordered = render([entry, ...remaining]);
    expect(ids(absent)).not.toContain("current-quote");
    expect(ids(inserted)).toEqual([...ids(absent), "current-quote"]);
    expect(ids(reordered)).toEqual(["current-quote", ...ids(absent)]);
    expect(inserted).toContain(stockBetaDictionary.en.intradayQuoteHeading);
    expect(quoteTag(reordered)).toContain("--desktop-order:0");
    expect(ids(absent)).toContain("ranked-signals");
  });

  it("omits a dashboard entry hidden at all breakpoints, including its empty state", () => {
    const { dashboard } = models();
    const hidden = stockBetaDashboardCatalog.map((entry) =>
      entry.id !== "current-quote"
        ? entry
        : {
            ...entry,
            placements: {
              desktop: {
                ...entry.placements.desktop,
                visible: false,
                empty: { ...entry.placements.desktop.empty, visible: false },
              },
              tablet: {
                ...entry.placements.tablet,
                visible: false,
                empty: { ...entry.placements.tablet.empty, visible: false },
              },
              mobile: {
                ...entry.placements.mobile,
                visible: false,
                empty: { ...entry.placements.mobile.empty, visible: false },
              },
            },
          },
    );
    for (const signals of [dashboard.signals, null]) {
      const markup = renderToStaticMarkup(
        <StockBetaSelectionProvider rows={signals?.rows ?? []}>
          {renderStockBetaDashboardGrid(defineStockBetaWidgetArchitecture(hidden), {
            ...dashboard,
            signals,
          })}
        </StockBetaSelectionProvider>,
      );
      expect(ids(markup)).not.toContain("current-quote");
      expect(markup).not.toContain(stockBetaDictionary.en.intradayQuoteHeading);
    }
  });

  it("inserts, removes and reorders the real detail entry without changing EOD widgets", () => {
    const viewModel = models().detail;
    const entry = stockBetaDetailCatalog.find((item) => item.id === "current-quote");
    if (entry === undefined) throw new Error("missing current-quote catalog entry");
    expect(entry.component).toBe(CurrentQuoteWidget);
    const remaining = stockBetaDetailCatalog.filter((item) => item.id !== "current-quote");
    const render = (catalog: readonly (typeof stockBetaDetailCatalog)[number][]) =>
      renderToStaticMarkup(
        renderStockBetaDetailGrid(defineStockBetaWidgetArchitecture(catalog), viewModel),
      );
    const absent = render(remaining);
    const inserted = render([...remaining, entry]);
    const reordered = render([entry, ...remaining]);
    expect(ids(absent)).not.toContain("current-quote");
    expect(ids(inserted)).toEqual([...ids(absent), "current-quote"]);
    expect(ids(reordered)).toEqual(["current-quote", ...ids(absent)]);
    expect(inserted).toContain(stockBetaDictionary.en.intradayQuoteHeading);
    expect(quoteTag(reordered)).toContain("--desktop-order:0");
    expect(ids(absent)).toContain("returns");
  });

  it("passes hidden detail placement through the real entry independently at every breakpoint", () => {
    const viewModel = models().detail;
    const hidden = stockBetaDetailCatalog.map((entry) =>
      entry.id !== "current-quote"
        ? entry
        : {
            ...entry,
            placements: {
              desktop: { ...entry.placements.desktop, visible: false },
              tablet: { ...entry.placements.tablet, visible: false },
              mobile: { ...entry.placements.mobile, visible: false },
            },
          },
    );
    const tree = renderStockBetaDetailGrid(defineStockBetaWidgetArchitecture(hidden), viewModel);
    const markup = renderToStaticMarkup(tree);
    for (const breakpoint of ["desktop", "tablet", "mobile"]) {
      expect(quoteTag(markup)).toContain(`data-${breakpoint}-visible="false"`);
    }
    expect(quotePlacement(tree)).toEqual({
      desktop: false,
      tablet: false,
      mobile: false,
    });
    // SSR proves composition/visibility propagation, not mounted effects. Hook/coordinator
    // visibility and unmount tests separately assert timer removal and demand release.
  });
});
