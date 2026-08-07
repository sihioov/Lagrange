import type { ReportProvenance } from "@/lib/products/contracts";
import { formatDate } from "@/lib/products/format";

export type ReportFooterProps = {
  readonly asOf: string | null;
  readonly licenseState: string;
  readonly provenance: ReportProvenance;
};

export function ReportFooter({ asOf, licenseState, provenance }: ReportFooterProps) {
  return (
    <footer className="report-footer">
      <dl className="provenance-grid">
        <div>
          <dt>Strategy version</dt>
          <dd>{provenance.strategy_version}</dd>
        </div>
        <div>
          <dt>Data version</dt>
          <dd>{provenance.data_version}</dd>
        </div>
        <div>
          <dt>Engine version</dt>
          <dd>{provenance.engine_version}</dd>
        </div>
        <div>
          <dt>As of</dt>
          <dd>{asOf === null ? "Not reported" : formatDate(asOf)}</dd>
        </div>
        <div>
          <dt>License state</dt>
          <dd>{licenseState}</dd>
        </div>
      </dl>
      <div className="report-warnings">
        <h3>Warnings</h3>
        {provenance.warnings.length === 0 ? (
          <p>No server warnings.</p>
        ) : (
          <ul>
            {provenance.warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        )}
      </div>
    </footer>
  );
}
