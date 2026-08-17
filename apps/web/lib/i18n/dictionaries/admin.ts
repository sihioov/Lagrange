import type { LocaleDictionary } from "@/lib/i18n/locale";

export type AdminDictionary = {
  readonly noAreaMessage: string;
  readonly noAreaTitle: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
};

export const adminDictionary: LocaleDictionary<AdminDictionary> = {
  en: {
    noAreaMessage:
      "No administrative area is selected. Operational data can populate this route only through audited Owner APIs.",
    noAreaTitle: "Choose an administrative area",
    pageDescription:
      "Review datasets, jobs, workers, users, and immutable audit evidence through explicit Owner pathways.",
    pageTitle: "Administration",
  },
  ko: {
    noAreaMessage:
      "선택된 운영 관리 영역이 없습니다. 운영 데이터는 감사 가능한 오너 API를 통해서만 이 화면에 표시될 수 있습니다.",
    noAreaTitle: "운영 관리 영역을 선택하세요",
    pageDescription:
      "명시적인 오너 경로를 통해 데이터셋, 작업, 워커, 사용자, 그리고 변경 불가능한 감사 증적을 검토하세요.",
    pageTitle: "운영 관리",
  },
};
