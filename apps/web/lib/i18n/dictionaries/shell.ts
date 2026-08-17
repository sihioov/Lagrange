import type { LocaleDictionary } from "@/lib/i18n/locale";

export type ShellDictionary = {
  readonly backtestsDescription: string;
  readonly brandName: string;
  readonly brandTagline: string;
  readonly chooseWorkspaceDescription: string;
  readonly chooseWorkspaceHeading: string;
  readonly dashboardDescription: string;
  readonly dashboardTitle: string;
  readonly errorMessage: string;
  readonly errorTitle: string;
  readonly languageToggleLabel: string;
  readonly loadingMessage: string;
  readonly loadingTitle: string;
  readonly navAdministration: string;
  readonly navBacktests: string;
  readonly navCandidates: string;
  readonly navDashboard: string;
  readonly navLiveControls: string;
  readonly navPaperAccount: string;
  readonly navRecommendations: string;
  readonly navScreener: string;
  readonly navStrategies: string;
  readonly ownerAccessRequiredMessage: string;
  readonly ownerAccessRequiredTitle: string;
  readonly paperAccountDescription: string;
  readonly privateSession: string;
  readonly recommendationsDescription: string;
  readonly refusedDescription: string;
  readonly returnToDashboard: string;
  readonly roleMember: string;
  readonly roleOwner: string;
  readonly signOut: string;
  readonly signOutFailed: string;
  readonly signingOut: string;
  readonly skipToMain: string;
  readonly strategiesDescription: string;
  readonly themeToggleToDark: string;
  readonly themeToggleToLight: string;
  readonly tryAgain: string;
};

export const shellDictionary: LocaleDictionary<ShellDictionary> = {
  en: {
    backtestsDescription: "Create reproducible runs and review risk evidence.",
    brandName: "Lagrange Station",
    brandTagline: "Equilibrium research console",
    chooseWorkspaceDescription:
      "Each destination opens a server-authorized view with conservative failure states.",
    chooseWorkspaceHeading: "Choose a workspace",
    dashboardDescription:
      "Move between isolated research workspaces. Authenticated data is fetched per request and is never shared across sessions.",
    dashboardTitle: "Research dashboard",
    errorMessage:
      "The authenticated request could not be completed. Retry the request without reusing a cached response.",
    errorTitle: "We could not load this workspace",
    languageToggleLabel: "Language",
    loadingMessage:
      "The authenticated workspace is requesting current data without using a shared cache.",
    loadingTitle: "Loading workspace",
    navAdministration: "Administration",
    navBacktests: "Backtests",
    navCandidates: "Daily candidates",
    navDashboard: "Dashboard",
    navLiveControls: "Live controls",
    navPaperAccount: "Paper account",
    navRecommendations: "Recommendations",
    navScreener: "Stock screener",
    navStrategies: "Strategies",
    ownerAccessRequiredMessage:
      "This workspace requires the Owner role. Your current session remains signed in with Member access.",
    ownerAccessRequiredTitle: "Owner access required",
    paperAccountDescription: "Monitor your private simulated account and orders.",
    privateSession: "Private session",
    recommendationsDescription: "Inspect explainable candidates, weights, and exclusions.",
    refusedDescription: "This area is restricted to the Owner.",
    returnToDashboard: "Return to dashboard",
    roleMember: "Member",
    roleOwner: "Owner",
    signOut: "Sign out",
    signOutFailed: "Sign out failed. Check your connection and retry.",
    signingOut: "Signing out",
    skipToMain: "Skip to main content",
    strategiesDescription: "Review approved strategies and their constrained parameters.",
    themeToggleToDark: "Switch to dark theme",
    themeToggleToLight: "Switch to light theme",
    tryAgain: "Try again",
  },
  ko: {
    backtestsDescription: "재현 가능한 실행을 생성하고 리스크 근거를 검토하세요.",
    brandName: "Lagrange Station",
    brandTagline: "평형 연구 콘솔",
    chooseWorkspaceDescription:
      "각 목적지는 서버가 인가한 화면을 열며, 실패 시에도 보수적으로 동작합니다.",
    chooseWorkspaceHeading: "워크스페이스 선택",
    dashboardDescription:
      "격리된 연구 워크스페이스 사이를 이동하세요. 인증된 데이터는 요청마다 새로 가져오며 세션 간에 공유되지 않습니다.",
    dashboardTitle: "연구 대시보드",
    errorMessage: "인증된 요청을 완료하지 못했습니다. 캐시된 응답을 사용하지 않고 다시 시도하세요.",
    errorTitle: "이 워크스페이스를 불러오지 못했습니다",
    languageToggleLabel: "언어",
    loadingMessage:
      "인증된 워크스페이스가 공유 캐시를 사용하지 않고 최신 데이터를 요청하고 있습니다.",
    loadingTitle: "워크스페이스 불러오는 중",
    navAdministration: "운영 관리",
    navBacktests: "백테스트",
    navCandidates: "일일 후보 종목",
    navDashboard: "대시보드",
    navLiveControls: "실거래 제어",
    navPaperAccount: "모의투자 계좌",
    navRecommendations: "추천",
    navScreener: "종목 스크리너",
    navStrategies: "전략",
    ownerAccessRequiredMessage:
      "이 워크스페이스는 오너 권한이 필요합니다. 현재 세션은 멤버 권한으로 로그인된 상태입니다.",
    ownerAccessRequiredTitle: "오너 권한이 필요합니다",
    paperAccountDescription: "비공개 모의투자 계좌와 주문 내역을 확인하세요.",
    privateSession: "비공개 세션",
    recommendationsDescription: "설명 가능한 후보 종목, 비중, 제외 내역을 확인하세요.",
    refusedDescription: "이 영역은 오너 전용입니다.",
    returnToDashboard: "대시보드로 돌아가기",
    roleMember: "멤버",
    roleOwner: "오너",
    signOut: "로그아웃",
    signOutFailed: "로그아웃에 실패했습니다. 연결 상태를 확인한 후 다시 시도하세요.",
    signingOut: "로그아웃 중",
    skipToMain: "본문으로 건너뛰기",
    strategiesDescription: "승인된 전략과 제한된 파라미터를 확인하세요.",
    themeToggleToDark: "다크 테마로 전환",
    themeToggleToLight: "라이트 테마로 전환",
    tryAgain: "다시 시도",
  },
};
