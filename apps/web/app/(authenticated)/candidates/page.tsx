import type { Metadata } from "next";
import Link from "next/link";
import { CandidateFeedReport } from "@/components/candidates/candidate-feed";
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
  title: "Daily candidates",
};

type CandidatesPageProps = {
  readonly searchParams?: Promise<{
    readonly date?: string;
    readonly universe?: string | readonly string[];
  }>;
};

class InvalidCandidateUniverse extends Error {}

function selectedUniverse(value: string | readonly string[] | undefined): UniverseKey {
  if (Array.isArray(value) && value.length > 1) {
    throw new InvalidCandidateUniverse("Candidate universe must be selected once.");
  }
  const raw = typeof value === "string" ? value : value?.[0];
  if (raw === undefined) return DEFAULT_UNIVERSE;
  if (!isUniverseKey(raw)) throw new InvalidCandidateUniverse("Candidate universe is invalid.");
  return raw;
}

function universeHref(universe: UniverseKey, date: string | undefined): string {
  const params = new URLSearchParams({ universe });
  if (date !== undefined && date !== "") params.set("date", date);
  return `/candidates?${params.toString()}`;
}

function frame(children: React.ReactNode) {
  return (
    <RoutePage
      description="Review the common post-close Top 5 built from investor flow, fundamental, and technical evidence."
      title="Daily candidates"
    >
      {children}
    </RoutePage>
  );
}

export default async function CandidatesPage({ searchParams }: CandidatesPageProps = {}) {
  try {
    const params = (await searchParams) ?? {};
    const date = params.date;
    const universe = selectedUniverse(params.universe);
    const feed = await (await getProductApi()).getCandidateFeed(
      date === "" ? undefined : date,
      universe,
    );
    return frame(
      <>
        <nav aria-label="Candidate universes" className="universe-tabs">
          {(["kospi200", "kosdaq150"] as const).map((candidateUniverse) => (
            <Link
              aria-current={candidateUniverse === universe ? "page" : undefined}
              className={candidateUniverse === universe ? "is-selected" : undefined}
              href={universeHref(candidateUniverse, date)}
              key={candidateUniverse}
            >
              {universeLabel(candidateUniverse)}
            </Link>
          ))}
        </nav>
        <section aria-labelledby="candidate-date-title" className="workflow-panel">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Immutable history</p>
              <h2 id="candidate-date-title">Choose a research date</h2>
            </div>
            <p>Leave the date empty to load the newest published candidate feed.</p>
          </div>
          <form className="workflow-form" method="get">
            <label className="form-field">
              <span>As-of date</span>
              <input defaultValue={date ?? feed.as_of} name="date" type="date" />
            </label>
            <button className="secondary-action" type="submit">
              Load governed snapshot
            </button>
          </form>
        </section>
        <CandidateFeedReport feed={feed} />
      </>,
    );
  } catch (error) {
    if (error instanceof InvalidCandidateUniverse) {
      return frame(
        <StatePanel
          kind="error"
          message="Choose either the KOSPI 200 or KOSDAQ 150 candidate universe."
          title="Candidate universe is invalid"
        />,
      );
    }
    if (
      error instanceof ApiProblem &&
      ["DATASET_BLOCKED", "DATA_ENTITLEMENT_REQUIRED", "FORBIDDEN"].includes(error.code)
    ) {
      return frame(
        <StatePanel
          kind="blocked"
          message="One or more exact source datasets are not licensed for candidate research. Proprietary rows are not rendered."
          title="Candidate research is blocked"
        />,
      );
    }
    if (error instanceof ApiProblem && error.code === "DATA_STALE") {
      return frame(
        <StatePanel
          kind="error"
          message="The selected candidate universe has no fresh governed snapshot yet."
          title="Candidate research is stale"
        />,
      );
    }
    if (error instanceof ApiProblem && error.code === "RESOURCE_NOT_FOUND") {
      return frame(
        <StatePanel
          kind="empty"
          message="No immutable candidate feed has been published for this date yet."
          title="No candidate snapshot"
        />,
      );
    }
    return frame(
      <StatePanel
        kind="error"
        message="Candidate research could not be loaded. Retry after checking the candidate-runner readiness."
        title="Candidate research unavailable"
      />,
    );
  }
}
