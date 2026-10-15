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
  const alive = useRef(false);
  const saving = useRef(false);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  useEffect(() => {
    setConfirmed(enabled);
    setFailed(false);
  }, [enabled]);

  async function save(): Promise<void> {
    if (saving.current || confirmed === undefined) {
      return;
    }
    const next = !confirmed;
    saving.current = true;
    setBusy(true);
    setFailed(false);
    try {
      const saved = await saveOwnerBetaDailyPreference(configId, next);
      if (alive.current) {
        setConfirmed(saved.owner_beta_daily_enabled);
        router.refresh();
      }
    } catch {
      if (alive.current) {
        setFailed(true);
      }
    } finally {
      saving.current = false;
      if (alive.current) {
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
