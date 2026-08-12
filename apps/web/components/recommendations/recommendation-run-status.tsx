"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { StatePanel } from "@/components/states/state-panel";
import { parseApiResponse } from "@/lib/api/response";
import { type RecommendationRunModel, recommendationRunSchema } from "@/lib/products/contracts";

const POLL_DELAYS_MS = [250, 500, 1_000, 2_000, 4_000] as const;

export type RecommendationRunStatusProps = {
  readonly onSettled?: (run: RecommendationRunModel) => void;
  readonly poll?: boolean;
  readonly run: RecommendationRunModel;
};

function statusPanel(run: RecommendationRunModel) {
  if (run.status === "PENDING") {
    return {
      kind: "loading" as const,
      message:
        "The server is producing the recommendation. The last successful proposal remains available.",
      title: "Recommendation is in progress",
    };
  }
  if (run.status === "FAILED") {
    return {
      kind: "error" as const,
      message:
        "The worker did not produce a recommendation. Candidate payloads for this run remain hidden.",
      title: "Recommendation failed",
    };
  }
  if (run.status === "BLOCKED") {
    return {
      kind: "blocked" as const,
      message: "The server blocked this run. Candidate payloads remain hidden.",
      title: "Recommendation run blocked",
    };
  }
  return null;
}

export function RecommendationRunStatus({
  onSettled,
  poll = false,
  run,
}: RecommendationRunStatusProps) {
  const router = useRouter();
  const [currentRun, setCurrentRun] = useState(run);
  const [pollError, setPollError] = useState<string | null>(null);

  useEffect(() => {
    setCurrentRun(run);
    setPollError(null);
  }, [run]);

  useEffect(() => {
    if (!poll || currentRun.status !== "PENDING") {
      return;
    }
    let cancelled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;

    const pollRun = async (attempt: number): Promise<void> => {
      try {
        const response = await fetch(`/api/v1/recommendations/runs/${encodeURIComponent(run.id)}`, {
          cache: "no-store",
          credentials: "same-origin",
        });
        const next = await parseApiResponse(response, recommendationRunSchema);
        if (cancelled) {
          return;
        }
        setCurrentRun(next);
        if (next.status !== "PENDING") {
          onSettled?.(next);
          router.refresh();
          return;
        }
      } catch (error) {
        if (!cancelled) {
          setPollError(
            error instanceof Error ? error.message : "Run status could not be refreshed.",
          );
        }
      }
      if (!cancelled && attempt < POLL_DELAYS_MS.length) {
        timeout = setTimeout(() => {
          void pollRun(attempt + 1);
        }, POLL_DELAYS_MS[attempt]);
      }
    };

    timeout = setTimeout(() => {
      void pollRun(1);
    }, POLL_DELAYS_MS[0]);

    return () => {
      cancelled = true;
      if (timeout !== undefined) {
        clearTimeout(timeout);
      }
    };
  }, [currentRun.status, onSettled, poll, router, run.id]);

  const panel = statusPanel(currentRun);
  if (panel === null) {
    return null;
  }
  return (
    <>
      <StatePanel {...panel} />
      {pollError === null ? null : (
        <p className="form-result" role="status">
          {pollError}
        </p>
      )}
    </>
  );
}
