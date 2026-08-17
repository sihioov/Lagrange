"use client";

import { createContext, type ReactNode, useContext, useMemo, useState } from "react";
import { DEFAULT_LOCALE, type Locale } from "./locale";

type LocaleContextValue = {
  readonly locale: Locale;
  readonly setLocale: (locale: Locale) => void;
};

const LocaleContext = createContext<LocaleContextValue>({
  locale: DEFAULT_LOCALE,
  setLocale: () => undefined,
});

export type LocaleProviderProps = {
  readonly children: ReactNode;
  readonly initialLocale: Locale;
};

/**
 * Client-side mirror of the `locale` cookie the server already read for this
 * request. Switching locale needs an immediate re-render for client
 * components (forms, the kill switch) that can't wait for the
 * `router.refresh()` a server component would need instead.
 */
export function LocaleProvider({ children, initialLocale }: LocaleProviderProps) {
  const [locale, setLocale] = useState<Locale>(initialLocale);
  const value = useMemo(() => ({ locale, setLocale }), [locale]);
  return <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>;
}

export function useLocale(): LocaleContextValue {
  return useContext(LocaleContext);
}
