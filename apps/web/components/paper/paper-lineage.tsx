import type { PaperLineageModel } from "@/lib/products/paper-contracts";

export type PaperLineageProps = {
  readonly lineage: PaperLineageModel;
};

/**
 * The account's strategy-binding history and the targets each close queued.
 *
 * Binding history is immutable: switching strategies closes the old binding
 * and opens a new one, so this panel is the account's branching record —
 * execution history never mixes strategy versions.
 */
export function PaperLineage({ lineage }: PaperLineageProps) {
  return (
    <section aria-labelledby="paper-lineage-title" className="report-section">
      <h3 id="paper-lineage-title">Strategy and target lineage</h3>

      <table>
        <caption>Strategy binding history</caption>
        <thead>
          <tr>
            <th scope="col">Strategy</th>
            <th scope="col">Version</th>
            <th scope="col">Bound</th>
            <th scope="col">Unbound</th>
            <th scope="col">State</th>
          </tr>
        </thead>
        <tbody>
          {lineage.bindings.map((binding) => (
            <tr key={binding.strategy_config_id}>
              <th scope="row">{binding.strategy_id}</th>
              <td>{binding.strategy_version}</td>
              <td>{binding.bound_at}</td>
              <td>{binding.unbound_at ?? "Still bound"}</td>
              <td>{binding.active ? "Active" : "Branched"}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table>
        <caption>Session targets</caption>
        <thead>
          <tr>
            <th scope="col">Computed on</th>
            <th scope="col">Executes at</th>
            <th scope="col">Status</th>
            <th scope="col">Executed</th>
          </tr>
        </thead>
        <tbody>
          {lineage.targets.map((target) => (
            <tr key={target.id}>
              <th scope="row">{target.computed_on}</th>
              <td>{target.effective_date}</td>
              <td>{target.status}</td>
              <td>{target.executed_at ?? "Not yet"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
