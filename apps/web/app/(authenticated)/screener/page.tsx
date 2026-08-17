import type { Metadata } from "next";
import { RoutePage } from "@/components/pages/route-page";
import { SavedScreens } from "@/components/screener/saved-screens";
import { ScreenerControls } from "@/components/screener/screener-controls";
import { ScreenerResults } from "@/components/screener/screener-results";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import type { SavedScreen, ScreenCriteria } from "@/lib/products/candidate-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Stock screener",
};

type SearchValue = string | readonly string[] | undefined;
type ScreenerPageProps = {
  readonly searchParams?: Promise<Readonly<Record<string, SearchValue>>>;
};

class InvalidScreenFilters extends Error {}

function values(value: SearchValue): readonly string[] {
  if (value === undefined) return [];
  return typeof value === "string" ? [value] : value;
}

function first(value: SearchValue): string | undefined {
  return values(value)[0];
}

function score(value: SearchValue, name: string): number | undefined {
  const raw = first(value);
  if (raw === undefined || raw === "") return undefined;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 100) {
    throw new InvalidScreenFilters(`${name} must be between 0 and 100.`);
  }
  return parsed;
}

function criteriaFrom(params: Readonly<Record<string, SearchValue>>): ScreenCriteria {
  const sectors = (first(params["sectors"]) ?? "")
    .split(",")
    .map((sector) => sector.trim())
    .filter((sector, index, rows) => sector !== "" && rows.indexOf(sector) === index);
  if (sectors.length > 64 || sectors.some((sector) => sector.length > 32)) {
    throw new InvalidScreenFilters("Use at most 64 sector codes of 32 characters or fewer.");
  }
  const evidence = values(params["evidence"]);
  if (evidence.some((value) => !["STRONG", "MODERATE", "WEAK"].includes(value))) {
    throw new InvalidScreenFilters("Evidence strength is invalid.");
  }
  return {
    sectors,
    evidence_strength: evidence as ScreenCriteria["evidence_strength"],
    min_total_score: score(params["min_total_score"], "Minimum total score"),
    min_flow_score: score(params["min_flow_score"], "Minimum flow score"),
    min_fundamental_score: score(params["min_fundamental_score"], "Minimum fundamental score"),
    min_technical_score: score(params["min_technical_score"], "Minimum technical score"),
  };
}

function href(asOf: string, criteria: ScreenCriteria, cursor?: string): string {
  const params = new URLSearchParams({ as_of: asOf });
  if ((criteria.sectors ?? []).length > 0) params.set("sectors", criteria.sectors?.join(",") ?? "");
  for (const evidence of criteria.evidence_strength ?? []) params.append("evidence", evidence);
  for (const [name, value] of [
    ["min_total_score", criteria.min_total_score],
    ["min_flow_score", criteria.min_flow_score],
    ["min_fundamental_score", criteria.min_fundamental_score],
    ["min_technical_score", criteria.min_technical_score],
  ] as const) {
    if (value !== null && value !== undefined) params.set(name, String(value));
  }
  if (cursor !== undefined) params.set("cursor", cursor);
  return `/screener?${params.toString()}`;
}

function withHref(
  screens: readonly SavedScreen[],
  asOf: string,
): readonly (SavedScreen & { readonly href: string })[] {
  return screens.map((screen) => ({ ...screen, href: href(asOf, screen.criteria) }));
}

function frame(children: React.ReactNode) {
  return (
    <RoutePage
      description="Filter one immutable candidate run without changing its ranking, evidence, or source lineage."
      title="Stock screener"
    >
      {children}
    </RoutePage>
  );
}

export default async function ScreenerPage({ searchParams }: ScreenerPageProps = {}) {
  try {
    const params = (await searchParams) ?? {};
    const criteria = criteriaFrom(params);
    const api = await getProductApi();
    const requestedAsOf = first(params["as_of"]);
    const feed = await api.getCandidateFeed(requestedAsOf);
    const runId = feed.items[0]?.run_id;
    if (runId === undefined) throw new Error("Candidate feed did not identify its run.");
    const [result, saved] = await Promise.all([
      api.queryScreener({
        run_id: runId,
        criteria,
        cursor: first(params["cursor"]) ?? null,
        limit: 25,
      }),
      api.getSavedScreens(),
    ]);
    return frame(
      <>
        <ScreenerControls asOf={feed.as_of} criteria={criteria} />
        <SavedScreens criteria={criteria} screens={withHref(saved.items, feed.as_of)} />
        <ScreenerResults
          nextHref={
            result.next_cursor === null ? null : href(feed.as_of, criteria, result.next_cursor)
          }
          result={result}
        />
      </>,
    );
  } catch (error) {
    if (error instanceof InvalidScreenFilters) {
      return frame(
        <StatePanel kind="error" message={error.message} title="Screen filters are invalid" />,
      );
    }
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return frame(
        <StatePanel
          kind="blocked"
          message="One or more exact source datasets are not licensed for candidate screening. Proprietary rows are not rendered."
          title="Stock screener is blocked"
        />,
      );
    }
    if (error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND") {
      return frame(
        <StatePanel
          kind="empty"
          message="Publish a candidate run before screening the governed universe."
          title="No candidate run is available"
        />,
      );
    }
    return frame(
      <StatePanel
        kind="error"
        message="The screener could not be loaded. Retry after checking API and candidate-runner readiness."
        title="Stock screener unavailable"
      />,
    );
  }
}
