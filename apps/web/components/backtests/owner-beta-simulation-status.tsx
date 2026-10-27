"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { z } from "zod";
import { StatePanel } from "@/components/states/state-panel";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { subscribeToBrowserLogout } from "@/lib/api/browser-lifecycle";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { ApiContractError, ApiProblem } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { ownerBetaSimulationDictionary } from "@/lib/i18n/dictionaries/owner-beta-simulation";
import {
  type OwnerBetaSimulationEnqueued,
  type OwnerBetaSimulationPair,
  type OwnerBetaSimulationRunBodyV2,
  type OwnerBetaSimulationVersionedRun,
  ownerBetaSimulationCancelPath,
  ownerBetaSimulationCancelSchema,
  ownerBetaSimulationPageSchema,
  ownerBetaSimulationPairMatchesRun,
  ownerBetaSimulationPairPath,
  ownerBetaSimulationPairSchema,
  ownerBetaSimulationRunMatchesRequest,
  ownerBetaSimulationRunPath,
  ownerBetaSimulationRunsPath,
  ownerBetaSimulationVersionedRunSchema,
} from "@/lib/products/owner-beta-simulation-contracts";
import type { OwnerBetaSimulationStrategyId } from "@/lib/products/owner-beta-simulation-v2-contracts";
import type { OwnerBetaSimulationFormConfig } from "./owner-beta-simulation-form";
import { OwnerBetaSimulationForm } from "./owner-beta-simulation-form";
import { OwnerBetaSimulationHistory } from "./owner-beta-simulation-history";
import { OwnerBetaSimulationReport } from "./owner-beta-simulation-report";

const PAGE_LIMIT = 20;
const MAX_HISTORY_ROWS = 100;
const POLL_INTERVAL_MS = 2_000;
const MAX_AUTOMATIC_POLLS = 60;
const ENTITLEMENT_ERROR_CODES: ReadonlySet<string> = new Set([
  "DATA_ENTITLEMENT_REQUIRED",
  "DATASET_BLOCKED",
  "DATA_STALE",
]);

type ErrorKind = "entitlement" | "forbidden" | "integrity" | "missing" | "notReady" | "unavailable";
type PollRefresh = (manual?: boolean) => void;

export function ownerBetaSimulationAutomaticPollAllowed(completedAttempts: number): boolean {
  return (
    Number.isInteger(completedAttempts) &&
    completedAttempts >= 0 &&
    completedAttempts < MAX_AUTOMATIC_POLLS
  );
}

function classifyError(error: unknown): ErrorKind {
  if (error instanceof ApiProblem) {
    if (ENTITLEMENT_ERROR_CODES.has(error.code)) {
      return "entitlement";
    }
    if (error.code === "FORBIDDEN") return "forbidden";
    if (error.code === "RESOURCE_NOT_FOUND") return "missing";
    if (error.code === "SIMULATION_RESULT_NOT_READY") return "notReady";
    if (error.code === "RESULT_INTEGRITY_FAILED") return "integrity";
  }
  if (error instanceof ApiContractError) return "integrity";
  return "unavailable";
}

function isSessionLoss(error: unknown): boolean {
  return (
    error instanceof ApiProblem &&
    (error.code === "SESSION_UNKNOWN" || error.code === "SESSION_EXPIRED")
  );
}

function errorPanel(kind: ErrorKind, t: (typeof ownerBetaSimulationDictionary)["en"]) {
  switch (kind) {
    case "entitlement":
      return { title: t.entitlementTitle, message: t.entitlementMessage, kind: "blocked" as const };
    case "forbidden":
      return { title: t.forbiddenTitle, message: t.forbiddenMessage, kind: "blocked" as const };
    case "integrity":
      return { title: t.integrityTitle, message: t.integrityMessage, kind: "error" as const };
    case "missing":
      return { title: t.missingTitle, message: t.missingMessage, kind: "blocked" as const };
    case "notReady":
      return { title: t.notReadyTitle, message: t.notReadyMessage, kind: "loading" as const };
    case "unavailable":
      return { title: t.unavailableTitle, message: t.unavailableMessage, kind: "error" as const };
  }
}

async function getSimulationResponse<Output>(
  path: string,
  schema: z.ZodType<Output>,
  signal: AbortSignal,
): Promise<Output> {
  const response = await fetch(path, {
    cache: "no-store",
    credentials: "same-origin",
    signal,
  });
  return parseBrowserApiResponse(response, schema, { signal });
}

export type OwnerBetaSimulationStatusProps = {
  readonly configs: readonly OwnerBetaSimulationFormConfig[];
};

export function OwnerBetaSimulationStatus({ configs }: OwnerBetaSimulationStatusProps) {
  const { locale } = useLocale();
  const t = ownerBetaSimulationDictionary[locale];
  const [runs, setRuns] = useState<readonly OwnerBetaSimulationVersionedRun[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyLoadingMore, setHistoryLoadingMore] = useState(false);
  const [historyError, setHistoryError] = useState(false);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [run, setRun] = useState<OwnerBetaSimulationVersionedRun | null>(null);
  const [pair, setPair] = useState<OwnerBetaSimulationPair | null>(null);
  const [statusError, setStatusError] = useState<ErrorKind | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const [cancelAcknowledged, setCancelAcknowledged] = useState(false);
  const [pollingExhausted, setPollingExhausted] = useState(false);
  const runRef = useRef<OwnerBetaSimulationVersionedRun | null>(null);
  const selectedIdRef = useRef<string | null>(null);
  const statusControllerRef = useRef<AbortController | null>(null);
  const historyControllerRef = useRef<AbortController | null>(null);
  const cancelControllerRef = useRef<AbortController | null>(null);
  const historyBusyRef = useRef(false);
  const refreshRef = useRef<PollRefresh | null>(null);
  const pairCacheRef = useRef<{
    readonly runId: string;
    readonly pair: OwnerBetaSimulationPair;
  } | null>(null);
  const accessEpochRef = useRef(0);
  const cancelKeyRef = useRef<{ readonly runId: string; readonly key: string } | null>(null);
  const enqueueBindingRef = useRef<{
    readonly queued: OwnerBetaSimulationEnqueued;
    readonly body: OwnerBetaSimulationRunBodyV2;
    readonly strategyId: OwnerBetaSimulationStrategyId;
  } | null>(null);

  const loadFirstPage = useCallback(async (controller: AbortController): Promise<void> => {
    setHistoryLoading(true);
    setHistoryError(false);
    try {
      const page = await getSimulationResponse(
        ownerBetaSimulationRunsPath({ limit: PAGE_LIMIT }),
        ownerBetaSimulationPageSchema,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      setRuns(page.items);
      setNextCursor(page.next_cursor);
      setHasMore(page.has_more);
    } catch (error) {
      if (!controller.signal.aborted) {
        setHistoryError(true);
        const kind = classifyError(error);
        if (kind === "entitlement" || kind === "forbidden" || isSessionLoss(error)) {
          accessEpochRef.current += 1;
          pairCacheRef.current = null;
          setPair(null);
          if (!isSessionLoss(error)) setStatusError(kind);
          if (isSessionLoss(error)) {
            selectedIdRef.current = null;
            runRef.current = null;
            setRun(null);
            setRuns([]);
            setSelectedRunId(null);
          }
        }
      }
    } finally {
      if (!controller.signal.aborted) setHistoryLoading(false);
    }
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    historyControllerRef.current = controller;
    void loadFirstPage(controller);
    return () => {
      controller.abort();
      if (historyControllerRef.current === controller) historyControllerRef.current = null;
    };
  }, [loadFirstPage]);

  function updateRun(value: OwnerBetaSimulationVersionedRun | null): void {
    runRef.current = value;
    setRun(value);
  }

  function refreshHistory(): void {
    historyControllerRef.current?.abort();
    historyBusyRef.current = false;
    setHistoryLoadingMore(false);
    const controller = new AbortController();
    historyControllerRef.current = controller;
    void loadFirstPage(controller);
  }

  async function loadMore(): Promise<void> {
    if (historyBusyRef.current || nextCursor === null || runs.length >= MAX_HISTORY_ROWS) return;
    historyBusyRef.current = true;
    setHistoryLoadingMore(true);
    setHistoryError(false);
    const controller = new AbortController();
    historyControllerRef.current?.abort();
    historyControllerRef.current = controller;
    try {
      const page = await getSimulationResponse(
        ownerBetaSimulationRunsPath({ cursor: nextCursor, limit: PAGE_LIMIT }),
        ownerBetaSimulationPageSchema,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      setRuns((current) => {
        const seen = new Set(current.map((item) => item.id));
        const additions = page.items.filter((item) => !seen.has(item.id));
        return [...current, ...additions].slice(0, MAX_HISTORY_ROWS);
      });
      setNextCursor(page.next_cursor);
      setHasMore(page.has_more);
    } catch (error) {
      if (!controller.signal.aborted) {
        setHistoryError(true);
        const kind = classifyError(error);
        if (kind === "entitlement" || kind === "forbidden" || isSessionLoss(error)) {
          accessEpochRef.current += 1;
          pairCacheRef.current = null;
          setPair(null);
          if (!isSessionLoss(error)) setStatusError(kind);
        }
      }
    } finally {
      if (!controller.signal.aborted && historyControllerRef.current === controller) {
        setHistoryLoadingMore(false);
        historyBusyRef.current = false;
      }
    }
  }

  function selectRun(selected: OwnerBetaSimulationVersionedRun): void {
    if (selected.id === selectedIdRef.current) {
      refreshRef.current?.(true);
      return;
    }
    statusControllerRef.current?.abort();
    selectedIdRef.current = selected.id;
    accessEpochRef.current += 1;
    pairCacheRef.current = null;
    enqueueBindingRef.current = null;
    setSelectedRunId(selected.id);
    updateRun(selected);
    setPair(null);
    setStatusError(null);
    setRefreshing(false);
    setCancelAcknowledged(false);
    setPollingExhausted(false);
  }

  function onQueued(
    queued: OwnerBetaSimulationEnqueued,
    body: OwnerBetaSimulationRunBodyV2,
    strategyId: OwnerBetaSimulationStrategyId,
  ): void {
    const runId = queued.run_id;
    statusControllerRef.current?.abort();
    selectedIdRef.current = runId;
    accessEpochRef.current += 1;
    pairCacheRef.current = null;
    enqueueBindingRef.current = { queued, body, strategyId };
    setSelectedRunId(runId);
    updateRun(null);
    setPair(null);
    setStatusError(null);
    setCancelAcknowledged(false);
    setPollingExhausted(false);
    refreshHistory();
  }

  function handleAccessLoss(kind: "entitlement" | "forbidden" | "session"): void {
    accessEpochRef.current += 1;
    pairCacheRef.current = null;
    setPair(null);
    if (kind === "session") {
      runRef.current = null;
      setRun(null);
      setStatusError(null);
      return;
    }
    setStatusError(kind);
  }

  useEffect(() => {
    const unsubscribe = subscribeToBrowserLogout(() => {
      statusControllerRef.current?.abort();
      historyControllerRef.current?.abort();
      cancelControllerRef.current?.abort();
      selectedIdRef.current = null;
      accessEpochRef.current += 1;
      setSelectedRunId(null);
      runRef.current = null;
      setRun(null);
      setPair(null);
      pairCacheRef.current = null;
      setRuns([]);
      setNextCursor(null);
      setHasMore(false);
      setStatusError(null);
      setCancelAcknowledged(false);
      setPollingExhausted(false);
      setCanceling(false);
      setRefreshing(false);
      setHistoryLoading(false);
      setHistoryLoadingMore(false);
      historyBusyRef.current = false;
      enqueueBindingRef.current = null;
      cancelKeyRef.current = null;
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    selectedIdRef.current = selectedRunId;
    if (selectedRunId === null) {
      refreshRef.current = null;
      return;
    }

    const controller = new AbortController();
    statusControllerRef.current = controller;
    let active = true;
    let busy = false;
    let manualRefreshQueued = false;
    let automaticAttempts = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const refreshSelected: PollRefresh = (manual = false) => {
      if (!active || selectedIdRef.current !== selectedRunId) return;
      if (busy) {
        if (manual) manualRefreshQueued = true;
        return;
      }
      if (!manual && !ownerBetaSimulationAutomaticPollAllowed(automaticAttempts)) {
        setPollingExhausted(true);
        return;
      }
      if (timer !== undefined) clearTimeout(timer);
      busy = true;
      const requestEpoch = accessEpochRef.current;
      if (!manual) automaticAttempts += 1;
      setRefreshing(true);
      setStatusError(null);
      void (async () => {
        let shouldPoll = false;
        let observedStatus: OwnerBetaSimulationVersionedRun["status"] | undefined;
        try {
          const current = await getSimulationResponse(
            ownerBetaSimulationRunPath(selectedRunId),
            ownerBetaSimulationVersionedRunSchema,
            controller.signal,
          );
          if (
            !active ||
            controller.signal.aborted ||
            selectedIdRef.current !== selectedRunId ||
            accessEpochRef.current !== requestEpoch
          )
            return;
          if (current.id !== selectedRunId)
            throw new ApiContractError(200, "Run identity mismatch");
          const enqueueBinding = enqueueBindingRef.current;
          if (
            enqueueBinding?.queued.run_id === selectedRunId &&
            (current.job_id !== enqueueBinding.queued.job_id ||
              !ownerBetaSimulationRunMatchesRequest(
                current,
                enqueueBinding.body,
                selectedRunId,
                enqueueBinding.strategyId,
              ))
          ) {
            throw new ApiContractError(200, "Run did not match the submitted intent");
          }
          observedStatus = current.status;
          runRef.current = current;
          setRun(current);
          setPair(null);
          if (current.status === "SUCCEEDED") {
            const cachedPair = pairCacheRef.current;
            let completePair = cachedPair?.runId === selectedRunId ? cachedPair.pair : null;
            if (completePair === null) {
              completePair = await getSimulationResponse(
                ownerBetaSimulationPairPath(selectedRunId),
                ownerBetaSimulationPairSchema,
                controller.signal,
              );
              if (
                !active ||
                controller.signal.aborted ||
                selectedIdRef.current !== selectedRunId ||
                accessEpochRef.current !== requestEpoch
              )
                return;
            }
            if (
              completePair.run_id !== current.id ||
              !ownerBetaSimulationPairMatchesRun(completePair, current)
            ) {
              throw new ApiContractError(200, "Result pair did not match selected run metadata");
            }
            pairCacheRef.current = { pair: completePair, runId: selectedRunId };
            setPair(completePair);
            enqueueBindingRef.current = null;
          } else {
            pairCacheRef.current = null;
            shouldPoll = current.status === "PENDING" || current.status === "RUNNING";
            if (!shouldPoll) enqueueBindingRef.current = null;
          }
        } catch (error) {
          if (
            !active ||
            controller.signal.aborted ||
            selectedIdRef.current !== selectedRunId ||
            accessEpochRef.current !== requestEpoch
          )
            return;
          setPair(null);
          pairCacheRef.current = null;
          const kind = classifyError(error);
          if (kind === "entitlement" || kind === "forbidden" || isSessionLoss(error)) {
            accessEpochRef.current += 1;
          }
          setStatusError(kind);
          if (isSessionLoss(error)) {
            runRef.current = null;
            setRun(null);
          }
          shouldPoll =
            kind === "unavailable" &&
            (observedStatus === "PENDING" ||
              observedStatus === "RUNNING" ||
              (observedStatus === undefined &&
                (runRef.current?.status === "PENDING" ||
                  runRef.current?.status === "RUNNING" ||
                  (enqueueBindingRef.current?.queued.run_id === selectedRunId &&
                    enqueueBindingRef.current.queued.status === "PENDING"))));
          if (kind === "entitlement" || kind === "forbidden" || kind === "missing") {
            // Results are purged immediately when the server revokes access.
            setPair(null);
          }
        } finally {
          busy = false;
          if (active && !controller.signal.aborted && selectedIdRef.current === selectedRunId) {
            setRefreshing(false);
          }
          if (active && manualRefreshQueued) {
            manualRefreshQueued = false;
            queueMicrotask(() => refreshSelected(true));
          }
        }
        if (!active || controller.signal.aborted || selectedIdRef.current !== selectedRunId) return;
        if (shouldPoll && ownerBetaSimulationAutomaticPollAllowed(automaticAttempts)) {
          timer = setTimeout(() => refreshSelected(false), POLL_INTERVAL_MS);
        } else if (shouldPoll) {
          setPollingExhausted(true);
        }
      })();
    };

    refreshRef.current = refreshSelected;
    void refreshSelected(false);
    return () => {
      active = false;
      controller.abort();
      if (timer !== undefined) clearTimeout(timer);
      if (statusControllerRef.current === controller) statusControllerRef.current = null;
    };
  }, [selectedRunId]);

  async function cancelSelectedRun(): Promise<void> {
    const selectedId = selectedIdRef.current;
    const selected = run;
    const queued = enqueueBindingRef.current?.queued;
    const expectedJobId =
      selected?.job_id ?? (queued?.run_id === selectedId ? queued.job_id : undefined);
    const activeStatus =
      selected?.status === "PENDING" ||
      selected?.status === "RUNNING" ||
      (queued?.run_id === selectedId && queued.status === "PENDING");
    if (selectedId === null || !activeStatus || canceling) {
      return;
    }
    const previous = cancelKeyRef.current;
    const idempotencyKey = previous?.runId === selectedId ? previous.key : crypto.randomUUID();
    cancelKeyRef.current = { runId: selectedId, key: idempotencyKey };
    const controller = new AbortController();
    cancelControllerRef.current = controller;
    setCanceling(true);
    setStatusError(null);
    try {
      const response = await mutateWithCsrf(ownerBetaSimulationCancelPath(selectedId), {
        idempotencyKey,
        json: {},
        method: "POST",
        signal: controller.signal,
      });
      const acknowledgement = await parseBrowserApiResponse(
        response,
        ownerBetaSimulationCancelSchema,
        { signal: controller.signal },
      );
      if (response.status !== 200) {
        throw new ApiContractError(response.status, "Simulation cancellation did not return 200");
      }
      if (
        acknowledgement.run_id !== selectedId ||
        (expectedJobId !== undefined && acknowledgement.job_id !== expectedJobId)
      ) {
        throw new ApiContractError(200, "Cancellation acknowledgement identity mismatch");
      }
      if (controller.signal.aborted || selectedIdRef.current !== selectedId) return;
      setCancelAcknowledged(true);
      refreshRef.current?.(true);
    } catch (error) {
      if (controller.signal.aborted || selectedIdRef.current !== selectedId) return;
      const kind = classifyError(error);
      if (isSessionLoss(error)) {
        handleAccessLoss("session");
      } else if (kind === "entitlement" || kind === "forbidden") {
        handleAccessLoss(kind);
      } else {
        accessEpochRef.current += 1;
        pairCacheRef.current = null;
        setPair(null);
        setStatusError(kind);
      }
    } finally {
      if (cancelControllerRef.current === controller) cancelControllerRef.current = null;
      if (!controller.signal.aborted) setCanceling(false);
    }
  }

  const statusPanel = statusError === null ? null : errorPanel(statusError, t);
  const currentStatus = run?.status;
  const canCancel =
    selectedRunId !== null &&
    (currentStatus === "PENDING" ||
      currentStatus === "RUNNING" ||
      (enqueueBindingRef.current?.queued.run_id === selectedRunId &&
        enqueueBindingRef.current.queued.status === "PENDING"));
  const visiblePair =
    pair !== null && run?.status === "SUCCEEDED" && pair.run_id === selectedRunId ? pair : null;
  const showRunPanel = selectedRunId !== null && statusError === null;

  return (
    <>
      <OwnerBetaSimulationForm
        configs={configs}
        onAccessLoss={handleAccessLoss}
        onQueued={onQueued}
      />
      {showRunPanel ? (
        <section aria-labelledby="owner-beta-simulation-status-title" className="product-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">{t.statusSectionLabel}</p>
              <h2 id="owner-beta-simulation-status-title">{t.statusTitle}</h2>
            </div>
            {run === null ? null : <p>{t.selectedPeriodLabel(run.start_date, run.end_date)}</p>}
          </div>
          {run === null ? (
            <StatePanel
              kind={refreshing ? "loading" : "error"}
              message={refreshing ? t.pendingMessage : t.genericErrorMessage}
              title={refreshing ? t.pendingTitle : t.genericErrorTitle}
            />
          ) : currentStatus === "PENDING" ? (
            <StatePanel kind="loading" message={t.pendingMessage} title={t.pendingTitle} />
          ) : currentStatus === "RUNNING" ? (
            <StatePanel kind="loading" message={t.runningMessage} title={t.runningTitle} />
          ) : currentStatus === "FAILED" ? (
            <StatePanel kind="error" message={t.failedMessage} title={t.failedTitle} />
          ) : currentStatus === "CANCELED" ? (
            <StatePanel kind="blocked" message={t.canceledMessage} title={t.canceledTitle} />
          ) : visiblePair === null ? (
            <StatePanel
              kind="loading"
              message={refreshing ? t.validatingMessage : t.notReadyMessage}
              title={refreshing ? t.validatingTitle : t.notReadyTitle}
            />
          ) : (
            <p className="supporting-copy" role="status">
              {t.succeededMessage}
            </p>
          )}
          {cancelAcknowledged ? (
            <p className="form-result" role="status">
              {t.cancelAcknowledgedMessage}
            </p>
          ) : null}
          {pollingExhausted ? (
            <p className="form-result" role="status">
              {t.pollingExhaustedMessage}
            </p>
          ) : null}
          <div className="field-grid">
            <button
              className="quiet-action"
              disabled={refreshing}
              onClick={() => refreshRef.current?.(true)}
              type="button"
            >
              {refreshing ? t.refreshingLabel : t.manualRefreshLabel}
            </button>
            {canCancel ? (
              <button
                className="quiet-action"
                disabled={canceling}
                onClick={() => void cancelSelectedRun()}
                type="button"
              >
                {canceling ? t.cancellingLabel : t.cancelLabel}
              </button>
            ) : null}
          </div>
        </section>
      ) : statusError !== null ? (
        <section aria-label={t.statusTitle} className="product-section">
          <p className="eyebrow">{t.statusSectionLabel}</p>
          {statusPanel === null ? null : <StatePanel {...statusPanel} />}
          {run !== null ? (
            <p className="supporting-copy">{t.selectedPeriodLabel(run.start_date, run.end_date)}</p>
          ) : null}
          {cancelAcknowledged ? (
            <p className="form-result" role="status">
              {t.cancelAcknowledgedMessage}
            </p>
          ) : null}
          {canCancel ? (
            <button
              className="quiet-action"
              disabled={canceling}
              onClick={() => void cancelSelectedRun()}
              type="button"
            >
              {canceling ? t.cancellingLabel : t.cancelLabel}
            </button>
          ) : null}
          <button
            className="quiet-action"
            disabled={refreshing}
            onClick={() => refreshRef.current?.(true)}
            type="button"
          >
            {refreshing ? t.refreshingLabel : t.manualRefreshLabel}
          </button>
        </section>
      ) : null}
      {visiblePair === null ? null : (
        <OwnerBetaSimulationReport locale={locale} pair={visiblePair} />
      )}
      <OwnerBetaSimulationHistory
        error={historyError}
        hasMore={hasMore}
        loading={historyLoading}
        loadingMore={historyLoadingMore}
        locale={locale}
        onLoadMore={() => void loadMore()}
        onSelect={selectRun}
        runs={runs}
        selectedRunId={selectedRunId}
      />
    </>
  );
}
