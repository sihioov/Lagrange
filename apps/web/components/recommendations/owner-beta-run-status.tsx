"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { StatePanel } from "@/components/states/state-panel";
import { parseApiResponse } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import {
  type RecommendationsDictionary,
  recommendationsDictionary,
} from "@/lib/i18n/dictionaries/recommendations";
import {
  type OwnerBetaRunStatus as OwnerBetaRunStatusValue,
  ownerBetaRunPath,
  ownerBetaRunSchema,
} from "@/lib/products/owner-beta-contracts";

const OWNER_BETA_POLL_DELAYS_MS = [250, 500, 1_000, 2_000, 4_000] as const;

export function ownerBetaPollDelay(attempt: number): number {
  return (
    OWNER_BETA_POLL_DELAYS_MS[Math.min(attempt, OWNER_BETA_POLL_DELAYS_MS.length - 1)] ?? 4_000
  );
}

export type OwnerBetaRunStatusProps = {
  readonly initialStatus: OwnerBetaRunStatusValue;
  readonly runId: string;
};

export type OwnerBetaPollOptions = {
  readonly fetcher?: typeof fetch;
  readonly refresh?: () => void;
};

export async function pollOwnerBetaRun(
  runId: string,
  options: OwnerBetaPollOptions = {},
): Promise<OwnerBetaRunStatusValue> {
  const response = await (options.fetcher ?? fetch)(ownerBetaRunPath(runId), {
    cache: "no-store",
    credentials: "same-origin",
  });
  const next = await parseApiResponse(response, ownerBetaRunSchema);
  if (next.status === "SUCCEEDED" || next.status === "FAILED" || next.status === "CANCELED") {
    options.refresh?.();
  }
  return next.status;
}

function statusPanel(status: OwnerBetaRunStatusValue, t: RecommendationsDictionary) {
  if (status === "PENDING") {
    return {
      kind: "loading" as const,
      message: t.pendingRunMessage,
      title: t.pendingRunTitle,
    };
  }
  if (status === "RUNNING") {
    return {
      kind: "loading" as const,
      message: t.ownerBetaRunningMessage,
      title: t.ownerBetaRunningTitle,
    };
  }
  if (status === "FAILED") {
    return {
      kind: "error" as const,
      message: t.failedRunMessage,
      title: t.failedRunTitle,
    };
  }
  if (status === "CANCELED") {
    return {
      kind: "blocked" as const,
      message: t.ownerBetaCanceledMessage,
      title: t.ownerBetaCanceledTitle,
    };
  }
  return null;
}

export function OwnerBetaRunStatus({ initialStatus, runId }: OwnerBetaRunStatusProps) {
  const router = useRouter();
  const { locale } = useLocale();
  const t = recommendationsDictionary[locale];
  const [status, setStatus] = useState<OwnerBetaRunStatusValue>(initialStatus);
  const [pollError, setPollError] = useState<string | null>(null);

  useEffect(() => {
    setStatus(initialStatus);
    setPollError(null);
  }, [initialStatus]);

  useEffect(() => {
    if (status !== "PENDING" && status !== "RUNNING") {
      return;
    }
    let cancelled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;

    const pollRun = async (attempt: number): Promise<void> => {
      try {
        const nextStatus = await pollOwnerBetaRun(runId, { refresh: router.refresh });
        if (cancelled) {
          return;
        }
        setStatus(nextStatus);
        if (nextStatus === "SUCCEEDED" || nextStatus === "FAILED" || nextStatus === "CANCELED") {
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
        }, ownerBetaPollDelay(attempt));
      }
    };

    timeout = setTimeout(() => {
      void pollRun(0);
    }, ownerBetaPollDelay(0));

    return () => {
      cancelled = true;
      if (timeout !== undefined) {
        clearTimeout(timeout);
      }
    };
  }, [router, runId, status, t.pollErrorFallback]);

  const panel = statusPanel(status, t);
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
