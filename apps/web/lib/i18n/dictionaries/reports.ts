import type { LocaleDictionary } from "@/lib/i18n/locale";

export type ReportsDictionary = {
  readonly asOfLabel: string;
  readonly dataVersionLabel: string;
  readonly engineVersionLabel: string;
  readonly licenseStateLabel: string;
  readonly noWarningsMessage: string;
  readonly notReported: string;
  readonly strategyVersionLabel: string;
  readonly warningsTitle: string;
};

export const reportsDictionary: LocaleDictionary<ReportsDictionary> = {
  en: {
    asOfLabel: "As of",
    dataVersionLabel: "Data version",
    engineVersionLabel: "Engine version",
    licenseStateLabel: "License state",
    noWarningsMessage: "No server warnings.",
    notReported: "Not reported",
    strategyVersionLabel: "Strategy version",
    warningsTitle: "Warnings",
  },
  ko: {
    asOfLabel: "기준일",
    dataVersionLabel: "데이터 버전",
    engineVersionLabel: "엔진 버전",
    licenseStateLabel: "라이선스 상태",
    noWarningsMessage: "서버 경고가 없습니다.",
    notReported: "보고되지 않음",
    strategyVersionLabel: "전략 버전",
    warningsTitle: "경고",
  },
};
