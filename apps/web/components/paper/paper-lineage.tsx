import type { PaperDictionary } from "@/lib/i18n/dictionaries/paper";
import type { PaperLineageModel } from "@/lib/products/paper-contracts";

export type PaperLineageProps = {
  readonly lineage: PaperLineageModel;
  readonly t: PaperDictionary;
};

/**
 * The account's strategy-binding history and the targets each close queued.
 *
 * Binding history is immutable: switching strategies closes the old binding
 * and opens a new one, so this panel is the account's branching record —
 * execution history never mixes strategy versions.
 */
export function PaperLineage({ lineage, t }: PaperLineageProps) {
  return (
    <section aria-labelledby="paper-lineage-title" className="report-section">
      <h3 id="paper-lineage-title">{t.lineageTitle}</h3>

      <table>
        <caption>{t.bindingHistoryCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnStrategy}</th>
            <th scope="col">{t.columnVersion}</th>
            <th scope="col">{t.columnBound}</th>
            <th scope="col">{t.columnUnbound}</th>
            <th scope="col">{t.columnState}</th>
          </tr>
        </thead>
        <tbody>
          {lineage.bindings.map((binding) => (
            <tr key={binding.strategy_config_id}>
              <th scope="row">{binding.strategy_id}</th>
              <td>{binding.strategy_version}</td>
              <td>{binding.bound_at}</td>
              <td>{binding.unbound_at ?? t.stillBound}</td>
              <td>{binding.active ? t.activeState : t.branchedState}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table>
        <caption>{t.sessionTargetsCaption}</caption>
        <thead>
          <tr>
            <th scope="col">{t.columnComputedOn}</th>
            <th scope="col">{t.columnExecutesAt}</th>
            <th scope="col">{t.statusLabel}</th>
            <th scope="col">{t.columnExecuted}</th>
          </tr>
        </thead>
        <tbody>
          {lineage.targets.map((target) => (
            <tr key={target.id}>
              <th scope="row">{target.computed_on}</th>
              <td>{target.effective_date}</td>
              <td>{target.status}</td>
              <td>{target.executed_at ?? t.notYet}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
