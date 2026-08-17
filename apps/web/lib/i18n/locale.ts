export type Locale = "en" | "ko";

export const LOCALES: readonly Locale[] = ["en", "ko"];

export const DEFAULT_LOCALE: Locale = "en";

export const LOCALE_COOKIE = "locale";

export function parseLocale(value: string | undefined): Locale {
  return value === "ko" ? "ko" : DEFAULT_LOCALE;
}

export type LocaleDictionary<T> = Record<Locale, T>;
