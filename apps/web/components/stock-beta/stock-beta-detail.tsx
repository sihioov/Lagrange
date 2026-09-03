import Link from "next/link";
import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerEquityV2SignalDetailModel } from "@/lib/products/equity-signals-contracts";
import styles from "./detail/detail.module.css";
import { StockBetaDetailLayout } from "./detail/stock-beta-detail-layout";
import { StockBetaTerminalPage } from "./terminal";

export function StockBetaDetailBackLink({
  backHref,
  t,
}: {
  readonly backHref: string;
  readonly t: StockBetaDictionary;
}) {
  return (
    <Link className={styles["backLink"]} href={backHref}>
      ← {t.backToWorkspace}
    </Link>
  );
}

export function StockBetaDetail({
  backHref = "/stock-beta",
  detail,
  locale,
  t,
}: {
  readonly backHref?: string;
  readonly detail: OwnerEquityV2SignalDetailModel;
  readonly locale?: Locale;
  readonly t: StockBetaDictionary;
}) {
  const resolvedLocale = locale ?? "en";
  const viewModel = { backHref, copy: t, detail, locale: resolvedLocale } as const;
  return (
    <StockBetaTerminalPage
      asOf={
        <span>
          {t.asOfLabel} <strong>{detail.snapshot.as_of}</strong>
        </span>
      }
      context={<StockBetaDetailBackLink backHref={backHref} t={t} />}
      snapshot={
        <dl className={styles["detailSnapshotStrip"]}>
          <div>
            <dt>{t.instrumentCodeLabel}</dt>
            <dd>{detail.signal.instrument_id}</dd>
          </div>
          <div>
            <dt>{t.generationLabel}</dt>
            <dd>{detail.signal.generation}</dd>
          </div>
          <div>
            <dt>{t.rankLabel}</dt>
            <dd>{detail.signal.rank}</dd>
          </div>
          <div>
            <dt>{t.conditionLabel}</dt>
            <dd>{detail.signal.condition}</dd>
          </div>
          <div>
            <dt>{t.snapshotIdLabel}</dt>
            <dd>{detail.snapshot.snapshot_id}</dd>
          </div>
        </dl>
      }
      title={t.detailTitle(detail.signal.instrument_id)}
    >
      <StockBetaDetailLayout viewModel={viewModel} />
    </StockBetaTerminalPage>
  );
}
