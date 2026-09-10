"use client";

import type { Locale } from "@/lib/i18n/locale";
import { CurrentQuoteView, type IntradayQuoteDictionary } from "./current-quote-view";
import { useIntradayQuote } from "./use-intraday-quote";

export type CurrentQuoteClientProps = Pick<
  Parameters<typeof useIntradayQuote>[0],
  "enabled" | "identity" | "placementVisibility" | "sessionKey" | "surface"
> & {
  readonly copy: IntradayQuoteDictionary;
  readonly locale: Locale;
};

/** Only serializable quote inputs cross the detail page's server/client boundary. */
export function CurrentQuoteClient({ copy, locale, ...input }: CurrentQuoteClientProps) {
  const result = useIntradayQuote(input);
  return <CurrentQuoteView copy={copy} locale={locale} state={result.state} />;
}
