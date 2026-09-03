import type { Locale } from "@/lib/i18n/locale";
import {
  formatStockBetaNumber as formatNumber,
  formatStockBetaPercent as formatPercent,
} from "../shared/formatters";

export function formatDashboardNumber(value: number, locale: Locale, fractionDigits = 2): string {
  return formatNumber(value, locale, { fractionDigits }).text;
}

export function formatDashboardPercent(value: number, locale: Locale, fractionDigits = 2): string {
  return formatPercent(value, locale, fractionDigits).text;
}

export function stockBetaFormatNumber(value: number, fractionDigits = 2): string {
  return formatDashboardNumber(value, "en", fractionDigits);
}

export function stockBetaFormatPercent(value: number): string {
  return formatDashboardPercent(value, "en");
}
