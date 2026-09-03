"use client";

import Link from "next/link";
import { useEffect, useRef } from "react";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type {
  OwnerEquityV2Lifecycle,
  OwnerEquityV2MembershipModel,
} from "@/lib/products/equity-signals-contracts";
import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardCopy, StockBetaDashboardWidgetViewModel } from "../types";

function lifecycleLabel(value: OwnerEquityV2Lifecycle, t: StockBetaDashboardCopy): string {
  const labels = {
    BACKFILLING: t.lifecycleBackfilling,
    DISABLED: t.lifecycleDisabled,
    FAILED: t.lifecycleFailed,
    INSUFFICIENT_HISTORY: t.lifecycleInsufficientHistory,
    MATERIALIZING: t.lifecycleMaterializing,
    READY: t.lifecycleReady,
    REQUESTED: t.lifecycleRequested,
    VALIDATING: t.lifecycleValidating,
  } as const;
  return labels[value];
}

function lifecycleTone(value: OwnerEquityV2Lifecycle): StatusTone {
  if (value === "READY") return "success";
  if (value === "FAILED") return "error";
  if (value === "INSUFFICIENT_HISTORY") return "warning";
  if (value === "DISABLED") return "neutral";
  return "info";
}

function MembershipRow({
  membership,
  viewModel,
}: {
  readonly membership: OwnerEquityV2MembershipModel;
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t } = viewModel;
  const confirmRef = useRef<HTMLButtonElement>(null);
  const confirming = viewModel.disableId === membership.id;
  const pending = viewModel.mutationPending && viewModel.pendingMembershipId === membership.id;
  const canRetry =
    membership.lifecycle === "INSUFFICIENT_HISTORY" ||
    (membership.lifecycle === "FAILED" && membership.failure?.retryable === true);
  useEffect(() => {
    if (confirming) confirmRef.current?.focus();
  }, [confirming]);

  return (
    <li
      className={styles["membershipRow"]}
      data-lifecycle={membership.lifecycle}
      data-testid="stock-beta-membership-card"
    >
      <div className={styles["membershipIdentity"]}>
        <strong>{membership.instrument_id}</strong>
        <small>
          {t.generationLabel} {membership.generation}
        </small>
      </div>
      <StatusPill
        label={`${t.lifecycleLabel}: ${lifecycleLabel(membership.lifecycle, t)}`}
        tone={lifecycleTone(membership.lifecycle)}
      />
      <span className={styles["coverage"]}>
        {membership.coverage.observed_sessions}/{membership.coverage.target_observed_sessions} ·{" "}
        {t.minimumCoverageLabel} {membership.coverage.minimum_observed_sessions}
      </span>
      {membership.failure === undefined ? null : (
        <code title={t.failureCodeLabel}>{membership.failure.code}</code>
      )}
      <div className={styles["membershipActions"]}>
        {canRetry ? (
          <button
            disabled={pending}
            onClick={() => void viewModel.onRetry(membership.id)}
            type="button"
          >
            {pending ? t.retrying : t.retry}
          </button>
        ) : null}
        {membership.lifecycle === "READY" ? (
          <Link href={`/stock-beta/${encodeURIComponent(membership.instrument_id)}`}>
            {t.instrumentDetailLink}
          </Link>
        ) : null}
        {membership.lifecycle === "DISABLED" ? null : (
          <button
            disabled={pending}
            onClick={() => viewModel.onRequestDisable(membership.id)}
            type="button"
          >
            {t.disable}
          </button>
        )}
      </div>
      {confirming ? (
        <fieldset className={styles["disableConfirm"]} aria-label={t.disablePrompt}>
          <legend>{t.disablePrompt}</legend>
          <button
            disabled={pending}
            onClick={() => void viewModel.onConfirmDisable()}
            ref={confirmRef}
            type="button"
          >
            {pending ? t.disabling : t.disableConfirmation}
          </button>
          <button disabled={pending} onClick={viewModel.onCancelDisable} type="button">
            {t.cancel}
          </button>
        </fieldset>
      ) : null}
    </li>
  );
}

export function MembershipStatusWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, memberships } = viewModel;
  return (
    <WidgetFrame
      status={
        <span>
          {t.totalMembershipsLabel}: {memberships.length}
        </span>
      }
      title={t.membershipStatusHeading}
    >
      {memberships.length === 0 ? (
        <p className={styles["membershipEmpty"]}>{t.emptyMembershipsMessage}</p>
      ) : (
        <ul className={styles["membershipList"]} data-testid="stock-beta-memberships">
          {memberships.map((membership) => (
            <MembershipRow key={membership.id} membership={membership} viewModel={viewModel} />
          ))}
        </ul>
      )}
    </WidgetFrame>
  );
}
