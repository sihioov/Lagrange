"use client";

import { useLocale } from "@/lib/i18n/client";
import { reportsDictionary } from "@/lib/i18n/dictionaries/reports";
import type { ReportProvenance } from "@/lib/products/contracts";
import { formatDate } from "@/lib/products/format";

export type ReportFooterProps = {
  readonly asOf: string | null;
  readonly licenseState: string;
  readonly provenance: ReportProvenance;
};

export function ReportFooter({ asOf, licenseState, provenance }: ReportFooterProps) {
  const { locale } = useLocale();
  const t = reportsDictionary[locale];

  return (
    <footer className="report-footer">
      <dl className="provenance-grid">
        <div>
          <dt>{t.strategyVersionLabel}</dt>
          <dd>{provenance.strategy_version}</dd>
        </div>
        <div>
          <dt>{t.dataVersionLabel}</dt>
          <dd>{provenance.data_version}</dd>
        </div>
        <div>
          <dt>{t.engineVersionLabel}</dt>
          <dd>{provenance.engine_version}</dd>
        </div>
        <div>
          <dt>{t.asOfLabel}</dt>
          <dd>{asOf === null ? t.notReported : formatDate(asOf)}</dd>
        </div>
        <div>
          <dt>{t.licenseStateLabel}</dt>
          <dd>{licenseState}</dd>
        </div>
      </dl>
      <div className="report-warnings">
        <h3>{t.warningsTitle}</h3>
        {provenance.warnings.length === 0 ? (
          <p>{t.noWarningsMessage}</p>
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
