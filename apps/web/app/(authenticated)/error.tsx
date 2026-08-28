"use client";

import { StatePanel } from "@/components/states/state-panel";
import { useLocale } from "@/lib/i18n/client";
import { shellDictionary } from "@/lib/i18n/dictionaries/shell";

export type AuthenticatedErrorProps = {
  readonly reset: () => void;
};

export default function AuthenticatedError({ reset }: AuthenticatedErrorProps) {
  const { locale } = useLocale();
  const t = shellDictionary[locale];
  return (
    <StatePanel
      action={
        <>
          <button className="primary-action" onClick={reset} type="button">
            {t.tryAgain}
          </button>{" "}
          <a className="secondary-action" href="/login">
            {t.signInAgain}
          </a>
        </>
      }
      kind="error"
      message={t.errorMessage}
      title={t.errorTitle}
    />
  );
}
