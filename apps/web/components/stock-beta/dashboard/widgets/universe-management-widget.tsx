"use client";

import { WidgetFrame } from "../../shared/widget-frame";
import styles from "../dashboard.module.css";
import type { StockBetaDashboardWidgetViewModel } from "../types";

export function UniverseManagementWidget({
  viewModel,
}: {
  readonly viewModel: StockBetaDashboardWidgetViewModel;
}) {
  const { copy: t, policy } = viewModel;
  const capacity = [
    [t.activeInstrumentsLabel, policy.active_instruments],
    [t.policyMaxActiveLabel, policy.max_active_instruments],
    [t.remainingCapacityLabel, policy.remaining_capacity],
    [t.targetCoverageLabel, policy.target_observed_sessions],
    [t.minimumCoverageLabel, policy.minimum_observed_sessions],
  ] as const;

  return (
    <WidgetFrame description={t.addInstrumentDescription} title={t.addInstrumentHeading}>
      <div className={styles["managementCompact"]} data-testid="stock-beta-policy-capacity">
        <form
          className={styles["addForm"]}
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void viewModel.onAdd();
          }}
        >
          <label htmlFor="stock-beta-instrument-code">
            <span>{t.instrumentCodeLabel}</span>
            <input
              aria-describedby={
                viewModel.inputError === null
                  ? "stock-beta-instrument-code-hint"
                  : "stock-beta-instrument-code-hint stock-beta-instrument-code-error"
              }
              aria-invalid={viewModel.inputError === null ? undefined : true}
              autoComplete="off"
              disabled={viewModel.mutationPending}
              id="stock-beta-instrument-code"
              inputMode="numeric"
              onChange={(event) => viewModel.onInstrumentCodeChange(event.target.value)}
              pattern="[0-9]{6}"
              value={viewModel.instrumentCode}
            />
            <small id="stock-beta-instrument-code-hint">{t.instrumentCodeHint}</small>
          </label>
          <button
            disabled={viewModel.mutationPending || policy.remaining_capacity === 0}
            type="submit"
          >
            {viewModel.mutationPending && viewModel.pendingMembershipId === null
              ? t.addingInstrument
              : t.addInstrument}
          </button>
        </form>
        <dl className={styles["capacityStrip"]}>
          {capacity.map(([label, value]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
        <div className={styles["managementMessages"]} aria-live="polite">
          {viewModel.inputError === null ? null : (
            <p id="stock-beta-instrument-code-error" role="alert">
              {viewModel.inputError}
            </p>
          )}
          {viewModel.actionError === null ? null : <p role="alert">{viewModel.actionError}</p>}
          {viewModel.actionMessage === null ? null : <p role="status">{viewModel.actionMessage}</p>}
          {viewModel.busy ? <p role="status">{t.pollingMessage}</p> : null}
          {viewModel.pollError ? <p role="status">{t.pollErrorMessage}</p> : null}
        </div>
      </div>
    </WidgetFrame>
  );
}
