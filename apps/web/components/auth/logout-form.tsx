"use client";

import { useState } from "react";
import { logout } from "@/lib/api/browser-client";
import { useLocale } from "@/lib/i18n/client";
import { shellDictionary } from "@/lib/i18n/dictionaries/shell";

type LogoutState = "idle" | "submitting" | "error";

export function LogoutForm() {
  const [state, setState] = useState<LogoutState>("idle");
  const { locale } = useLocale();
  const t = shellDictionary[locale];

  async function submitLogout(): Promise<void> {
    setState("submitting");
    try {
      const response = await logout();
      if (response.ok) {
        window.location.assign("/login");
        return;
      }
      setState("error");
    } catch (error) {
      if (error instanceof Error) {
        setState("error");
        return;
      }
      throw error;
    }
  }

  return (
    <form
      aria-label={t.signOut}
      className="shell-signout"
      onSubmit={(event) => {
        event.preventDefault();
        void submitLogout();
      }}
    >
      <button disabled={state === "submitting"} type="submit">
        {state === "submitting" ? t.signingOut : t.signOut}
      </button>
      {state === "error" ? <p role="alert">{t.signOutFailed}</p> : null}
    </form>
  );
}
