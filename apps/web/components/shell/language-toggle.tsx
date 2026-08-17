"use client";

import { useRouter } from "next/navigation";
import { useLocale } from "@/lib/i18n/client";
import { LOCALE_COOKIE, type Locale } from "@/lib/i18n/locale";

function setLocaleCookie(locale: Locale): void {
  document.cookie = `${LOCALE_COOKIE}=${locale}; path=/; max-age=31536000; samesite=lax`;
}

export type LanguageToggleProps = {
  readonly label: string;
};

/**
 * Server components (every page) read the `locale` cookie fresh on each
 * request, so switching requires a `router.refresh()` — unlike the theme
 * toggle, a DOM attribute flip alone would leave server-rendered copy in
 * the old language until the next navigation.
 */
export function LanguageToggle({ label }: LanguageToggleProps) {
  const { locale, setLocale } = useLocale();
  const router = useRouter();

  function switchTo(next: Locale): void {
    if (next === locale) {
      return;
    }
    setLocaleCookie(next);
    setLocale(next);
    router.refresh();
  }

  return (
    <fieldset aria-label={label} className="language-toggle">
      <button aria-pressed={locale === "en"} onClick={() => switchTo("en")} type="button">
        EN
      </button>
      <button aria-pressed={locale === "ko"} onClick={() => switchTo("ko")} type="button">
        한국어
      </button>
    </fieldset>
  );
}
