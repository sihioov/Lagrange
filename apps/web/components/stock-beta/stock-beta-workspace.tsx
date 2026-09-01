"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { StatePanel } from "@/components/states/state-panel";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
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
  type OwnerEquityV2CoverageModel,
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2Lifecycle,
  type OwnerEquityV2MembershipListModel,
  type OwnerEquityV2MembershipModel,
  type OwnerEquityV2PolicyModel,
  type OwnerEquityV2SignalModel,
  ownerEquityV2AddBodySchema,
} from "@/lib/products/equity-signals-contracts";

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

function conditionLabel(
  condition: OwnerEquityV2SignalModel["condition"],
  t: StockBetaDictionary,
): string {
  switch (condition) {
    case "BULLISH":
      return t.bullishLabel;
    case "NEUTRAL":
      return t.neutralLabel;
    case "BEARISH":
      return t.bearishLabel;
  }
}

function conditionTone(condition: OwnerEquityV2SignalModel["condition"]): StatusTone {
  switch (condition) {
    case "BULLISH":
      return "success";
    case "NEUTRAL":
      return "neutral";
    case "BEARISH":
      return "warning";
  }
}

function lifecycleLabel(lifecycle: OwnerEquityV2Lifecycle, t: StockBetaDictionary): string {
  switch (lifecycle) {
    case "REQUESTED":
      return t.lifecycleRequested;
    case "VALIDATING":
      return t.lifecycleValidating;
    case "BACKFILLING":
      return t.lifecycleBackfilling;
    case "MATERIALIZING":
      return t.lifecycleMaterializing;
    case "READY":
      return t.lifecycleReady;
    case "INSUFFICIENT_HISTORY":
      return t.lifecycleInsufficientHistory;
    case "FAILED":
      return t.lifecycleFailed;
    case "DISABLED":
      return t.lifecycleDisabled;
  }
}

function lifecycleTone(lifecycle: OwnerEquityV2Lifecycle): StatusTone {
  switch (lifecycle) {
    case "READY":
      return "success";
    case "FAILED":
      return "error";
    case "INSUFFICIENT_HISTORY":
      return "warning";
    case "DISABLED":
      return "neutral";
    default:
      return "info";
  }
}

function formatNumber(value: number, fractionDigits = 2): string {
  return value.toLocaleString("en-US", {
    maximumFractionDigits: fractionDigits,
    minimumFractionDigits: fractionDigits,
  });
}

function formatPercent(value: number): string {
  const sign = value > 0 ? "+" : "";
  return `${sign}${formatNumber(value * 100)}%`;
}

function displayFailure(error: unknown, t: StockBetaDictionary): string {
  if (error instanceof ApiProblem) {
    return t.requestFailure(error.code);
  }
  if (error instanceof ApiContractError) {
    return t.contractFailureMessage;
  }
  return t.genericUnavailableMessage;
}

function failureCode(error: unknown): string | null {
  if (error instanceof ApiProblem) return error.code;
  if (error instanceof ApiContractError) return "CONTRACT_ERROR";
  return "UNCLASSIFIED_ERROR";
}

function coverageText(coverage: OwnerEquityV2CoverageModel, t: StockBetaDictionary): string {
  return `${t.observedCoverageLabel} ${coverage.observed_sessions} · ${t.coverageTargetLabel} ${coverage.target_observed_sessions} · ${t.minimumCoverageLabel} ${coverage.minimum_observed_sessions}`;
}

function PolicyCapacity({
  policy,
  t,
}: {
  readonly policy: OwnerEquityV2PolicyModel;
  readonly t: StockBetaDictionary;
}) {
  const items = [
    [t.policyMaxActiveLabel, policy.max_active_instruments],
    [t.activeInstrumentsLabel, policy.active_instruments],
    [t.remainingCapacityLabel, policy.remaining_capacity],
    [t.targetCoverageLabel, policy.target_observed_sessions],
    [t.minimumCoverageLabel, policy.minimum_observed_sessions],
  ] as const;
  return (
    <div className="stock-beta-policy-capacity" data-testid="stock-beta-policy-capacity">
      <div className="stock-beta-policy-capacity-heading">
        <strong>{t.capacityLabel}</strong>
        <span>{t.policyCapacityDescription}</span>
      </div>
      <dl className="stock-beta-policy-grid">
        {items.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value.toLocaleString("en-US")}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export function StockBetaPolicyNotice({ locale }: { readonly locale?: Locale | undefined } = {}) {
  const context = useLocale();
  const t = stockBetaDictionary[locale ?? context.locale];
  return (
    <aside aria-label={t.policyAriaLabel} className="warning-strip stock-beta-policy" role="note">
      <strong>{t.warningLabel}</strong>
      <p>{t.ownerOnlyPolicy}</p>
      <p>{t.vendorSnapshotPolicy}</p>
      <p>{t.originalPricePolicy}</p>
      <p>{t.nonPitPolicy}</p>
      <p>{t.activityPolicy}</p>
      <p>{t.conditionPolicy}</p>
    </aside>
  );
}

function MembershipCard({
  membership,
  pending,
  disableConfirmationOpen,
  onCancelDisable,
  onConfirmDisable,
  onRequestDisable,
  onRetry,
  t,
}: {
  readonly membership: OwnerEquityV2MembershipModel;
  readonly pending: boolean;
  readonly disableConfirmationOpen: boolean;
  readonly onCancelDisable: () => void;
  readonly onConfirmDisable: () => void;
  readonly onRequestDisable: () => void;
  readonly onRetry: () => void;
  readonly t: StockBetaDictionary;
}) {
  const confirmationRef = useRef<HTMLButtonElement>(null);
  const canRetry =
    membership.lifecycle === "INSUFFICIENT_HISTORY" ||
    (membership.lifecycle === "FAILED" && membership.failure?.retryable === true);
  const canDisable = membership.lifecycle !== "DISABLED";

  useEffect(() => {
    if (disableConfirmationOpen) confirmationRef.current?.focus();
  }, [disableConfirmationOpen]);

  return (
    <article
      aria-labelledby={`stock-beta-membership-${membership.id}`}
      className="stock-beta-membership-card"
      data-lifecycle={membership.lifecycle}
      data-testid="stock-beta-membership-card"
    >
      <header className="stock-beta-membership-heading">
        <div>
          <p className="eyebrow">{t.instrumentCodeLabel}</p>
          <h3 id={`stock-beta-membership-${membership.id}`}>{membership.instrument_id}</h3>
        </div>
        <StatusPill
          label={`${t.lifecycleLabel}: ${lifecycleLabel(membership.lifecycle, t)}`}
          tone={lifecycleTone(membership.lifecycle)}
        />
      </header>
      <dl className="stock-beta-membership-meta">
        <div>
          <dt>{t.coverageLabel}</dt>
          <dd>{coverageText(membership.coverage, t)}</dd>
        </div>
        <div>
          <dt>{t.requestedAtLabel}</dt>
          <dd>{membership.requested_at}</dd>
        </div>
        <div>
          <dt>{t.lifecycleLabel}</dt>
          <dd>{membership.updated_at}</dd>
        </div>
        {membership.coverage.first_session === undefined ? null : (
          <div>
            <dt>{t.firstSessionLabel}</dt>
            <dd>{membership.coverage.first_session}</dd>
          </div>
        )}
        {membership.coverage.last_session === undefined ? null : (
          <div>
            <dt>{t.lastSessionLabel}</dt>
            <dd>{membership.coverage.last_session}</dd>
          </div>
        )}
      </dl>
      {membership.failure === undefined ? null : (
        <p className="stock-beta-failure" role="status">
          <span>{t.failureCodeLabel}:</span> <code>{membership.failure.code}</code>
        </p>
      )}
      <div className="stock-beta-membership-actions">
        {canRetry ? (
          <button className="secondary-action" disabled={pending} onClick={onRetry} type="button">
            {pending ? t.retrying : t.retry}
          </button>
        ) : null}
        {canDisable ? (
          <button
            className="quiet-action"
            disabled={pending}
            onClick={onRequestDisable}
            type="button"
          >
            {t.disable}
          </button>
        ) : null}
        {membership.lifecycle === "READY" ? (
          <Link
            className="data-link stock-beta-detail-link"
            href={`/stock-beta/${encodeURIComponent(membership.instrument_id)}`}
          >
            {t.instrumentDetailLink}
          </Link>
        ) : null}
      </div>
      {disableConfirmationOpen ? (
        <fieldset
          aria-labelledby={`stock-beta-disable-prompt-${membership.id}`}
          className="stock-beta-disable-confirmation"
        >
          <legend>{t.disableConfirmation}</legend>
          <p id={`stock-beta-disable-prompt-${membership.id}`}>{t.disablePrompt}</p>
          <div className="inline-form">
            <button
              className="secondary-action"
              disabled={pending}
              onClick={onConfirmDisable}
              ref={confirmationRef}
              type="button"
            >
              {pending ? t.disabling : t.disableConfirmation}
            </button>
            <button
              className="quiet-action"
              disabled={pending}
              onClick={onCancelDisable}
              type="button"
            >
              {t.cancel}
            </button>
          </div>
        </fieldset>
      ) : null}
    </article>
  );
}

function SnapshotSummary({
  snapshot,
  t,
}: {
  readonly snapshot: OwnerEquityV2LatestSignalsModel["snapshot"];
  readonly t: StockBetaDictionary;
}) {
  const items = [
    [t.asOfLabel, snapshot.as_of],
    [t.snapshotRowsLabel, snapshot.row_count.toLocaleString("en-US")],
    [t.publishedAtLabel, snapshot.published_at],
    [t.universeHashLabel, snapshot.universe_sha256],
  ] as const;
  return (
    <section
      aria-labelledby="stock-beta-snapshot-heading"
      className="stock-beta-snapshot"
      data-testid="stock-beta-snapshot"
    >
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.snapshotHeading}</p>
          <h2 id="stock-beta-snapshot-heading">{t.snapshotHeading}</h2>
        </div>
        <p>{t.snapshotDescription}</p>
      </div>
      <dl className="provenance-grid">
        {items.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function SignalCard({
  row,
  t,
}: {
  readonly row: OwnerEquityV2SignalModel;
  readonly t: StockBetaDictionary;
}) {
  return (
    <article className="stock-beta-top-card">
      <div className="stock-beta-card-meta">
        <span>
          {t.rankLabel} {row.rank}
        </span>
        <StatusPill
          label={`${row.condition} · ${conditionLabel(row.condition, t)}`}
          tone={conditionTone(row.condition)}
        />
      </div>
      <h3>
        <Link href={`/stock-beta/${encodeURIComponent(row.instrument_id)}`}>
          {row.instrument_id}
        </Link>
      </h3>
      <dl className="stock-beta-card-metrics">
        <div>
          <dt>{t.scoreLabel}</dt>
          <dd>{formatNumber(row.score)}</dd>
        </div>
        <div>
          <dt>{t.return20Label}</dt>
          <dd>{formatPercent(row.return_20)}</dd>
        </div>
        <div>
          <dt>{t.activityProxyLabel}</dt>
          <dd>{formatNumber(row.average_trading_value_20)}</dd>
        </div>
      </dl>
    </article>
  );
}

function SignalTable({
  rows,
  t,
}: {
  readonly rows: readonly OwnerEquityV2SignalModel[];
  readonly t: StockBetaDictionary;
}) {
  return (
    <section
      aria-labelledby="stock-beta-ranked-table-title"
      className="data-report stock-beta-rank-report"
    >
      <header className="report-heading">
        <div>
          <p className="eyebrow">{t.signalsHeading}</p>
          <h2 id="stock-beta-ranked-table-title">{t.rankTableHeading}</h2>
          <p>{t.readySignalsDescription}</p>
        </div>
      </header>
      <div className="data-table-wrap">
        <table data-testid="stock-beta-rank-table">
          <caption>{t.rankTableCaption}</caption>
          <thead>
            <tr>
              <th scope="col">{t.rankLabel}</th>
              <th scope="col">{t.instrumentCodeLabel}</th>
              <th scope="col">{t.scoreLabel}</th>
              <th scope="col">{t.tableConditionLabel}</th>
              <th scope="col">{t.return20Label}</th>
              <th scope="col">{t.return60Label}</th>
              <th scope="col">{t.return120Label}</th>
              <th scope="col">{t.volatility20Label}</th>
              <th scope="col">{t.volatility60Label}</th>
              <th scope="col">{t.volatility120Label}</th>
              <th scope="col">{t.drawdown120Label}</th>
              <th scope="col">{t.sma20Label}</th>
              <th scope="col">{t.sma60Label}</th>
              <th scope="col">{t.averageVolumeLabel}</th>
              <th scope="col">{t.volumeRatioLabel}</th>
              <th scope="col">{t.activityProxyLabel}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.instrument_id}>
                <th scope="row">{row.rank}</th>
                <td>
                  <Link
                    className="data-link"
                    href={`/stock-beta/${encodeURIComponent(row.instrument_id)}`}
                  >
                    {row.instrument_id}
                  </Link>
                </td>
                <td className="score-emphasis">{formatNumber(row.score)}</td>
                <td>
                  <StatusPill
                    label={`${row.condition} · ${conditionLabel(row.condition, t)}`}
                    tone={conditionTone(row.condition)}
                  />
                </td>
                <td>{formatPercent(row.return_20)}</td>
                <td>{formatPercent(row.return_60)}</td>
                <td>{formatPercent(row.return_120)}</td>
                <td>{formatPercent(row.volatility_20)}</td>
                <td>{formatPercent(row.volatility_60)}</td>
                <td>{formatPercent(row.volatility_120)}</td>
                <td>{formatPercent(row.max_drawdown_120)}</td>
                <td>{formatNumber(row.sma_20)}</td>
                <td>{formatNumber(row.sma_60)}</td>
                <td>{formatNumber(row.average_volume_20)}</td>
                <td>{formatNumber(row.volume_ratio_20_60)}</td>
                <td>{formatNumber(row.average_trading_value_20)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function SignalReport({
  hasReadyMembership,
  signals,
  signalError,
  signalUnavailable,
  t,
}: {
  readonly hasReadyMembership: boolean;
  readonly signals: OwnerEquityV2LatestSignalsModel | null;
  readonly signalError: string | null;
  readonly signalUnavailable: boolean;
  readonly t: StockBetaDictionary;
}) {
  if (signals === null) {
    const error =
      signalUnavailable && hasReadyMembership
        ? {
            kind: "blocked" as const,
            message: t.signalUnavailableMessage,
            title: t.signalUnavailableTitle,
          }
        : signalError === "CONTRACT_ERROR"
          ? {
              kind: "error" as const,
              message: t.contractFailureMessage,
              title: t.genericUnavailableTitle,
            }
          : signalError === "UNCLASSIFIED_ERROR"
            ? {
                kind: "error" as const,
                message: t.genericUnavailableMessage,
                title: t.genericUnavailableTitle,
              }
            : signalError === null
              ? { kind: "empty" as const, message: t.notReadyMessage, title: t.notReadyTitle }
              : {
                  kind: "error" as const,
                  message: t.requestFailure(signalError),
                  title: t.genericUnavailableTitle,
                };
    return <StatePanel {...error} />;
  }

  const topRows = signals.top5.length > 0 ? signals.top5 : signals.rows.slice(0, 5);
  return (
    <>
      <SnapshotSummary snapshot={signals.snapshot} t={t} />
      {signals.rows.length === 0 ? (
        <StatePanel kind="empty" message={t.notReadyMessage} title={t.notReadyTitle} />
      ) : (
        <>
          <section
            aria-labelledby="stock-beta-top-five-title"
            className="data-report stock-beta-top-five"
          >
            <header className="report-heading">
              <div>
                <p className="eyebrow">{t.signalsHeading}</p>
                <h2 id="stock-beta-top-five-title">{t.signalsHeading}</h2>
                <p>{t.readySignalsDescription}</p>
              </div>
            </header>
            <div className="stock-beta-top-grid" data-testid="stock-beta-top-five">
              {topRows.map((row) => (
                <SignalCard key={row.instrument_id} row={row} t={t} />
              ))}
            </div>
          </section>
          <SignalTable rows={signals.rows} t={t} />
        </>
      )}
    </>
  );
}

export type StockBetaWorkspaceProps = {
  readonly initialMemberships: OwnerEquityV2MembershipListModel;
  readonly initialSignals: OwnerEquityV2LatestSignalsModel | null;
  readonly initialSignalUnavailable?: boolean;
  readonly locale?: Locale | undefined;
};

export function StockBetaWorkspace({
  initialMemberships,
  initialSignals,
  initialSignalUnavailable = false,
  locale,
}: StockBetaWorkspaceProps) {
  const router = useRouter();
  const context = useLocale();
  const t = stockBetaDictionary[locale ?? context.locale];
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
      if (error instanceof ApiProblem && error.code === "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE") {
        setSignals(null);
        setSignalUnavailable(true);
        setSignalError(null);
        return;
      }
      setSignals(null);
      setSignalUnavailable(false);
      setSignalError(failureCode(error));
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
        const delay = ownerEquityV2PollDelay(pollAttemptRef.current);
        pollAttemptRef.current += 1;
        timeout = setTimeout(() => void poll(), delay);
      }
    };

    const delay = ownerEquityV2PollDelay(pollAttemptRef.current);
    pollAttemptRef.current += 1;
    timeout = setTimeout(() => void poll(), delay);
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

    const scheduleNext = (): void => {
      const delay = ownerEquityV2PollDelay(signalPollAttemptRef.current);
      signalPollAttemptRef.current += 1;
      timeout = setTimeout(() => void poll(), delay);
    };
    const poll = async (): Promise<void> => {
      try {
        const next = await getOwnerEquityV2LatestSignals();
        if (cancelled) return;
        if (next.rows.some((row) => row.instrument_id === pendingSignalRemovalInstrument)) {
          scheduleNext();
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
        scheduleNext();
      }
    };

    scheduleNext();
    return () => {
      cancelled = true;
      if (timeout !== undefined) clearTimeout(timeout);
    };
  }, [pendingSignalRemovalInstrument, router]);

  async function submitAdd(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
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
      // Never leave the pre-disable snapshot visible while its exact
      // replacement is still being published.
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

  return (
    <>
      <StockBetaPolicyNotice locale={locale} />
      <section
        aria-labelledby="stock-beta-management-title"
        aria-busy={
          mutationPending || hasNonTerminalMembership || pendingSignalRemovalInstrument !== null
        }
        className="workflow-panel stock-beta-management"
      >
        <div className="section-heading">
          <div>
            <p className="eyebrow">{t.addInstrumentHeading}</p>
            <h2 id="stock-beta-management-title">{t.addInstrumentHeading}</h2>
          </div>
          <p>{t.addInstrumentDescription}</p>
        </div>
        <PolicyCapacity policy={policy} t={t} />
        <form
          className="stock-beta-add-form"
          noValidate
          onSubmit={(event) => void submitAdd(event)}
        >
          <label className="form-field" htmlFor="stock-beta-instrument-code">
            <span>{t.instrumentCodeLabel}</span>
            <input
              aria-describedby={
                inputError === null
                  ? "stock-beta-instrument-code-hint"
                  : "stock-beta-instrument-code-hint stock-beta-instrument-code-error"
              }
              aria-invalid={inputError === null ? undefined : true}
              autoComplete="off"
              disabled={mutationPending}
              id="stock-beta-instrument-code"
              inputMode="numeric"
              maxLength={6}
              pattern="[0-9]{6}"
              value={instrumentCode}
              onChange={(event) => {
                setInstrumentCode(event.target.value);
                setInputError(null);
              }}
            />
            <small id="stock-beta-instrument-code-hint">{t.instrumentCodeHint}</small>
          </label>
          <button
            className="primary-action"
            disabled={mutationPending || policy.remaining_capacity === 0}
            type="submit"
          >
            {mutationPending && pendingMembershipId === null ? t.addingInstrument : t.addInstrument}
          </button>
        </form>
        {inputError === null ? null : (
          <p className="form-result" id="stock-beta-instrument-code-error" role="alert">
            {inputError}
          </p>
        )}
        {actionError === null ? null : (
          <p className="form-result" role="alert">
            {actionError}
          </p>
        )}
        {actionMessage === null ? null : (
          <p className="form-result" role="status">
            {actionMessage}
          </p>
        )}
        {hasNonTerminalMembership || pendingSignalRemovalInstrument !== null ? (
          <p className="form-result" role="status">
            {t.pollingMessage}
          </p>
        ) : null}
        {pollError ? (
          <p className="form-result" role="status">
            {t.pollErrorMessage}
          </p>
        ) : null}
      </section>

      <section
        aria-labelledby="stock-beta-memberships-title"
        aria-busy={hasNonTerminalMembership || pendingSignalRemovalInstrument !== null}
        className="data-report stock-beta-memberships"
      >
        <header className="report-heading">
          <div>
            <p className="eyebrow">{t.capacityLabel}</p>
            <h2 id="stock-beta-memberships-title">{t.addInstrumentHeading}</h2>
          </div>
          <p>
            {t.totalMembershipsLabel}: {memberships.length}
          </p>
        </header>
        {memberships.length === 0 ? (
          <StatePanel
            kind="empty"
            message={t.emptyMembershipsMessage}
            title={t.emptyMembershipsTitle}
          />
        ) : (
          <div className="stock-beta-membership-grid" data-testid="stock-beta-memberships">
            {memberships.map((membership) => (
              <MembershipCard
                key={membership.id}
                disableConfirmationOpen={disableId === membership.id}
                membership={membership}
                onCancelDisable={() => setDisableId(null)}
                onConfirmDisable={() => void confirmDisable()}
                onRequestDisable={() => {
                  setActionError(null);
                  setActionMessage(null);
                  setDisableId(membership.id);
                }}
                onRetry={() => void retryMembership(membership.id)}
                pending={mutationPending && pendingMembershipId === membership.id}
                t={t}
              />
            ))}
          </div>
        )}
      </section>

      <section
        aria-labelledby="stock-beta-signals-title"
        aria-busy={hasNonTerminalMembership || pendingSignalRemovalInstrument !== null}
        className="stock-beta-signals"
        data-testid="stock-beta-signals"
      >
        <div className="section-heading">
          <div>
            <p className="eyebrow">{t.signalsHeading}</p>
            <h2 id="stock-beta-signals-title">{t.signalsHeading}</h2>
          </div>
        </div>
        <SignalReport
          hasReadyMembership={memberships.some((membership) => membership.lifecycle === "READY")}
          signalError={signalError}
          signalUnavailable={signalUnavailable}
          signals={signals}
          t={t}
        />
      </section>
    </>
  );
}

export function stockBetaConditionLabel(
  condition: OwnerEquityV2SignalModel["condition"],
  t: StockBetaDictionary,
): string {
  return conditionLabel(condition, t);
}

export function stockBetaConditionTone(
  condition: OwnerEquityV2SignalModel["condition"],
): StatusTone {
  return conditionTone(condition);
}

export function stockBetaFormatNumber(value: number, fractionDigits = 2): string {
  return formatNumber(value, fractionDigits);
}

export function stockBetaFormatPercent(value: number): string {
  return formatPercent(value);
}
