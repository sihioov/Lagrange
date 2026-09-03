import type { Locale } from "@/lib/i18n/locale";

const LOCALE_TAGS = {
  en: "en-US",
  ko: "ko-KR",
} as const satisfies Record<Locale, string>;

export class InvalidStockBetaNumericValue extends Error {
  override readonly name = "InvalidStockBetaNumericValue";
}

export type StockBetaNumericPresentation = {
  readonly rawValue: number;
  readonly text: string;
};

export type StockBetaNumberFormatOptions = {
  readonly fractionDigits?: number;
  readonly useGrouping?: boolean;
};

function validatedFractionDigits(fractionDigits: number): number {
  if (!Number.isSafeInteger(fractionDigits) || fractionDigits < 0 || fractionDigits > 12) {
    throw new InvalidStockBetaNumericValue(
      "fractionDigits must be a safe integer between 0 and 12.",
    );
  }
  return fractionDigits;
}

function validatedValue(value: number): number {
  if (!Number.isFinite(value)) {
    throw new InvalidStockBetaNumericValue("Stock-beta numeric values must be finite.");
  }
  return value;
}

function presentation(rawValue: number, text: string): StockBetaNumericPresentation {
  return { rawValue, text };
}

export function formatStockBetaNumber(
  value: number,
  locale: Locale,
  { fractionDigits = 2, useGrouping = true }: StockBetaNumberFormatOptions = {},
): StockBetaNumericPresentation {
  const rawValue = validatedValue(value);
  const digits = validatedFractionDigits(fractionDigits);
  const formatter = new Intl.NumberFormat(LOCALE_TAGS[locale], {
    maximumFractionDigits: digits,
    minimumFractionDigits: digits,
    useGrouping,
  });
  return presentation(rawValue, formatter.format(rawValue));
}

export function formatStockBetaPercent(
  value: number,
  locale: Locale,
  fractionDigits = 2,
): StockBetaNumericPresentation {
  const rawValue = validatedValue(value);
  const digits = validatedFractionDigits(fractionDigits);
  const formatter = new Intl.NumberFormat(LOCALE_TAGS[locale], {
    maximumFractionDigits: digits,
    minimumFractionDigits: digits,
    signDisplay: "exceptZero",
    style: "percent",
  });
  return presentation(rawValue, formatter.format(rawValue));
}
