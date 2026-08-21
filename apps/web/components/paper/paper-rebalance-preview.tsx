"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseApiResponse } from "@/lib/api/response";
import { useLocale } from "@/lib/i18n/client";
import { type PaperDictionary, paperDictionary } from "@/lib/i18n/dictionaries/paper";
import {
  type AppliedRebalancePreviewModel,
  appliedRebalancePreviewSchema,
  type RebalancePreviewDecisionModel,
  type RebalancePreviewModel,
  rebalancePreviewSchema,
} from "@/lib/products/paper-contracts";

/**
 * Bounded polling schedule for an async preview job.
 *
 * The delay grows to a plateau of 8s and the caller caps total attempts —
 * there is no unbounded loop. A preview that never reaches READY/FAILED
 * within the cap surfaces a typed "timed-out" state instead of polling
 * forever.
 */
const POLL_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
export const MAX_POLL_ATTEMPTS = 12;

export function rebalancePollDelay(attempt: number): number {
  return POLL_DELAYS_MS[Math.min(attempt, POLL_DELAYS_MS.length - 1)] ?? 8_000;
}

type PreviewState =
  | { readonly kind: "already-applied"; readonly preview: RebalancePreviewModel }
  | {
      readonly kind: "apply-error";
      readonly message: string;
      readonly preview: RebalancePreviewModel;
    }
  | { readonly kind: "applied"; readonly applied: AppliedRebalancePreviewModel }
  | { readonly kind: "applying"; readonly preview: RebalancePreviewModel }
  | { readonly kind: "creating" }
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "failed"; readonly preview: RebalancePreviewModel }
  | { readonly kind: "idle" }
  | { readonly kind: "polling"; readonly attempt: number; readonly preview: RebalancePreviewModel }
  | { readonly kind: "ready"; readonly preview: RebalancePreviewModel }
  | { readonly kind: "timed-out"; readonly preview: RebalancePreviewModel };

/**
 * The settled state a preview status maps to, or `null` while it is still
 * being computed and the caller should keep polling.
 *
 * APPLIED is terminal: the preview has already been turned into a pending
 * target and the server will never move it again. Polling it would burn the
 * whole attempt budget and then report a timeout that contradicts the status
 * it just read.
 */
export function settledPreviewState(preview: RebalancePreviewModel): PreviewState | null {
  if (preview.status === "READY") {
    return { kind: "ready", preview };
  }
  if (preview.status === "FAILED") {
    return { kind: "failed", preview };
  }
  if (preview.status === "APPLIED") {
    return { kind: "already-applied", preview };
  }
  return null;
}

export type RecommendationRunOption = {
  readonly asOf: string;
  readonly id: string;
  readonly strategyLabel: string;
};

export type PaperRebalancePreviewProps = {
  readonly accountId: string;
  readonly runs: readonly RecommendationRunOption[];
};

function decisionSkipReasonLabel(
  reason: RebalancePreviewDecisionModel["skip_reason"],
  t: PaperDictionary,
): string {
  return reason ?? t.rebalanceNoSkipReason;
}

export function RebalancePreviewDetails({
  preview,
  t,
}: {
  readonly preview: RebalancePreviewModel;
  readonly t: PaperDictionary;
}) {
  const { result } = preview;
  if (result === undefined) {
    return null;
  }
  return (
    <>
      <div
        aria-label={t.rebalanceIndicativeWarningTitle}
        className="state-panel"
        data-kind="warning"
        role="alert"
      >
        <strong>{t.rebalanceIndicativeWarningTitle}</strong>
        <p>{t.rebalanceIndicativeWarningMessage}</p>
      </div>

      <h3 id="paper-rebalance-totals-title">{t.rebalanceTotalsCaption}</h3>
      <dl aria-labelledby="paper-rebalance-totals-title" className="definition-grid">
        <dt>{t.rebalanceEquityLabel}</dt>
        <dd>{result.equity}</dd>
        <dt>{t.rebalanceCashBeforeLabel}</dt>
        <dd>{result.cash_before}</dd>
        <dt>{t.rebalanceAvailableCashLabel}</dt>
        <dd>{result.available_cash}</dd>
        <dt>{t.rebalanceLeftoverCashLabel}</dt>
        <dd>{result.leftover_cash}</dd>
        <dt>{t.rebalanceBuyNotionalLabel}</dt>
        <dd>{result.buy_notional}</dd>
        <dt>{t.rebalanceSellNotionalLabel}</dt>
        <dd>{result.sell_notional}</dd>
        <dt>{t.rebalanceExplicitFeesLabel}</dt>
        <dd>{result.explicit_fees}</dd>
        <dt>{t.columnSlippage}</dt>
        <dd>{result.informational_slippage}</dd>
      </dl>

      <table>
        <caption>{t.rebalanceDecisionsCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnInstrument}</th>
            <th scope="col">{t.columnAction}</th>
            <th scope="col">{t.columnCurrentQuantity}</th>
            <th scope="col">{t.columnCurrentValue}</th>
            <th scope="col">{t.columnCurrentWeight}</th>
            <th scope="col">{t.columnTargetValue}</th>
            <th scope="col">{t.columnTargetWeight}</th>
            <th scope="col">{t.columnDeltaValue}</th>
            <th scope="col">{t.columnSkipReason}</th>
          </tr>
        </thead>
        <tbody>
          {result.decisions.map((decision) => (
            <tr key={decision.instrument_id}>
              <th scope="row">{decision.instrument_id}</th>
              <td>{decision.action}</td>
              <td>{decision.current_quantity}</td>
              <td>{decision.current_value}</td>
              <td>{decision.current_weight}</td>
              <td>{decision.target_value}</td>
              <td>{decision.target_weight}</td>
              <td>{decision.delta_value}</td>
              <td>{decisionSkipReasonLabel(decision.skip_reason, t)}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table>
        <caption>{t.rebalanceOrdersCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnInstrument}</th>
            <th scope="col">{t.columnSide}</th>
            <th scope="col">{t.columnQuantity}</th>
            <th scope="col">{t.columnRawPrice}</th>
            <th scope="col">{t.columnEstimatedPrice}</th>
            <th scope="col">{t.columnNotional}</th>
            <th scope="col">{t.columnCommission}</th>
            <th scope="col">{t.columnTax}</th>
            <th scope="col">{t.columnSlippage}</th>
          </tr>
        </thead>
        <tbody>
          {result.orders.map((order) => (
            <tr key={`${order.instrument_id}-${order.side}-${order.quantity}`}>
              <th scope="row">{order.instrument_id}</th>
              <td>{order.side}</td>
              <td>{order.quantity}</td>
              <td>{order.raw_price}</td>
              <td>{order.estimated_execution_price}</td>
              <td>{order.notional}</td>
              <td>{order.commission}</td>
              <td>{order.tax}</td>
              <td>{order.informational_slippage}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h3 id="paper-rebalance-lineage-title">{t.rebalanceLineageCaption}</h3>
      <dl aria-labelledby="paper-rebalance-lineage-title" className="definition-grid">
        <dt>{t.rebalanceLineageRecommendationRun}</dt>
        <dd>{result.lineage.recommendation_run_id}</dd>
        <dt>{t.rebalanceLineageTargetPortfolio}</dt>
        <dd>{result.lineage.target_portfolio_id}</dd>
        <dt>{t.rebalanceLineageStrategyConfig}</dt>
        <dd>{result.lineage.strategy_config_id}</dd>
        <dt>{t.rebalanceLineageDatasetVersion}</dt>
        <dd>{result.lineage.dataset_version_id}</dd>
        <dt>{t.rebalanceLineageCuratedVersion}</dt>
        <dd>{result.lineage.curated_version}</dd>
        <dt>{t.rebalanceLineageDatasetManifest}</dt>
        <dd>{result.lineage.dataset_manifest_sha256}</dd>
        <dt>{t.rebalanceLineageAccountStateVersion}</dt>
        <dd>{result.lineage.account_state_version}</dd>
        <dt>{t.rebalanceLineageAccountStateSha}</dt>
        <dd>{result.lineage.account_state_sha256}</dd>
        <dt>{t.rebalanceLineageTargetPortfolioSha}</dt>
        <dd>{result.lineage.target_portfolio_sha256}</dd>
      </dl>
    </>
  );
}

export type RebalancePreviewOutcomeProps = {
  /** A failed apply attempt, kept beside the still-usable preview. */
  readonly applyError: string | null;
  readonly applying: boolean;
  readonly onApply: () => void;
  readonly preview: RebalancePreviewModel;
  readonly t: PaperDictionary;
};

/**
 * The result of a create/poll cycle: status, a FAILED banner when
 * applicable, the full result breakdown, and the Apply control.
 *
 * Pure and prop-driven so it can render any preview state (including one a
 * component test hands it directly) without needing to drive the polling
 * state machine to get there.
 */
export function RebalancePreviewOutcome({
  applyError,
  applying,
  onApply,
  preview,
  t,
}: RebalancePreviewOutcomeProps) {
  // `result === undefined` renders no breakdown and therefore no indicative
  // warning. Applying from there would be a one-click mutation with nothing
  // disclosed, so "a warning is shown" and "Apply is possible" are one
  // condition.
  const applyDisabled =
    preview.status !== "READY" ||
    preview.preview_token === null ||
    preview.result === undefined ||
    applying;
  return (
    <>
      <p className="supporting-copy">{t.rebalanceStatusLabel(preview.status)}</p>
      {preview.status === "FAILED" ? (
        <p className="form-result" role="alert">
          <strong>{t.rebalanceFailedTitle}</strong>
          {preview.error === undefined ? null : (
            <>
              {" "}
              — {t.rebalanceErrorCodeLabel}: {preview.error.code} — {preview.error.message}
            </>
          )}
        </p>
      ) : null}
      <RebalancePreviewDetails preview={preview} t={t} />
      <button className="primary-action" disabled={applyDisabled} onClick={onApply} type="button">
        {applying ? t.rebalanceApplyingButton : t.rebalanceApplyButton}
      </button>
      {applyError === null ? null : (
        <p className="form-result" role="alert">
          <strong>{t.rebalanceApplyFailedTitle}</strong> — {applyError}
        </p>
      )}
    </>
  );
}

export function PaperRebalancePreview({ accountId, runs }: PaperRebalancePreviewProps) {
  const router = useRouter();
  const { locale } = useLocale();
  const t = paperDictionary[locale];
  const [selectedRunId, setSelectedRunId] = useState(runs[0]?.id ?? "");
  const [state, setState] = useState<PreviewState>({ kind: "idle" });

  useEffect(() => {
    if (state.kind !== "polling") {
      return;
    }
    if (state.attempt >= MAX_POLL_ATTEMPTS) {
      setState({ kind: "timed-out", preview: state.preview });
      return;
    }
    let cancelled = false;
    const timeout = setTimeout(() => {
      void (async () => {
        try {
          const response = await fetch(
            `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/recommendation-previews/${encodeURIComponent(state.preview.id)}`,
            { cache: "no-store", credentials: "same-origin" },
          );
          const next = await parseApiResponse(response, rebalancePreviewSchema);
          if (cancelled) {
            return;
          }
          setState(
            settledPreviewState(next) ?? {
              kind: "polling",
              attempt: state.attempt + 1,
              preview: next,
            },
          );
        } catch (error) {
          if (!cancelled) {
            setState({
              kind: "error",
              message: error instanceof Error ? error.message : t.unavailableMessage,
            });
          }
        }
      })();
    }, rebalancePollDelay(state.attempt));
    return () => {
      cancelled = true;
      clearTimeout(timeout);
    };
  }, [accountId, state, t.unavailableMessage]);

  async function createPreview(): Promise<void> {
    if (selectedRunId === "") {
      return;
    }
    setState({ kind: "creating" });
    try {
      const response = await mutateWithCsrf(
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/recommendation-previews`,
        { json: { recommendation_run_id: selectedRunId }, method: "POST" },
      );
      const preview = await parseApiResponse(response, rebalancePreviewSchema);
      setState(settledPreviewState(preview) ?? { kind: "polling", attempt: 0, preview });
    } catch (error) {
      setState({
        kind: "error",
        message: error instanceof Error ? error.message : t.unavailableMessage,
      });
    }
  }

  async function applyPreview(preview: RebalancePreviewModel): Promise<void> {
    if (preview.preview_token === null) {
      return;
    }
    setState({ kind: "applying", preview });
    try {
      const response = await mutateWithCsrf(
        `/api/v1/paper/accounts/${encodeURIComponent(accountId)}/recommendation-previews/${encodeURIComponent(preview.id)}/apply`,
        { json: { preview_token: preview.preview_token }, method: "POST" },
      );
      const applied = await parseApiResponse(response, appliedRebalancePreviewSchema);
      setState({ kind: "applied", applied });
      router.refresh();
    } catch (error) {
      // Apply is idempotent on the same token, and a stale-token 409 is the
      // one outcome the reader most needs the plan on screen for. Discarding
      // the READY preview here would leave no retry and nothing to read.
      setState({
        kind: "apply-error",
        message: error instanceof Error ? error.message : t.unavailableMessage,
        preview,
      });
    }
  }

  const busy = state.kind === "creating" || state.kind === "polling" || state.kind === "applying";
  const activePreview =
    state.kind === "ready" ||
    state.kind === "failed" ||
    state.kind === "timed-out" ||
    state.kind === "applying" ||
    state.kind === "already-applied" ||
    state.kind === "apply-error"
      ? state.preview
      : null;

  return (
    <section aria-labelledby="paper-rebalance-preview-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.rebalanceEyebrow}</p>
          <h2 id="paper-rebalance-preview-title">{t.rebalanceHeading}</h2>
        </div>
      </div>

      {runs.length === 0 ? (
        <p className="supporting-copy">{t.rebalanceNoRunsMessage}</p>
      ) : (
        <form
          aria-label={t.rebalanceCreateAriaLabel}
          className="workflow-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void createPreview();
          }}
        >
          <label className="form-field">
            <span>{t.rebalanceRunLabel}</span>
            <select
              name="recommendation_run_id"
              onChange={(event) => setSelectedRunId(event.currentTarget.value)}
              required
              value={selectedRunId}
            >
              {runs.map((run) => (
                <option key={run.id} value={run.id}>
                  {run.asOf} · {run.strategyLabel} · {run.id.slice(0, 8)}
                </option>
              ))}
            </select>
          </label>
          <button className="primary-action" disabled={busy} type="submit">
            {state.kind === "creating" ? t.rebalanceCreatingButton : t.rebalanceCreateButton}
          </button>
        </form>
      )}

      {state.kind === "polling" ? (
        <p className="form-result" role="status">
          {t.rebalancePollingMessage}
        </p>
      ) : null}

      {state.kind === "timed-out" ? (
        <p className="form-result" role="alert">
          <strong>{t.rebalanceTimedOutTitle}</strong> — {t.rebalanceTimedOutMessage}
        </p>
      ) : null}

      {state.kind === "error" ? (
        <p className="form-result" role="alert">
          {state.message}
        </p>
      ) : null}

      {state.kind === "applied" ? (
        <p className="form-result" role="status">
          {t.rebalanceAppliedMessage(state.applied.effective_date)}
        </p>
      ) : null}

      {activePreview === null ? null : (
        <RebalancePreviewOutcome
          applyError={state.kind === "apply-error" ? state.message : null}
          applying={state.kind === "applying"}
          onApply={() => void applyPreview(activePreview)}
          preview={activePreview}
          t={t}
        />
      )}
    </section>
  );
}
