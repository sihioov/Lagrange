"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import type { StatusTone } from "@/components/states/status-pill";
import { ApiContractError, ApiProblem } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { type StockBetaDictionary, stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import {
  addOwnerEquityV2Membership,
  disableOwnerEquityV2Membership,
  getOwnerEquityV2Chart,
  getOwnerEquityV2LatestSignals,
  getOwnerEquityV2Memberships,
  retryOwnerEquityV2Membership,
} from "@/lib/products/equity-signals-client";
import {
  assertOwnerEquityV2ChartMatchesExpectation,
  OwnerEquityV2ChartIntegrityError,
  type OwnerEquityV2ChartModel,
  type OwnerEquityV2ChartRange,
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2Lifecycle,
  type OwnerEquityV2MembershipListModel,
  type OwnerEquityV2SignalModel,
  ownerEquityV2AddBodySchema,
  ownerEquityV2ChartRangeSchema,
} from "@/lib/products/equity-signals-contracts";
import {
  StockBetaChartLoadCoordinator,
  type StockBetaChartLoadRequest,
} from "./chart-load-coordinator";
import { StockBetaInstrumentSearch } from "./dashboard/instrument-search";
import { StockBetaSelectionProvider } from "./dashboard/selection-provider";
import { StockBetaSnapshotStrip } from "./dashboard/snapshot-strip";
import { StockBetaDashboard } from "./dashboard/stock-beta-dashboard";
import type {
  StockBetaChartError,
  StockBetaChartState,
  StockBetaSignalState,
} from "./dashboard/types";
import { StockBetaPolicyNotice } from "./dashboard/widgets/policy-boundary-widget";
import { formatStockBetaNumber, formatStockBetaPercent } from "./shared/formatters";
import { StockBetaSignalRefreshCoordinator } from "./signal-refresh-coordinator";
import { StockBetaTerminalPage } from "./terminal";

const OWNER_EQUITY_V2_POLL_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
const NON_TERMINAL_LIFECYCLES = new Set<OwnerEquityV2Lifecycle>([
  "REQUESTED",
  "VALIDATING",
  "BACKFILLING",
  "MATERIALIZING",
]);
const DEFAULT_CHART_RANGE: OwnerEquityV2ChartRange = "1y";

type StockBetaChartSeed = {
  readonly chart: OwnerEquityV2ChartModel | null;
  readonly error: StockBetaChartError | null;
  readonly key: string;
};

function defaultSignal(
  signals: OwnerEquityV2LatestSignalsModel | null,
): OwnerEquityV2SignalModel | undefined {
  return signals?.top5[0] ?? signals?.rows[0];
}

function chartRequestKey(request: StockBetaChartLoadRequest): string {
  return [
    request.instrumentId,
    request.snapshotId,
    String(request.generation),
    request.range,
    request.asOf,
  ].join("\u0000");
}

function chartErrorFor(error: unknown): StockBetaChartError {
  if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_CHART_UNAVAILABLE") {
    return { code: "OWNER_EQUITY_CHART_UNAVAILABLE", kind: "unavailable" };
  }
  if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_INTEGRITY_FAILED") {
    return { code: "OWNER_EQUITY_INTEGRITY_FAILED", kind: "integrity" };
  }
  if (error instanceof OwnerEquityV2ChartIntegrityError) {
    return { code: "CHART_CONTEXT_MISMATCH", kind: "integrity" };
  }
  if (error instanceof ApiContractError) {
    return { code: "CHART_CONTRACT_INVALID", kind: "integrity" };
  }
  return { code: failureCode(error), kind: "error" };
}

function sameChartContext(
  chart: OwnerEquityV2ChartModel,
  request: StockBetaChartLoadRequest,
): boolean {
  return (
    chart.snapshot_id === request.snapshotId &&
    chart.instrument_id === request.instrumentId &&
    chart.generation === request.generation &&
    chart.as_of === request.asOf
  );
}

export function ownerEquityV2PollDelay(attempt: number): number {
  return (
    OWNER_EQUITY_V2_POLL_DELAYS_MS[
      Math.min(Math.max(attempt, 0), OWNER_EQUITY_V2_POLL_DELAYS_MS.length - 1)
    ] ?? 8_000
  );
}

function displayFailure(error: unknown, t: StockBetaDictionary): string {
  if (error instanceof ApiProblem) return t.requestFailure(error.code);
  if (error instanceof ApiContractError) return t.contractFailureMessage;
  return t.genericUnavailableMessage;
}

function failureCode(error: unknown): string {
  if (error instanceof ApiProblem) return error.code;
  if (error instanceof ApiContractError) return "CONTRACT_ERROR";
  return "UNCLASSIFIED_ERROR";
}

export type StockBetaWorkspaceProps = {
  readonly initialChart?: OwnerEquityV2ChartModel | null;
  readonly initialChartError?: StockBetaChartError | null;
  readonly initialMemberships: OwnerEquityV2MembershipListModel;
  readonly initialSignals: OwnerEquityV2LatestSignalsModel | null;
  readonly initialSignalUnavailable?: boolean;
  readonly locale?: Locale;
};

export function StockBetaWorkspace({
  initialChart = null,
  initialChartError = null,
  initialMemberships,
  initialSignals,
  initialSignalUnavailable = false,
  locale,
}: StockBetaWorkspaceProps) {
  const router = useRouter();
  const context = useLocale();
  const resolvedLocale = locale ?? context.locale;
  const t = stockBetaDictionary[resolvedLocale];
  const [policy, setPolicy] = useState(initialMemberships.policy);
  const [memberships, setMemberships] = useState(initialMemberships.memberships);
  const [signals, setSignals] = useState<OwnerEquityV2LatestSignalsModel | null>(initialSignals);
  const [selectedInstrumentId, setSelectedInstrumentId] = useState<string | null>(
    defaultSignal(initialSignals)?.instrument_id ?? null,
  );
  const [chartRange, setChartRange] = useState<OwnerEquityV2ChartRange>(DEFAULT_CHART_RANGE);
  const [chartData, setChartData] = useState<OwnerEquityV2ChartModel | null>(initialChart);
  const [chartError, setChartError] = useState<StockBetaChartError | null>(initialChartError);
  const [chartState, setChartState] = useState<StockBetaChartState>(
    initialChart !== null ? { kind: "ready" } : (initialChartError ?? { kind: "idle" }),
  );
  const [signalUnavailable, setSignalUnavailable] = useState(
    initialSignals === null && initialSignalUnavailable,
  );
  const [signalError, setSignalError] = useState<string | null>(null);
  const [instrumentCode, setInstrumentCode] = useState("");
  const [inputError, setInputError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [pollError, setPollError] = useState(false);
  const [mutationPending, setMutationPending] = useState(false);
  const [pendingMembershipId, setPendingMembershipId] = useState<string | null>(null);
  const [pendingSignalRemovalInstrument, setPendingSignalRemovalInstrument] = useState<
    string | null
  >(null);
  const [disableId, setDisableId] = useState<string | null>(null);
  const mutationPendingRef = useRef(false);
  const previousMembershipsRef = useRef(initialMemberships.memberships);
  const pollAttemptRef = useRef(0);
  const signalPollAttemptRef = useRef(0);
  const signalRefreshCoordinatorRef = useRef<
    StockBetaSignalRefreshCoordinator<OwnerEquityV2LatestSignalsModel> | undefined
  >(undefined);
  if (signalRefreshCoordinatorRef.current === undefined) {
    signalRefreshCoordinatorRef.current =
      new StockBetaSignalRefreshCoordinator<OwnerEquityV2LatestSignalsModel>();
  }
  const signalRefreshCoordinator = signalRefreshCoordinatorRef.current;
  const chartLoadCoordinatorRef = useRef<StockBetaChartLoadCoordinator | undefined>(undefined);
  if (chartLoadCoordinatorRef.current === undefined) {
    chartLoadCoordinatorRef.current = new StockBetaChartLoadCoordinator();
  }
  const chartLoadCoordinator = chartLoadCoordinatorRef.current;
  const chartSeedRef = useRef<StockBetaChartSeed | null>(null);
  if (chartSeedRef.current === null) {
    const initialSignal = defaultSignal(initialSignals);
    if (initialSignal !== undefined && (initialChart !== null || initialChartError !== null)) {
      chartSeedRef.current = {
        chart: initialChart,
        error: initialChartError,
        key: chartRequestKey({
          asOf: initialSignals?.snapshot.as_of ?? "",
          generation: initialSignal.generation,
          instrumentId: initialSignal.instrument_id,
          range: DEFAULT_CHART_RANGE,
          snapshotId: initialSignals?.snapshot.snapshot_id ?? "",
        }),
      };
    }
  }
  const rows = signals?.rows ?? [];
  const selectedSignal = rows.find((row) => row.instrument_id === selectedInstrumentId) ?? rows[0];
  const effectiveSelectedInstrumentId = selectedSignal?.instrument_id ?? null;

  const refreshSignals = useCallback(async (): Promise<void> => {
    await signalRefreshCoordinator.run(getOwnerEquityV2LatestSignals, {
      onFailure: (error) => {
        setSignals(null);
        if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
          setSignalUnavailable(true);
          setSignalError(null);
        } else {
          setSignalUnavailable(false);
          setSignalError(failureCode(error));
        }
      },
      onSuccess: (next) => {
        setSignals(next);
        setSignalUnavailable(false);
        setSignalError(null);
      },
    });
  }, [signalRefreshCoordinator]);

  const refreshMemberships = useCallback(async (): Promise<OwnerEquityV2MembershipListModel> => {
    const next = await getOwnerEquityV2Memberships();
    setPolicy(next.policy);
    setMemberships(next.memberships);
    return next;
  }, []);

  useEffect(() => {
    signalRefreshCoordinator.invalidate();
    setPolicy(initialMemberships.policy);
    setMemberships(initialMemberships.memberships);
    const acceptedInitialSignals =
      initialSignals === null || signalRefreshCoordinator.acceptsSnapshot(initialSignals)
        ? initialSignals
        : null;
    const initialSignal = defaultSignal(acceptedInitialSignals);
    let acceptedInitialChart = initialChart;
    let acceptedInitialChartError = initialChartError;
    let initialChartSeed: StockBetaChartSeed | null = null;
    if (initialSignal === undefined) {
      acceptedInitialChart = null;
      acceptedInitialChartError = null;
    } else {
      const initialChartRequest: StockBetaChartLoadRequest = {
        asOf: acceptedInitialSignals?.snapshot.as_of ?? "",
        generation: initialSignal.generation,
        instrumentId: initialSignal.instrument_id,
        range: DEFAULT_CHART_RANGE,
        snapshotId: acceptedInitialSignals?.snapshot.snapshot_id ?? "",
      };
      if (acceptedInitialChart !== null) {
        try {
          acceptedInitialChart = assertOwnerEquityV2ChartMatchesExpectation(
            acceptedInitialChart,
            initialChartRequest,
          );
        } catch {
          acceptedInitialChart = null;
          acceptedInitialChartError = {
            code: "CHART_CONTEXT_MISMATCH",
            kind: "integrity",
          };
        }
      }
      if (acceptedInitialChart !== null || acceptedInitialChartError !== null) {
        initialChartSeed = {
          chart: acceptedInitialChart,
          error: acceptedInitialChartError,
          key: chartRequestKey(initialChartRequest),
        };
      }
    }
    setSignals(acceptedInitialSignals);
    setSignalUnavailable(acceptedInitialSignals === null && initialSignalUnavailable);
    setSignalError(null);
    setSelectedInstrumentId((current) => {
      const rows = acceptedInitialSignals?.rows ?? [];
      return current !== null && rows.some((row) => row.instrument_id === current)
        ? current
        : (initialSignal?.instrument_id ?? null);
    });
    setChartData(acceptedInitialChart);
    setChartError(acceptedInitialChartError);
    setChartState(
      acceptedInitialChart !== null
        ? { kind: "ready" }
        : (acceptedInitialChartError ?? { kind: "idle" }),
    );
    chartSeedRef.current = initialChartSeed;
    previousMembershipsRef.current = initialMemberships.memberships;
  }, [
    initialChart,
    initialChartError,
    initialMemberships,
    initialSignalUnavailable,
    initialSignals,
    signalRefreshCoordinator,
  ]);

  useEffect(() => {
    if (selectedInstrumentId !== effectiveSelectedInstrumentId) {
      setSelectedInstrumentId(effectiveSelectedInstrumentId);
    }
  }, [effectiveSelectedInstrumentId, selectedInstrumentId]);

  useEffect(() => {
    if (signals === null || selectedSignal === undefined) {
      chartLoadCoordinator.invalidate();
      chartSeedRef.current = null;
      setChartData(null);
      setChartError(null);
      setChartState({ kind: "idle" });
      return;
    }

    const request: StockBetaChartLoadRequest = {
      asOf: signals.snapshot.as_of,
      generation: selectedSignal.generation,
      instrumentId: selectedSignal.instrument_id,
      range: chartRange,
      snapshotId: signals.snapshot.snapshot_id,
    };
    const requestKey = chartRequestKey(request);
    const seed = chartSeedRef.current;
    if (seed?.key === requestKey) {
      setChartData(seed.chart);
      setChartError(seed.error);
      setChartState(seed.chart !== null ? { kind: "ready" } : (seed.error ?? { kind: "idle" }));
      return;
    }
    chartSeedRef.current = null;
    setChartError(null);
    setChartState({ kind: "loading", request });
    setChartData((current) =>
      current !== null && sameChartContext(current, request) ? current : null,
    );
    void chartLoadCoordinator.run(
      request,
      (nextRequest, signal) =>
        getOwnerEquityV2Chart(nextRequest.instrumentId, nextRequest.snapshotId, nextRequest.range, {
          signal,
        }),
      {
        onFailure: (error) => {
          const nextError = chartErrorFor(error);
          setChartData(null);
          setChartError(nextError);
          setChartState(nextError);
        },
        onSuccess: (chart) => {
          setChartData(chart);
          setChartError(null);
          setChartState({ kind: "ready" });
        },
      },
    );
    return () => chartLoadCoordinator.invalidate();
  }, [chartLoadCoordinator, chartRange, selectedSignal, signals]);

  useEffect(() => {
    const previous = previousMembershipsRef.current;
    const becameReady = memberships.some(
      (membership) =>
        membership.lifecycle === "READY" &&
        previous.find((item) => item.id === membership.id)?.lifecycle !== "READY",
    );
    previousMembershipsRef.current = memberships;
    if (becameReady) {
      void refreshSignals();
      router.refresh();
    }
  }, [memberships, refreshSignals, router]);

  const hasNonTerminalMembership = memberships.some((membership) =>
    NON_TERMINAL_LIFECYCLES.has(membership.lifecycle),
  );

  useEffect(() => {
    if (!hasNonTerminalMembership) {
      pollAttemptRef.current = 0;
      return;
    }
    let cancelled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const poll = async (): Promise<void> => {
      try {
        await refreshMemberships();
        if (!cancelled) setPollError(false);
      } catch {
        if (!cancelled) setPollError(true);
      }
      if (!cancelled) {
        const delay = ownerEquityV2PollDelay(pollAttemptRef.current++);
        timeout = setTimeout(() => void poll(), delay);
      }
    };
    timeout = setTimeout(() => void poll(), ownerEquityV2PollDelay(pollAttemptRef.current++));
    return () => {
      cancelled = true;
      if (timeout !== undefined) clearTimeout(timeout);
    };
  }, [hasNonTerminalMembership, refreshMemberships]);

  useEffect(() => {
    if (pendingSignalRemovalInstrument === null) {
      signalPollAttemptRef.current = 0;
      return;
    }
    let cancelled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const schedule = (poll: () => Promise<void>) => {
      timeout = setTimeout(
        () => void poll(),
        ownerEquityV2PollDelay(signalPollAttemptRef.current++),
      );
    };
    const poll = async (): Promise<void> => {
      let shouldSchedule = false;
      const outcome = await signalRefreshCoordinator.run(getOwnerEquityV2LatestSignals, {
        onFailure: (error) => {
          if (cancelled) return;
          if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
            setSignals(null);
            setSignalUnavailable(true);
            setSignalError(null);
            setPollError(false);
            signalRefreshCoordinator.releaseInstrument(pendingSignalRemovalInstrument);
            setPendingSignalRemovalInstrument(null);
            router.refresh();
            return;
          }
          setPollError(true);
          shouldSchedule = true;
        },
        onSuccess: (next) => {
          if (cancelled) return;
          if (next.rows.some((row) => row.instrument_id === pendingSignalRemovalInstrument)) {
            shouldSchedule = true;
            return;
          }
          setSignals(next);
          setSignalUnavailable(false);
          setSignalError(null);
          setPollError(false);
          signalRefreshCoordinator.releaseInstrument(pendingSignalRemovalInstrument);
          setPendingSignalRemovalInstrument(null);
          router.refresh();
        },
      });
      if (!cancelled && (outcome === "blocked" || outcome === "stale" || shouldSchedule)) {
        schedule(poll);
      }
    };
    schedule(poll);
    return () => {
      cancelled = true;
      if (timeout !== undefined) clearTimeout(timeout);
    };
  }, [pendingSignalRemovalInstrument, router, signalRefreshCoordinator]);

  async function addMembership(): Promise<void> {
    if (mutationPendingRef.current) return;
    const parsed = ownerEquityV2AddBodySchema.safeParse({ instrument_code: instrumentCode });
    if (!parsed.success) {
      setInputError(t.invalidInstrumentCode);
      return;
    }
    mutationPendingRef.current = true;
    setMutationPending(true);
    setPendingMembershipId(null);
    setInputError(null);
    setActionError(null);
    setActionMessage(null);
    try {
      await addOwnerEquityV2Membership(parsed.data);
      setInstrumentCode("");
      setActionMessage(t.addInstrumentSuccess);
      await refreshMemberships();
    } catch (error) {
      setActionError(displayFailure(error, t));
    } finally {
      mutationPendingRef.current = false;
      setMutationPending(false);
    }
  }

  async function retryMembership(membershipId: string): Promise<void> {
    if (mutationPendingRef.current) return;
    mutationPendingRef.current = true;
    setMutationPending(true);
    setPendingMembershipId(membershipId);
    setActionError(null);
    setActionMessage(null);
    try {
      await retryOwnerEquityV2Membership(membershipId);
      setActionMessage(t.retrySuccess);
      await refreshMemberships();
    } catch (error) {
      setActionError(displayFailure(error, t));
    } finally {
      mutationPendingRef.current = false;
      setMutationPending(false);
      setPendingMembershipId(null);
    }
  }

  async function confirmDisable(): Promise<void> {
    if (disableId === null || mutationPendingRef.current) return;
    mutationPendingRef.current = true;
    setMutationPending(true);
    setPendingMembershipId(disableId);
    setActionError(null);
    setActionMessage(null);
    try {
      const result = await disableOwnerEquityV2Membership(disableId);
      setDisableId(null);
      setActionMessage(t.disableSuccess);
      signalRefreshCoordinator.blockInstrument(result.resource.instrument_id);
      setPendingSignalRemovalInstrument(result.resource.instrument_id);
      setSignals(null);
      setSignalUnavailable(false);
      setSignalError(null);
      await refreshMemberships();
    } catch (error) {
      setActionError(displayFailure(error, t));
    } finally {
      mutationPendingRef.current = false;
      setMutationPending(false);
      setPendingMembershipId(null);
    }
  }

  const onChartRangeChange = useCallback((nextRange: OwnerEquityV2ChartRange): void => {
    const parsed = ownerEquityV2ChartRangeSchema.safeParse(nextRange);
    if (parsed.success) setChartRange(parsed.data);
  }, []);

  const signalState: StockBetaSignalState =
    signals !== null
      ? { kind: "ready" }
      : signalUnavailable
        ? { kind: "unavailable" }
        : signalError === null
          ? { kind: "not-ready" }
          : { code: signalError, kind: "error" };
  const busy =
    mutationPending || hasNonTerminalMembership || pendingSignalRemovalInstrument !== null;
  const viewModel = {
    actionError,
    actionMessage,
    busy,
    chartData,
    chartError,
    chartRange,
    chartState,
    copy: t,
    disableId,
    inputError,
    instrumentCode,
    locale: resolvedLocale,
    memberships,
    mutationPending,
    onAdd: addMembership,
    onCancelDisable: () => setDisableId(null),
    onConfirmDisable: confirmDisable,
    onInstrumentCodeChange: (value: string) => {
      setInstrumentCode(value);
      setInputError(null);
    },
    onRequestDisable: (membershipId: string) => {
      setActionError(null);
      setActionMessage(null);
      setDisableId(membershipId);
    },
    onRetry: retryMembership,
    onChartRangeChange,
    pendingMembershipId,
    policy,
    pollError,
    signalState,
    signals,
    selectedInstrumentId: effectiveSelectedInstrumentId,
  } as const;

  return (
    <StockBetaSelectionProvider
      onSelectionChange={setSelectedInstrumentId}
      rows={rows}
      selectedInstrumentId={selectedInstrumentId}
    >
      <StockBetaTerminalPage
        asOf={
          signals === null ? undefined : (
            <span>
              {t.asOfLabel} <strong>{signals.snapshot.as_of}</strong>
            </span>
          )
        }
        context={<span>{t.terminalContextLabel}</span>}
        search={rows.length === 0 ? undefined : <StockBetaInstrumentSearch copy={t} rows={rows} />}
        snapshot={signals === null ? undefined : <StockBetaSnapshotStrip copy={t} data={signals} />}
        title={t.pageTitle}
      >
        <StockBetaDashboard selectionProvided viewModel={viewModel} />
      </StockBetaTerminalPage>
    </StockBetaSelectionProvider>
  );
}

export { StockBetaPolicyNotice };

export function stockBetaConditionLabel(
  condition: OwnerEquityV2SignalModel["condition"],
  t: StockBetaDictionary,
): string {
  return condition === "BULLISH"
    ? t.bullishLabel
    : condition === "BEARISH"
      ? t.bearishLabel
      : t.neutralLabel;
}

export function stockBetaConditionTone(
  condition: OwnerEquityV2SignalModel["condition"],
): StatusTone {
  return condition === "BULLISH" ? "success" : condition === "BEARISH" ? "warning" : "neutral";
}

export function stockBetaFormatNumber(value: number, fractionDigits = 2): string {
  return formatStockBetaNumber(value, "en", { fractionDigits }).text;
}
export function stockBetaFormatPercent(value: number): string {
  return formatStockBetaPercent(value, "en").text;
}
