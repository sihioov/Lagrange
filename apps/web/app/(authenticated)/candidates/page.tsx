import type { Metadata } from "next";
import { CandidateFeedReport } from "@/components/candidates/candidate-feed";
import { RoutePage } from "@/components/pages/route-page";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Daily candidates",
};

type CandidatesPageProps = {
  readonly searchParams?: Promise<{ readonly date?: string }>;
};

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
    const date = (await searchParams)?.date;
    const feed = await (await getProductApi()).getCandidateFeed(date === "" ? undefined : date);
    return frame(
      <>
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
