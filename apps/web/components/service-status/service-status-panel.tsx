"use client";

import {
  ActivityIcon,
  CaretDownIcon,
  CheckCircleIcon,
  ClockIcon,
  MinusCircleIcon,
  QuestionIcon,
  WarningCircleIcon,
  XCircleIcon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useSyncExternalStore } from "react";
import { serviceStatusDictionary } from "@/lib/i18n/dictionaries/service-status";
import type { Locale } from "@/lib/i18n/locale";
import { requestServiceStatus } from "@/lib/operations/service-status-client";
import {
  type ServiceStatusId,
  type ServiceStatusMetrics,
  type ServiceStatusReason,
  type ServiceStatusReasonGroup,
  type ServiceStatusState,
  serviceStatusContributesToOverall,
  serviceStatusReasonGroup,
} from "@/lib/operations/service-status-contract";
import {
  ServiceStatusController,
  type ServiceStatusSnapshot,
} from "@/lib/operations/service-status-controller";
import styles from "./service-status-panel.module.css";

type MetricKey = keyof ServiceStatusMetrics;

const METRIC_KEYS_BY_ID: Readonly<Partial<Record<ServiceStatusId, readonly MetricKey[]>>> = {
  "owner-equity-v2-runner": [
    "last_success_at",
    "last_failure_at",
    "data_due_at",
    "expected_session",
    "v2_data_as_of",
    "v2_snapshot_as_of",
    "queued",
    "running",
    "recent_failed",
    "expired_claims",
    "required_identities",
    "covered_identities",
  ],
  "owner-beta-runner": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "queued",
    "running",
    "recent_failed",
    "expired_claims",
  ],
  "candidate-runner": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "data_as_of",
    "queued",
    "running",
    "recent_failed",
    "expired_claims",
  ],
  "recommendation-runner": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "queued",
    "running",
    "recent_failed",
    "expired_claims",
  ],
  "nt-backtest-worker-1": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "queued",
    "running",
    "recent_failed",
    "expired_claims",
  ],
  "nt-backtest-worker-2": ["last_attempt_at", "last_success_at", "last_failure_at"],
  "research-worker": ["last_attempt_at", "last_success_at", "last_failure_at"],
  "kis-eod": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "next_run_at",
    "data_due_at",
    "data_as_of",
    "expected_session",
    "raw_as_of",
    "required_identities",
    "covered_identities",
  ],
  "market-stream": [
    "last_received_at",
    "oldest_provider_event_at",
    "active_demand",
    "live_identities",
    "halted_identities",
    "last_state_change_at",
    "calendar_status",
    "window_status",
  ],
  "intraday-calendar": ["expected_session", "calendar_status", "window_status"],
  "production-backup": ["last_attempt_at", "last_success_at", "last_failure_at", "next_run_at"],
  "production-backup-verify": [
    "last_attempt_at",
    "last_success_at",
    "last_failure_at",
    "next_run_at",
  ],
  "tls-renewal": ["last_attempt_at", "last_success_at", "last_failure_at", "next_run_at"],
};

const STATUS_ICONS: Readonly<Record<ServiceStatusState, typeof CheckCircleIcon>> = {
  healthy: CheckCircleIcon,
  waiting: ClockIcon,
  degraded: WarningCircleIcon,
  failed: XCircleIcon,
  unknown: QuestionIcon,
  disabled: MinusCircleIcon,
};

const OBSERVATION_TIME_KEYS = new Set<MetricKey>([
  "last_attempt_at",
  "last_success_at",
  "last_failure_at",
  "last_received_at",
  "oldest_provider_event_at",
  "next_run_at",
  "data_due_at",
  "last_state_change_at",
]);

const DATE_ONLY_KEYS = new Set<MetricKey>([
  "data_as_of",
  "expected_session",
  "raw_as_of",
  "v2_data_as_of",
  "v2_snapshot_as_of",
]);

function formatKstTimestamp(value: string, locale: Locale): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "";
  const formatted = new Intl.DateTimeFormat(locale === "ko" ? "ko-KR" : "en-GB", {
    timeZone: "Asia/Seoul",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(date);
  return `${formatted} KST`;
}

function relativeAge(value: string, snapshot: ServiceStatusSnapshot, locale: Locale): string {
  const observed = Date.parse(value);
  const generated =
    snapshot.lastRecordedAt === null ? Number.NaN : Date.parse(snapshot.lastRecordedAt);
  if (!Number.isFinite(observed) || !Number.isFinite(generated)) return "";
  const elapsed =
    snapshot.receivedAtMono === null ? 0 : Math.max(0, snapshot.nowMono - snapshot.receivedAtMono);
  const ageMs = Math.max(0, generated - observed + elapsed);
  const formatter = new Intl.RelativeTimeFormat(locale === "ko" ? "ko-KR" : "en-GB", {
    numeric: "auto",
  });
  if (ageMs < 10_000) return locale === "ko" ? "방금" : "just now";
  if (ageMs < 60_000) return formatter.format(-Math.floor(ageMs / 1_000), "second");
  if (ageMs < 3_600_000) return formatter.format(-Math.floor(ageMs / 60_000), "minute");
  if (ageMs < 86_400_000) return formatter.format(-Math.floor(ageMs / 3_600_000), "hour");
  return formatter.format(-Math.floor(ageMs / 86_400_000), "day");
}

function selectedMetricKeys(id: ServiceStatusId): readonly MetricKey[] {
  return METRIC_KEYS_BY_ID[id] ?? [];
}

function formatMetricValue(
  key: MetricKey,
  value: ServiceStatusMetrics[MetricKey],
  locale: Locale,
): string {
  const t = serviceStatusDictionary[locale];
  if (typeof value === "number") {
    return new Intl.NumberFormat(locale === "ko" ? "ko-KR" : "en-US").format(value);
  }
  if (typeof value !== "string") return "";
  if (key === "calendar_status") {
    return Object.entries(t.calendarStates).find(([state]) => state === value)?.[1] ?? "";
  }
  if (key === "window_status") {
    return Object.entries(t.windowStates).find(([state]) => state === value)?.[1] ?? "";
  }
  if (DATE_ONLY_KEYS.has(key)) return value;
  if (OBSERVATION_TIME_KEYS.has(key)) return formatKstTimestamp(value, locale);
  return "";
}

function metricKeysForRow(
  id: ServiceStatusId,
  metrics: ServiceStatusMetrics,
): readonly MetricKey[] {
  const selected = selectedMetricKeys(id);
  return selected.filter((key) => metrics[key] !== null && metrics[key] !== "not_applicable");
}

function reasonGroupCopy(
  state: ServiceStatusState,
  reasons: readonly ServiceStatusReason[],
  locale: Locale,
): readonly string[] {
  if (state !== "failed" && state !== "degraded" && state !== "unknown" && state !== "disabled") {
    return [];
  }
  const t = serviceStatusDictionary[locale];
  const groups = new Set<ServiceStatusReasonGroup>();
  for (const reason of reasons) {
    groups.add(serviceStatusReasonGroup(reason));
  }
  return [...groups].map((group) => t.reasonGroups[group]);
}

function StatusIcon({ status }: { readonly status: ServiceStatusState }) {
  const Icon = STATUS_ICONS[status];
  return <Icon aria-hidden="true" className={styles["statusIcon"]} size={15} weight="fill" />;
}

function StatusBadge({
  label,
  status,
}: {
  readonly label: string;
  readonly status: ServiceStatusState;
}) {
  return (
    <span className={`${styles["statusBadge"]} ${styles[`status-${status}`]}`}>
      <StatusIcon status={status} />
      <span>{label}</span>
    </span>
  );
}

function MetricList({
  id,
  locale,
  metrics,
}: {
  readonly id: ServiceStatusId;
  readonly locale: Locale;
  readonly metrics: ServiceStatusMetrics;
}) {
  const t = serviceStatusDictionary[locale];
  const keys = metricKeysForRow(id, metrics);
  if (keys.length === 0) return <span className={styles["noMetrics"]}>{t.noMetrics}</span>;

  return (
    <dl className={styles["metrics"]}>
      {keys.map((key) => {
        const value = formatMetricValue(key, metrics[key], locale);
        if (value === "") return null;
        return (
          <div className={styles["metric"]} key={key}>
            <dt>{t.metricLabels[key]}</dt>
            <dd>{value}</dd>
          </div>
        );
      })}
      {metrics.queue_scope === "actor_shared" ? (
        <div className={styles["metricNote"]}>
          <dt>{t.metricLabels.queue_scope}</dt>
          <dd>{t.sharedQueue}</dd>
        </div>
      ) : null}
    </dl>
  );
}

function SummaryCounts({
  snapshot,
  locale,
}: {
  readonly snapshot: ServiceStatusSnapshot;
  readonly locale: Locale;
}) {
  const t = serviceStatusDictionary[locale];
  const total = Object.values(snapshot.counts).reduce((sum, count) => sum + count, 0);
  if (total === 0) {
    const message =
      snapshot.issue === "denied"
        ? t.denied
        : snapshot.issue === "rate_limited"
          ? t.rateLimited
          : snapshot.issue === "unavailable"
            ? t.unavailable
            : t.loading;
    return <span className={styles["summaryMessage"]}>{message}</span>;
  }
  return (
    <span className={styles["summaryCounts"]}>
      {(["healthy", "waiting", "degraded", "failed", "unknown", "disabled"] as const)
        .filter((status) => snapshot.counts[status] > 0)
        .map((status) => (
          <span className={styles["summaryCount"]} key={status}>
            <span
              aria-hidden="true"
              className={`${styles["countDot"]} ${styles[`status-${status}`]}`}
            />
            <span>{snapshot.counts[status]}</span>
            <span>{t.statuses[status]}</span>
          </span>
        ))}
    </span>
  );
}

function summaryAnnouncement(snapshot: ServiceStatusSnapshot, locale: Locale): string {
  const t = serviceStatusDictionary[locale];
  const statuses = ["healthy", "waiting", "degraded", "failed", "unknown", "disabled"] as const;
  const total = Object.values(snapshot.counts).reduce((sum, count) => sum + count, 0);
  const breakdown = statuses
    .filter((status) => snapshot.counts[status] > 0)
    .map((status) => `${snapshot.counts[status]} ${t.statuses[status]}`);
  return [
    t.heading,
    t.statuses[snapshot.overallStatus],
    t.countSummary(total),
    ...breakdown,
    t.disclosureLabel,
  ].join(". ");
}

export function ServiceStatusPanel({ locale }: { readonly locale: Locale }) {
  const controller = useMemo(() => new ServiceStatusController(requestServiceStatus), []);
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getServerSnapshot,
  );

  useEffect(() => {
    const updateActivity = () => {
      const visible = document.visibilityState !== "hidden";
      const online = typeof navigator === "undefined" || navigator.onLine !== false;
      controller.setActive(visible && online);
    };
    controller.start();
    document.addEventListener("visibilitychange", updateActivity);
    window.addEventListener("online", updateActivity);
    window.addEventListener("offline", updateActivity);
    updateActivity();
    return () => {
      document.removeEventListener("visibilitychange", updateActivity);
      window.removeEventListener("online", updateActivity);
      window.removeEventListener("offline", updateActivity);
      controller.stop();
    };
  }, [controller]);

  return <ServiceStatusPanelView locale={locale} snapshot={snapshot} />;
}

export function ServiceStatusPanelView({
  locale,
  snapshot,
}: {
  readonly locale: Locale;
  readonly snapshot: ServiceStatusSnapshot;
}) {
  const t = serviceStatusDictionary[locale];

  const recordedAt =
    snapshot.lastRecordedAt === null ? null : formatKstTimestamp(snapshot.lastRecordedAt, locale);

  return (
    <section className={styles["panel"]} data-testid="service-status-panel">
      <details className={styles["disclosure"]}>
        <summary aria-label={summaryAnnouncement(snapshot, locale)} className={styles["summary"]}>
          <span className={styles["summaryTitle"]}>
            <ActivityIcon aria-hidden="true" size={17} weight="bold" />
            <span>{t.heading}</span>
          </span>
          <span className={styles["overall"]} role="status">
            <StatusBadge
              label={t.statuses[snapshot.overallStatus]}
              status={snapshot.overallStatus}
            />
          </span>
          <SummaryCounts locale={locale} snapshot={snapshot} />
          {recordedAt === null ? null : (
            <span className={styles["recordedAt"]}>
              <span>{snapshot.issue === null ? t.observedLabel : t.lastRecorded}</span>
              <time dateTime={snapshot.lastRecordedAt ?? undefined}>{recordedAt}</time>
              {snapshot.receivedAtMono === null ? null : (
                <span className={styles["relativeAge"]}>
                  {relativeAge(snapshot.lastRecordedAt ?? "", snapshot, locale)}
                </span>
              )}
            </span>
          )}
          <CaretDownIcon aria-hidden="true" className={styles["disclosureIcon"]} size={15} />
        </summary>

        <div className={styles["content"]}>
          {snapshot.issue === "unavailable" || snapshot.issue === "rate_limited" ? (
            <p className={styles["transportMessage"]} role="status">
              {snapshot.issue === "rate_limited" ? t.rateLimited : t.unavailable}
            </p>
          ) : null}
          {snapshot.issue === "denied" ? (
            <p className={styles["transportMessage"]} role="status">
              {t.denied}
            </p>
          ) : null}
          {snapshot.rows.length === 0 ? (
            <p className={styles["emptyMessage"]} role="status">
              {snapshot.issue === "loading" ? t.loading : t.noDetails}
            </p>
          ) : (
            <>
              {snapshot.policyState === null ? null : (
                <p className={styles["policyState"]}>
                  <span>{t.policyLabel}</span>
                  <strong>
                    {snapshot.policyState === "verified"
                      ? t.policyVerified
                      : snapshot.policyState === "mismatch"
                        ? t.policyMismatch
                        : t.policyUnavailable}
                  </strong>
                </p>
              )}
              <div className={styles["tableWrap"]}>
                <table className={styles["table"]}>
                  <caption>{t.tableCaption}</caption>
                  <thead>
                    <tr>
                      <th scope="col">{t.serviceHeading}</th>
                      <th scope="col">{t.statusHeading}</th>
                      <th scope="col">{t.processHeading}</th>
                      <th scope="col">{t.workHeading}</th>
                      <th scope="col">{t.detailsHeading}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {snapshot.rows.map((row) => {
                      const service = row.service;
                      const reasonCopy = reasonGroupCopy(row.status, row.reasonCodes, locale);
                      return (
                        <tr key={service.id}>
                          <th className={styles["serviceCell"]} scope="row">
                            <span className={styles["serviceName"]}>{t.names[service.id]}</span>
                            <span className={styles["serviceDescription"]}>
                              {t.descriptions[service.id]}
                            </span>
                            <span className={styles["serviceMode"]}>
                              {t.expectedModes[service.expected_mode]}
                            </span>
                          </th>
                          <td>
                            <span className={styles["mobileLabel"]}>{t.statusHeading}</span>
                            <StatusBadge label={t.statuses[row.status]} status={row.status} />
                            {row.expired ? (
                              <span className={styles["staleEvidence"]}>{t.staleEvidence}</span>
                            ) : null}
                          </td>
                          <td>
                            <span className={styles["mobileLabel"]}>{t.processHeading}</span>
                            <span
                              className={`${styles["stateText"]} ${styles[`process-${row.processStatus}`]}`}
                            >
                              {t.processes[row.processStatus]}
                            </span>
                          </td>
                          <td>
                            <span className={styles["mobileLabel"]}>{t.workHeading}</span>
                            <span
                              className={`${styles["stateText"]} ${styles[`work-${row.workStatus}`]}`}
                            >
                              {t.work[row.workStatus]}
                            </span>
                          </td>
                          <td className={styles["detailsCell"]}>
                            <span className={styles["mobileLabel"]}>{t.detailsHeading}</span>
                            <p className={styles["observed"]}>
                              <span>{t.observedAt}</span>
                              {service.observed_at === null ? (
                                <span>{t.statuses.unknown}</span>
                              ) : (
                                <time dateTime={service.observed_at}>
                                  {formatKstTimestamp(service.observed_at, locale)}
                                  <span className={styles["relativeAge"]}>
                                    {relativeAge(service.observed_at, snapshot, locale)}
                                  </span>
                                </time>
                              )}
                            </p>
                            {reasonCopy.length === 0 ? null : (
                              <ul className={styles["reasonList"]}>
                                {reasonCopy.map((reason) => (
                                  <li key={reason}>{reason}</li>
                                ))}
                              </ul>
                            )}
                            {serviceStatusContributesToOverall(service.id) ? null : (
                              <span className={styles["supportingRow"]}>{t.kinds.execution}</span>
                            )}
                            <MetricList id={service.id} locale={locale} metrics={service.metrics} />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </div>
      </details>
    </section>
  );
}
