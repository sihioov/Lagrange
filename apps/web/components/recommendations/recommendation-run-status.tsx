"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { StatePanel } from "@/components/states/state-panel";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { useLocale } from "@/lib/i18n/client";
import {
  type RecommendationsDictionary,
  recommendationsDictionary,
} from "@/lib/i18n/dictionaries/recommendations";
import { type RecommendationRunModel, recommendationRunSchema } from "@/lib/products/contracts";

const POLL_DELAYS_MS = [250, 500, 1_000, 2_000, 4_000] as const;

export function recommendationPollDelay(attempt: number): number {
  return POLL_DELAYS_MS[Math.min(attempt, POLL_DELAYS_MS.length - 1)] ?? 4_000;
}

export type RecommendationRunStatusProps = {
  readonly onSettled?: (run: RecommendationRunModel) => void;
  readonly poll?: boolean;
  readonly run: RecommendationRunModel;
};

function statusPanel(run: RecommendationRunModel, t: RecommendationsDictionary) {
  if (run.status === "PENDING") {
    return {
      kind: "loading" as const,
      message: t.pendingRunMessage,
      title: t.pendingRunTitle,
    };
  }
  if (run.status === "FAILED") {
    return {
      kind: "error" as const,
      message: t.failedRunMessage,
      title: t.failedRunTitle,
    };
  }
  if (run.status === "BLOCKED") {
    return {
      kind: "blocked" as const,
      message: t.blockedRunMessage,
      title: t.blockedRunTitle,
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
  const { locale } = useLocale();
  const t = recommendationsDictionary[locale];
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
        const next = await parseBrowserApiResponse(response, recommendationRunSchema);
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
          setPollError(error instanceof Error ? error.message : t.pollErrorFallback);
        }
      }
      if (!cancelled) {
        timeout = setTimeout(() => {
          void pollRun(attempt + 1);
        }, recommendationPollDelay(attempt));
      }
    };

    timeout = setTimeout(() => {
      void pollRun(1);
    }, recommendationPollDelay(0));

    return () => {
      cancelled = true;
      if (timeout !== undefined) {
        clearTimeout(timeout);
      }
    };
  }, [currentRun.status, onSettled, poll, router, run.id, t.pollErrorFallback]);

  const panel = statusPanel(currentRun, t);
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
