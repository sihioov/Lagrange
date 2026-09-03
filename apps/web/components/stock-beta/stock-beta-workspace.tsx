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
  getOwnerEquityV2LatestSignals,
  getOwnerEquityV2Memberships,
  retryOwnerEquityV2Membership,
} from "@/lib/products/equity-signals-client";
import {
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2Lifecycle,
  type OwnerEquityV2MembershipListModel,
  type OwnerEquityV2SignalModel,
  ownerEquityV2AddBodySchema,
} from "@/lib/products/equity-signals-contracts";
import { StockBetaInstrumentSearch } from "./dashboard/instrument-search";
import { StockBetaSelectionProvider } from "./dashboard/selection-provider";
import { StockBetaSnapshotStrip } from "./dashboard/snapshot-strip";
import { StockBetaDashboard } from "./dashboard/stock-beta-dashboard";
import type { StockBetaSignalState } from "./dashboard/types";
import { StockBetaPolicyNotice } from "./dashboard/widgets/policy-boundary-widget";
import { formatStockBetaNumber, formatStockBetaPercent } from "./shared/formatters";
import { StockBetaTerminalPage } from "./terminal";

const OWNER_EQUITY_V2_POLL_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
const NON_TERMINAL_LIFECYCLES = new Set<OwnerEquityV2Lifecycle>([
  "REQUESTED",
  "VALIDATING",
  "BACKFILLING",
  "MATERIALIZING",
]);

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
  readonly initialMemberships: OwnerEquityV2MembershipListModel;
  readonly initialSignals: OwnerEquityV2LatestSignalsModel | null;
  readonly initialSignalUnavailable?: boolean;
  readonly locale?: Locale;
};

export function StockBetaWorkspace({
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

  const refreshSignals = useCallback(async (): Promise<void> => {
    try {
      const next = await getOwnerEquityV2LatestSignals();
      setSignals(next);
      setSignalUnavailable(false);
      setSignalError(null);
    } catch (error) {
      setSignals(null);
      if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
        setSignalUnavailable(true);
        setSignalError(null);
      } else {
        setSignalUnavailable(false);
        setSignalError(failureCode(error));
      }
    }
  }, []);

  const refreshMemberships = useCallback(async (): Promise<OwnerEquityV2MembershipListModel> => {
    const next = await getOwnerEquityV2Memberships();
    setPolicy(next.policy);
    setMemberships(next.memberships);
    return next;
  }, []);

  useEffect(() => {
    setPolicy(initialMemberships.policy);
    setMemberships(initialMemberships.memberships);
    setSignals(initialSignals);
    setSignalUnavailable(initialSignals === null && initialSignalUnavailable);
    setSignalError(null);
    previousMembershipsRef.current = initialMemberships.memberships;
  }, [initialMemberships, initialSignalUnavailable, initialSignals]);

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
      try {
        const next = await getOwnerEquityV2LatestSignals();
        if (cancelled) return;
        if (next.rows.some((row) => row.instrument_id === pendingSignalRemovalInstrument)) {
          schedule(poll);
          return;
        }
        setSignals(next);
        setSignalUnavailable(false);
        setSignalError(null);
        setPollError(false);
        setPendingSignalRemovalInstrument(null);
        router.refresh();
      } catch (error) {
        if (cancelled) return;
        if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
          setSignals(null);
          setSignalUnavailable(true);
          setSignalError(null);
          setPollError(false);
          setPendingSignalRemovalInstrument(null);
          router.refresh();
          return;
        }
        setPollError(true);
        schedule(poll);
      }
    };
    schedule(poll);
    return () => {
      cancelled = true;
      if (timeout !== undefined) clearTimeout(timeout);
    };
  }, [pendingSignalRemovalInstrument, router]);

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

  const signalState: StockBetaSignalState =
    signals !== null
      ? { kind: "ready" }
      : signalUnavailable
        ? { kind: "unavailable" }
        : signalError === null
          ? { kind: "not-ready" }
          : { code: signalError, kind: "error" };
  const rows = signals?.rows ?? [];
  const defaultSelectionId = signals?.top5[0]?.instrument_id ?? rows[0]?.instrument_id;
  const busy =
    mutationPending || hasNonTerminalMembership || pendingSignalRemovalInstrument !== null;
  const viewModel = {
    actionError,
    actionMessage,
    busy,
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
    pendingMembershipId,
    policy,
    pollError,
    signalState,
    signals,
  } as const;

  return (
    <StockBetaSelectionProvider
      {...(defaultSelectionId === undefined
        ? {}
        : { initialSelectedInstrumentId: defaultSelectionId })}
      rows={rows}
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
