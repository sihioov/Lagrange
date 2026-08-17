import type { Metadata } from "next";
import Link from "next/link";
import { StockAnalysisReport } from "@/components/candidates/stock-analysis";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock analysis",
};

type StockPageProps = {
  readonly params: Promise<{ readonly instrument: string }>;
  readonly searchParams?: Promise<{ readonly date?: string }>;
};

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
  const date = (await searchParams)?.date;
  try {
    const report = await (await getProductApi()).getStockAnalysis(instrument, date);
    return frame(
      instrument,
      <>
        <nav aria-label="Research context" className="context-navigation">
          <Link href={`/candidates?date=${report.as_of}`}>Daily Top 5</Link>
          <Link href={`/screener?as_of=${report.as_of}`}>Screen this run</Link>
        </nav>
        <StockAnalysisReport report={report} />
      </>,
    );
  } catch (error) {
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
    if (error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND") {
      return frame(
        instrument,
        <StatePanel
          action={
            <Link className="secondary-action" href="/candidates">
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
