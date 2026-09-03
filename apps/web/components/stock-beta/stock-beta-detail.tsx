import Link from "next/link";
import type { ReactNode } from "react";
import type { StockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import type { Locale } from "@/lib/i18n/locale";
import type { OwnerBetaEquitySignalsDetailModel } from "@/lib/products/equity-signals-contracts";
import styles from "./detail/detail.module.css";
import { safeStockBetaDetailBackHref } from "./detail/filter-context";
import { StockBetaDetailLayout } from "./detail/stock-beta-detail-layout";
import type { StockBetaDetailViewModel } from "./detail/types";
import { StockBetaTerminalPage } from "./terminal";

export function StockBetaDetailBackLink({
  backHref,
  t,
}: {
  readonly backHref: string;
  readonly t: StockBetaDictionary;
}) {
  return (
    <nav
      aria-label={t.backToWorkspace}
      className={styles["contextNavigation"]}
      data-testid="stock-beta-detail-context"
    >
      <Link
        className={styles["contextLink"]}
        href={safeStockBetaDetailBackHref(backHref)}
        prefetch={false}
      >
        {t.backToWorkspace}
      </Link>
    </nav>
  );
}

function DetailFact({ children, label }: { readonly children: ReactNode; readonly label: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

export function StockBetaDetailContextBar({
  backHref,
  detail,
  t,
}: {
  readonly backHref: string;
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const { signal } = detail;
  const score = String(signal.score);

  return (
    <div className={styles["detailContextBar"]} data-testid="stock-beta-detail-context-bar">
      <div className={styles["detailIdentity"]} data-testid="stock-beta-instrument-header">
        <StockBetaDetailBackLink backHref={backHref} t={t} />
        <div className={styles["detailIdentityText"]}>
          <p className={styles["eyebrow"]}>{t.detailEyebrow}</p>
          <strong>{signal.instrument_name}</strong>
          <span>{signal.instrument_id}</span>
        </div>
      </div>
      <dl className={styles["detailContextFacts"]}>
        <DetailFact label={t.rankLabel}>{String(signal.rank)}</DetailFact>
        <DetailFact label={t.scoreLabel}>
          <data data-raw-value={score} value={score}>
            {score}
          </data>
        </DetailFact>
        <DetailFact label={t.conditionLabel}>
          <span className={styles["conditionValue"]} data-condition={signal.condition}>
            {signal.condition}
          </span>
        </DetailFact>
        <DetailFact label={t.asOfLabel}>{detail.provenance.as_of}</DetailFact>
      </dl>
    </div>
  );
}

export function StockBetaDetailSnapshotStrip({
  detail,
  t,
}: {
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly t: StockBetaDictionary;
}) {
  const { signal, provenance } = detail;
  const fields = [
    { key: "as-of", label: t.asOfLabel, value: provenance.as_of },
    { key: "rank", label: t.rankLabel, value: String(signal.rank) },
    { key: "condition", label: t.conditionLabel, value: signal.condition },
    {
      key: "registration-status",
      label: t.registrationStatusLabel,
      value: provenance.registration_status,
    },
    {
      key: "publication-status",
      label: t.publicationStatusLabel,
      value: provenance.publication_status,
    },
    {
      key: "materialization-status",
      label: t.materializationStatusLabel,
      value: provenance.materialization_status,
    },
    { key: "read-only", label: t.modeLabel, value: t.readOnlyBadgeLabel },
  ] as const;

  return (
    <div className={styles["detailSnapshotScroll"]}>
      <dl className={styles["detailSnapshot"]} data-testid="stock-beta-detail-strip">
        {fields.map((field) => (
          <div data-status-key={field.key} key={field.key}>
            <dt>{field.label}</dt>
            <dd>{field.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export function StockBetaDetail({
  backHref = "/stock-beta",
  detail,
  locale = "en",
  t,
}: {
  readonly backHref?: string;
  readonly detail: OwnerBetaEquitySignalsDetailModel;
  readonly locale?: Locale;
  readonly t: StockBetaDictionary;
}) {
  const viewModel: StockBetaDetailViewModel = {
    backHref: safeStockBetaDetailBackHref(backHref),
    copy: t,
    detail,
    locale,
  };

  return (
    <StockBetaTerminalPage
      asOf={
        <span className={styles["detailAsOf"]}>
          {t.asOfLabel} {detail.provenance.as_of}
        </span>
      }
      context={<StockBetaDetailContextBar backHref={viewModel.backHref} detail={detail} t={t} />}
      snapshot={<StockBetaDetailSnapshotStrip detail={detail} t={t} />}
      title={t.detailTitle(detail.signal.instrument_id)}
    >
      <StockBetaDetailLayout viewModel={viewModel} />
    </StockBetaTerminalPage>
  );
}
