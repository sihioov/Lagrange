import type { Metadata } from "next";
import Link from "next/link";
import { StockAnalysisReport } from "@/components/candidates/stock-analysis";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import {
  DEFAULT_UNIVERSE,
  isUniverseKey,
  type UniverseKey,
  universeLabel,
} from "@/lib/products/candidate-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock analysis",
};

type StockPageProps = {
  readonly params: Promise<{ readonly instrument: string }>;
  readonly searchParams?: Promise<{
    readonly date?: string;
    readonly universe?: string | readonly string[];
  }>;
};

class InvalidStockUniverse extends Error {}

function selectedUniverse(value: string | readonly string[] | undefined): UniverseKey {
  if (Array.isArray(value) && value.length > 1) {
    throw new InvalidStockUniverse("Stock universe must be selected once.");
  }
  const raw = typeof value === "string" ? value : value?.[0];
  if (raw === undefined) return DEFAULT_UNIVERSE;
  if (!isUniverseKey(raw)) throw new InvalidStockUniverse("Stock universe is invalid.");
  return raw;
}

function frame(instrument: string, children: React.ReactNode) {
  return (
    <RoutePage
      description="Inspect point-in-time flow, fundamental, technical, and conditional scenario evidence for one KRX instrument."
      title={`Stock analysis · ${instrument}`}
    >
      {children}
    </RoutePage>
  );
}

export default async function StockPage({ params, searchParams }: StockPageProps) {
  const { instrument } = await params;
  const search = (await searchParams) ?? {};
  const date = search.date;
  try {
    const universe = selectedUniverse(search.universe);
    const report = await (await getProductApi()).getStockAnalysis(instrument, date, universe);
    return frame(
      instrument,
      <>
        <nav aria-label="Research context" className="context-navigation">
          <Link
            href={`/candidates?date=${encodeURIComponent(report.as_of)}&universe=${encodeURIComponent(report.universe)}`}
          >
            Daily Top 5 · {universeLabel(report.universe)}
          </Link>
          <Link
            href={`/screener?as_of=${encodeURIComponent(report.as_of)}&universes=${encodeURIComponent(report.universe)}`}
          >
            Screen this run · {universeLabel(report.universe)}
          </Link>
        </nav>
        <StockAnalysisReport report={report} />
      </>,
    );
  } catch (error) {
    if (error instanceof InvalidStockUniverse) {
      return frame(
        instrument,
        <StatePanel
          kind="error"
          message="Choose either the KOSPI 200 or KOSDAQ 150 stock-analysis universe."
          title="Stock universe is invalid"
        />,
      );
    }
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return frame(
        instrument,
        <StatePanel
          kind="blocked"
          message="One or more exact source datasets are not licensed for this research use. The analysis is withheld."
          title="Stock analysis is blocked"
        />,
      );
    }
    if (error instanceof ApiProblem && error.code === "DATA_STALE") {
      return frame(
        instrument,
        <StatePanel
          kind="error"
          message="The selected universe has no fresh governed analysis snapshot yet."
          title="Stock analysis is stale"
        />,
      );
    }
    if (error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND") {
      return frame(
        instrument,
        <StatePanel
          action={
            <Link
              className="secondary-action"
              href={`/candidates?universe=${encodeURIComponent(selectedUniverse(search.universe))}`}
            >
              Return to daily candidates
            </Link>
          }
          kind="empty"
          message="No published point-in-time analysis matches this instrument and date."
          title="Analysis not found"
        />,
      );
    }
    return frame(
      instrument,
      <StatePanel
        kind="error"
        message="The stock analysis could not be loaded. Retry after checking service readiness."
        title="Stock analysis unavailable"
      />,
    );
  }
}
