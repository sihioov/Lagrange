import { StatePanel } from "@/components/states/state-panel";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import { ownerBetaSimulationDictionary } from "@/lib/i18n/dictionaries/owner-beta-simulation";
import type { Locale } from "@/lib/i18n/locale";
import { formatDate } from "@/lib/products/format";
import type { OwnerBetaSimulationVersionedRun } from "@/lib/products/owner-beta-simulation-contracts";

const STATUS_TONES = {
  CANCELED: "warning",
  FAILED: "error",
  PENDING: "neutral",
  RUNNING: "info",
  SUCCEEDED: "success",
} as const satisfies Record<OwnerBetaSimulationVersionedRun["status"], StatusTone>;

export type OwnerBetaSimulationHistoryProps = {
  readonly error: boolean;
  readonly hasMore: boolean;
  readonly loading: boolean;
  readonly loadingMore: boolean;
  readonly onLoadMore: () => void;
  readonly onSelect: (run: OwnerBetaSimulationVersionedRun) => void;
  readonly runs: readonly OwnerBetaSimulationVersionedRun[];
  readonly selectedRunId: string | null;
  readonly locale: Locale;
};

export function OwnerBetaSimulationHistory({
  error,
  hasMore,
  loading,
  loadingMore,
  onLoadMore,
  onSelect,
  runs,
  selectedRunId,
  locale,
}: OwnerBetaSimulationHistoryProps) {
  const t = ownerBetaSimulationDictionary[locale];

  return (
    <section aria-labelledby="owner-beta-simulation-history-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.historySectionLabel}</p>
          <h2 id="owner-beta-simulation-history-title">{t.historyTitle}</h2>
        </div>
        <p>{t.historyCaption}</p>
      </div>
      {loading && runs.length === 0 ? (
        <StatePanel kind="loading" message={t.historyLoadingMessage} title={t.historyTitle} />
      ) : runs.length === 0 ? (
        <StatePanel
          kind={error ? "error" : "empty"}
          message={error ? t.historyLoadError : t.noRunsMessage}
          title={error ? t.genericErrorTitle : t.noRunsTitle}
        />
      ) : (
        <div className="data-table-wrap">
          <table>
            <caption>{t.historyCaption}</caption>
            <thead>
              <tr>
                <th scope="col">{t.historyStrategyColumn}</th>
                <th scope="col">{t.startDateColumn}</th>
                <th scope="col">{t.endDateColumn}</th>
                <th scope="col">{t.statusColumn}</th>
                <th scope="col">{t.selectedRunLabel}</th>
              </tr>
            </thead>
            <tbody>
              {runs.map((run) => {
                const selected = run.id === selectedRunId;
                return (
                  <tr aria-selected={selected} key={run.id}>
                    <td>{t.strategyLabels[run.strategy_id]}</td>
                    <td>{formatDate(run.start_date)}</td>
                    <td>{formatDate(run.end_date)}</td>
                    <td>
                      <StatusPill label={run.status} tone={STATUS_TONES[run.status]} />
                    </td>
                    <td>
                      <button
                        aria-label={t.selectRunLabel(run.start_date, run.end_date, run.status)}
                        aria-pressed={selected}
                        className="quiet-action"
                        onClick={() => onSelect(run)}
                        type="button"
                      >
                        {selected ? t.selectedRunLabel : t.statusTitle}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {error && runs.length > 0 ? (
        <p className="form-result" role="alert">
          {t.loadMoreError}
        </p>
      ) : null}
      {hasMore && runs.length < 100 ? (
        <button className="quiet-action" disabled={loadingMore} onClick={onLoadMore} type="button">
          {loadingMore ? t.loadingMoreLabel : t.loadMoreLabel}
        </button>
      ) : null}
    </section>
  );
}
