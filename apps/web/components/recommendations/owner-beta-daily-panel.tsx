"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import { fetchOwnerBetaDailyState } from "@/lib/api/owner-beta-daily";
import { useLocale } from "@/lib/i18n/client";
import { recommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { formatDate, formatTimestamp } from "@/lib/products/format";
import type {
  OwnerBetaDailyStateCode,
  OwnerBetaDailyStateModel,
} from "@/lib/products/owner-beta-daily-contracts";

const TONES = {
  DISABLED: "neutral",
  UNVERIFIABLE: "warning",
  INPUT_PENDING: "info",
  INPUT_BLOCKED: "error",
  QUEUED: "info",
  RUNNING: "info",
  RETRYING: "warning",
  FAILED: "error",
  CURRENT: "success",
  STALE: "warning",
} as const satisfies Record<OwnerBetaDailyStateCode, StatusTone>;

export type OwnerBetaDailyPanelProps = {
  readonly configId: string;
  readonly initialState: OwnerBetaDailyStateModel | null;
};

export function OwnerBetaDailyPanel({ configId, initialState }: OwnerBetaDailyPanelProps) {
  const router = useRouter();
  const { locale } = useLocale();
  const t = recommendationsDictionary[locale];
  const [value, setValue] = useState(initialState);
  // Server-rendered dates are an observation; refresh before presenting CURRENT.
  const [verified, setVerified] = useState(false);
  const [busy, setBusy] = useState(false);
  const refreshRef = useRef<(() => void) | null>(null);
  const refreshedIdentity = useRef(
    JSON.stringify([initialState?.current_reservation_run_id, initialState?.published_at]),
  );

  useEffect(() => {
    let active = true;
    let pending: AbortController | null = null;
    setValue(initialState);
    setVerified(false);

    const refresh = async (): Promise<void> => {
      if (!active || document.hidden || pending !== null) {
        return;
      }
      const controller = new AbortController();
      pending = controller;
      setVerified(false);
      setBusy(true);
      const timeout = window.setTimeout(() => controller.abort(), 10_000);
      try {
        const next = await fetchOwnerBetaDailyState(configId, { signal: controller.signal });
        if (active && pending === controller && !controller.signal.aborted && !document.hidden) {
          setValue(next);
          setVerified(true);
          const identity = JSON.stringify([next.current_reservation_run_id, next.published_at]);
          if (refreshedIdentity.current !== identity) {
            refreshedIdentity.current = identity;
            router.refresh();
          }
        }
      } catch {
        if (active && pending === controller) {
          setVerified(false);
        }
      } finally {
        window.clearTimeout(timeout);
        if (active && pending === controller) {
          pending = null;
          setBusy(false);
        }
      }
    };
    const onVisibility = (): void => {
      setVerified(false);
      if (document.hidden) {
        const old = pending;
        pending = null;
        old?.abort();
        setBusy(false);
      } else {
        void refresh();
      }
    };
    refreshRef.current = () => void refresh();
    void refresh();
    const interval = window.setInterval(() => void refresh(), 30_000);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      active = false;
      refreshRef.current = null;
      pending?.abort();
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [configId, initialState, router]);

  const unavailable = !verified || value === null;
  const date = (v: string | null | undefined): string =>
    v === null || v === undefined ? t.notReported : formatDate(v);
  const timestamp = (v: string | null | undefined): string =>
    v === null || v === undefined ? t.notReported : formatTimestamp(v);

  return (
    <section aria-labelledby="owner-beta-daily-title" className="workflow-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.dailyEyebrow}</p>
          <h2 id="owner-beta-daily-title">{t.dailyHeading}</h2>
        </div>
        <StatusPill
          label={unavailable ? t.dailyUnverified : t.dailyStates[value.state]}
          tone={unavailable ? "warning" : TONES[value.state]}
        />
      </div>
      <p className="supporting-copy">{t.dailyDatesHelp}</p>
      {unavailable ? <p role="status">{t.dailyLastObservation}</p> : null}
      {verified && value !== null && value.reason_code !== null ? (
        <p role="status">{t.dailyReasons[value.reason_code]}</p>
      ) : null}
      <dl className="provenance-grid">
        <div>
          <dt>{t.dailyExpectedSession}</dt>
          <dd>{date(value?.expected_session)}</dd>
        </div>
        <div>
          <dt>{t.dailySupportEnd}</dt>
          <dd>{date(value?.data_support_end)}</dd>
        </div>
        <div>
          <dt>{t.dailyResultAsOf}</dt>
          <dd>{date(value?.recommendation_as_of)}</dd>
        </div>
        <div>
          <dt>{t.dailyPublishedAt}</dt>
          <dd>{timestamp(value?.published_at)}</dd>
        </div>
        <div>
          <dt>{t.dailyDueAt}</dt>
          <dd>{timestamp(value?.data_due_at)}</dd>
        </div>
        <div>
          <dt>{t.dailyObservedAt}</dt>
          <dd>{timestamp(value?.observed_at)}</dd>
        </div>
      </dl>
      {value?.current_reservation_run_id === null ||
      value?.current_reservation_run_id === undefined ? null : (
        <Link
          href={`/recommendations?run_id=${encodeURIComponent(value.current_reservation_run_id)}&daily_config_id=${encodeURIComponent(configId)}`}
        >
          {t.dailyReservedRun}
        </Link>
      )}
      <button aria-busy={busy} disabled={busy} onClick={() => refreshRef.current?.()} type="button">
        {t.dailyRefresh}
      </button>
    </section>
  );
}
