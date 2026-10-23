"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { saveOwnerBetaDailyPreference } from "@/lib/api/owner-beta-daily";
import { useLocale } from "@/lib/i18n/client";
import { strategiesDictionary } from "@/lib/i18n/dictionaries/strategies";

export type StrategyDailyControlProps = {
  readonly configId: string;
  readonly enabled: boolean | undefined;
};

export function StrategyDailyControl({ configId, enabled }: StrategyDailyControlProps) {
  const router = useRouter();
  const { locale } = useLocale();
  const t = strategiesDictionary[locale];
  const [confirmed, setConfirmed] = useState(enabled);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const requestRef = useRef<AbortController | null>(null);

  useEffect(
    () => () => {
      // This signal owns UI recovery; an already-dispatched PATCH can still commit.
      requestRef.current?.abort();
      requestRef.current = null;
    },
    [],
  );

  useEffect(() => {
    setConfirmed(enabled);
    setFailed(false);
  }, [enabled]);

  async function save(): Promise<void> {
    if (requestRef.current !== null || confirmed === undefined) {
      return;
    }
    const next = !confirmed;
    const request = new AbortController();
    requestRef.current = request;
    setBusy(true);
    setFailed(false);
    try {
      const saved = await saveOwnerBetaDailyPreference(configId, next, { signal: request.signal });
      if (!request.signal.aborted) {
        setConfirmed(saved.owner_beta_daily_enabled);
        router.refresh();
      }
    } catch {
      if (!request.signal.aborted) {
        setFailed(true);
      }
    } finally {
      if (requestRef.current === request) requestRef.current = null;
      if (!request.signal.aborted) {
        setBusy(false);
      }
    }
  }

  return (
    <div className="config-form">
      <button
        aria-busy={busy}
        aria-checked={confirmed === true}
        disabled={busy || confirmed === undefined}
        onClick={() => void save()}
        role="switch"
        type="button"
      >
        {t.dailyUpdateLabel} ·{" "}
        {confirmed === undefined ? t.notReported : confirmed ? t.dailyOn : t.dailyOff}
      </button>
      <p className="supporting-copy">{t.dailyUpdateHelp}</p>
      {failed ? (
        <p className="form-result" role="alert">
          {t.dailyUpdateFailed}
        </p>
      ) : null}
    </div>
  );
}
