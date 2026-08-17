import type { LocaleDictionary } from "@/lib/i18n/locale";

export type PaperDictionary = {
  readonly accountBranchingEyebrow: string;
  readonly accountLabel: string;
  readonly accountSwitcherEyebrow: string;
  readonly accountSwitcherHeading: string;
  readonly activeState: string;
  readonly bindFormAriaLabel: string;
  readonly bindFormBoundMessage: (strategyId: string, strategyVersion: string) => string;
  readonly bindFormBranchExplanation: string;
  readonly bindFormButtonBind: string;
  readonly bindFormButtonBinding: string;
  readonly bindFormCurrentlyBoundTo: (strategy: string) => string;
  readonly bindFormNoActiveBinding: string;
  readonly bindFormSelectConfigError: string;
  readonly bindFormStrategyConfigLabel: string;
  readonly bindingHistoryCaption: string;
  readonly bindStrategyHeading: string;
  readonly branchedState: string;
  readonly columnAveragePrice: string;
  readonly columnBacktest: string;
  readonly columnBacktestWeight: string;
  readonly columnBound: string;
  readonly columnCash: string;
  readonly columnComputedOn: string;
  readonly columnDailyReturn: string;
  readonly columnDate: string;
  readonly columnDelivery: string;
  readonly columnEquity: string;
  readonly columnExecuted: string;
  readonly columnExecutesAt: string;
  readonly columnField: string;
  readonly columnInstrument: string;
  readonly columnKind: string;
  readonly columnNotice: string;
  readonly columnOrder: string;
  readonly columnPaper: string;
  readonly columnPaperWeight: string;
  readonly columnPositions: string;
  readonly columnPrice: string;
  readonly columnQuantity: string;
  readonly columnRaised: string;
  readonly columnSide: string;
  readonly columnState: string;
  readonly columnStrategy: string;
  readonly columnUnbound: string;
  readonly columnVersion: string;
  readonly costProfileLabel: string;
  readonly createStrategyConfigLink: string;
  readonly currentPositionsCaption: string;
  readonly dataBlockedTitle: string;
  readonly deliveryFailedMessage: (detail: string) => string;
  readonly entitlementInactiveMessage: string;
  readonly fillModelDifferenceLabel: string;
  readonly firstSession: string;
  readonly holdingsTitle: string;
  readonly ledgerEquityCaption: string;
  readonly lineageDifferencesCaption: string;
  readonly lineageTitle: string;
  readonly noAccountMessage: string;
  readonly noAccountTitle: string;
  readonly noConfigMessage: string;
  readonly noConfigTitle: string;
  readonly noDetailRecorded: string;
  readonly noNoticesMessage: string;
  readonly noParityMessage: string;
  readonly noParityTitle: string;
  readonly noSessionsValuedMessage: string;
  readonly noticesCaption: string;
  readonly notificationsTitle: string;
  readonly notReported: string;
  readonly notTargeted: string;
  readonly notYet: string;
  readonly onlyServerBindsNote: string;
  readonly openingCashLabel: string;
  readonly ownerLabel: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly paperOrdersCaption: string;
  readonly parityAriaLabel: (status: string) => string;
  readonly parityTitle: string;
  readonly performanceTitle: string;
  readonly reviewStrategiesLink: string;
  readonly sessionLabel: string;
  readonly sessionTargetsCaption: string;
  readonly signalDivergencesCaption: string;
  readonly statusDivergent: string;
  readonly statusLabel: string;
  readonly statusMatch: string;
  readonly statusNotComparable: string;
  readonly stillBound: string;
  readonly sharedAccountLabel: (owner: string) => string;
  readonly sharedAccountShortLabel: string;
  readonly unavailableMessage: string;
  readonly unavailableTitle: string;
  readonly yourAccountLabel: string;
};

export const paperDictionary: LocaleDictionary<PaperDictionary> = {
  en: {
    accountBranchingEyebrow: "Account branching",
    accountLabel: "Account",
    accountSwitcherEyebrow: "Invite group",
    accountSwitcherHeading: "Shared paper accounts",
    activeState: "Active",
    bindFormAriaLabel: "Bind strategy",
    bindFormBoundMessage: (strategyId, strategyVersion) =>
      `Bound ${strategyId}@${strategyVersion}. Sessions from the next close run on this version; earlier sessions keep theirs.`,
    bindFormBranchExplanation:
      "Binding a different configuration branches the account: the current binding is closed and a new one opens, so execution history never mixes strategy versions.",
    bindFormButtonBind: "Bind strategy",
    bindFormButtonBinding: "Binding strategy",
    bindFormCurrentlyBoundTo: (strategy) => `Currently bound to ${strategy}.`,
    bindFormNoActiveBinding: "This account has no active binding yet.",
    bindFormSelectConfigError: "Select a strategy configuration to bind.",
    bindFormStrategyConfigLabel: "Strategy configuration",
    bindingHistoryCaption: "Strategy binding history",
    bindStrategyHeading: "Bind strategy",
    branchedState: "Branched",
    columnAveragePrice: "Average price",
    columnBacktest: "Backtest",
    columnBacktestWeight: "Backtest weight",
    columnBound: "Bound",
    columnCash: "Cash",
    columnComputedOn: "Computed on",
    columnDailyReturn: "Daily return",
    columnDate: "Date",
    columnDelivery: "Delivery",
    columnEquity: "Equity",
    columnExecuted: "Executed",
    columnExecutesAt: "Executes at",
    columnField: "Field",
    columnInstrument: "Instrument",
    columnKind: "Kind",
    columnNotice: "Notice",
    columnOrder: "Order",
    columnPaper: "Paper",
    columnPaperWeight: "Paper weight",
    columnPositions: "Positions",
    columnPrice: "Price",
    columnQuantity: "Quantity",
    columnRaised: "Raised",
    columnSide: "Side",
    columnState: "State",
    columnStrategy: "Strategy",
    columnUnbound: "Unbound",
    columnVersion: "Version",
    costProfileLabel: "Cost profile",
    createStrategyConfigLink: "Create a strategy configuration",
    currentPositionsCaption: "Current positions",
    dataBlockedTitle: "Paper data is blocked",
    deliveryFailedMessage: (detail) => `Delivery failed — ${detail}`,
    entitlementInactiveMessage:
      "The paper entitlement is inactive. Account data and simulated results are not rendered.",
    fillModelDifferenceLabel: "Fill model difference",
    firstSession: "First session",
    holdingsTitle: "Account and holdings",
    ledgerEquityCaption: "Ledger-derived daily equity",
    lineageDifferencesCaption: "Lineage differences",
    lineageTitle: "Strategy and target lineage",
    noAccountMessage:
      "No paper account is selected. Account data can populate this route only after server ownership checks succeed.",
    noAccountTitle: "No paper account selected",
    noConfigMessage: "Binding an account needs a saved strategy configuration.",
    noConfigTitle: "No strategy configuration to bind",
    noDetailRecorded: "no detail recorded",
    noNoticesMessage:
      "No session notices yet. Completion, block, and divergence notices appear here once a session settles.",
    noParityMessage:
      "No session has queued a target yet, so there is nothing to compare against a backtest.",
    noParityTitle: "No parity report available",
    noSessionsValuedMessage:
      "No sessions have been valued yet. Performance appears after the first close valuation.",
    noticesCaption: "Notices and delivery outcome",
    notificationsTitle: "Session notifications",
    notReported: "Not reported",
    notTargeted: "Not targeted",
    notYet: "Not yet",
    onlyServerBindsNote: "Only the server opens and closes bindings.",
    openingCashLabel: "Opening cash",
    ownerLabel: "Owner",
    pageDescription:
      "Review cash, positions, orders, fills, and daily performance across shared simulated accounts.",
    pageTitle: "Paper account",
    paperOrdersCaption: "Paper orders and fills",
    parityAriaLabel: (status) => `Paper parity ${status}`,
    parityTitle: "Backtest parity",
    performanceTitle: "Daily performance",
    reviewStrategiesLink: "Review strategies",
    sessionLabel: "Session",
    sessionTargetsCaption: "Session targets",
    signalDivergencesCaption: "Signal divergences",
    statusDivergent: "Divergent",
    statusLabel: "Status",
    statusMatch: "Match",
    statusNotComparable: "Not comparable",
    stillBound: "Still bound",
    sharedAccountLabel: (owner) => `Shared account · ${owner}`,
    sharedAccountShortLabel: "Shared",
    unavailableMessage:
      "Paper account data could not be loaded. Retry after checking the service status.",
    unavailableTitle: "Paper account unavailable",
    yourAccountLabel: "Your account",
  },
  ko: {
    accountBranchingEyebrow: "계좌 분기",
    accountLabel: "계좌",
    accountSwitcherEyebrow: "초대 그룹",
    accountSwitcherHeading: "공유 모의투자 계좌",
    activeState: "활성",
    bindFormAriaLabel: "전략 바인딩",
    bindFormBoundMessage: (strategyId, strategyVersion) =>
      `${strategyId}@${strategyVersion}에 바인딩되었습니다. 다음 마감부터의 세션은 이 버전으로 실행되며, 이전 세션은 기존 버전을 유지합니다.`,
    bindFormBranchExplanation:
      "다른 구성으로 바인딩하면 계좌가 분기됩니다: 현재 바인딩이 종료되고 새 바인딩이 시작되므로, 실행 이력에 전략 버전이 섞이지 않습니다.",
    bindFormButtonBind: "전략 바인딩",
    bindFormButtonBinding: "바인딩 중",
    bindFormCurrentlyBoundTo: (strategy) => `현재 ${strategy}에 바인딩되어 있습니다.`,
    bindFormNoActiveBinding: "이 계좌는 아직 활성 바인딩이 없습니다.",
    bindFormSelectConfigError: "바인딩할 전략 구성을 선택하세요.",
    bindFormStrategyConfigLabel: "전략 구성",
    bindingHistoryCaption: "전략 바인딩 이력",
    bindStrategyHeading: "전략 바인딩",
    branchedState: "분기됨",
    columnAveragePrice: "평균 단가",
    columnBacktest: "백테스트",
    columnBacktestWeight: "백테스트 비중",
    columnBound: "바인딩",
    columnCash: "현금",
    columnComputedOn: "계산일",
    columnDailyReturn: "일일 수익률",
    columnDate: "날짜",
    columnDelivery: "전송",
    columnEquity: "자산",
    columnExecuted: "실행됨",
    columnExecutesAt: "실행일",
    columnField: "항목",
    columnInstrument: "종목",
    columnKind: "종류",
    columnNotice: "알림",
    columnOrder: "주문",
    columnPaper: "모의투자",
    columnPaperWeight: "모의투자 비중",
    columnPositions: "포지션",
    columnPrice: "가격",
    columnQuantity: "수량",
    columnRaised: "발생 시각",
    columnSide: "매매 구분",
    columnState: "상태",
    columnStrategy: "전략",
    columnUnbound: "해제",
    columnVersion: "버전",
    costProfileLabel: "비용 프로필",
    createStrategyConfigLink: "전략 구성 생성",
    currentPositionsCaption: "현재 포지션",
    dataBlockedTitle: "모의투자 데이터가 차단되었습니다",
    deliveryFailedMessage: (detail) => `전송 실패 — ${detail}`,
    entitlementInactiveMessage:
      "모의투자 권한이 비활성 상태입니다. 계좌 데이터와 시뮬레이션 결과는 표시되지 않습니다.",
    fillModelDifferenceLabel: "체결 모델 차이",
    firstSession: "첫 세션",
    holdingsTitle: "계좌 및 보유 내역",
    ledgerEquityCaption: "원장 기반 일일 자산",
    lineageDifferencesCaption: "계보 차이",
    lineageTitle: "전략 및 목표 계보",
    noAccountMessage:
      "선택된 모의투자 계좌가 없습니다. 서버의 소유권 확인이 통과된 후에만 계좌 데이터가 이 화면에 표시될 수 있습니다.",
    noAccountTitle: "선택된 모의투자 계좌 없음",
    noConfigMessage: "계좌를 바인딩하려면 저장된 전략 구성이 필요합니다.",
    noConfigTitle: "바인딩할 전략 구성 없음",
    noDetailRecorded: "기록된 세부 정보 없음",
    noNoticesMessage:
      "아직 세션 알림이 없습니다. 완료, 차단, 괴리 알림은 세션이 마감되면 여기에 표시됩니다.",
    noParityMessage: "아직 목표를 대기열에 등록한 세션이 없어 백테스트와 비교할 대상이 없습니다.",
    noParityTitle: "제공 가능한 정합성 보고서 없음",
    noSessionsValuedMessage: "아직 평가된 세션이 없습니다. 첫 마감 평가 이후 성과가 표시됩니다.",
    noticesCaption: "알림 및 전송 결과",
    notificationsTitle: "세션 알림",
    notReported: "보고되지 않음",
    notTargeted: "목표 대상 아님",
    notYet: "아직 없음",
    onlyServerBindsNote: "바인딩의 개설과 종료는 서버만 수행합니다.",
    openingCashLabel: "개시 현금",
    ownerLabel: "소유자",
    pageDescription: "공유된 모의투자 계좌의 현금, 포지션, 주문, 체결, 일일 성과를 확인하세요.",
    pageTitle: "모의투자 계좌",
    paperOrdersCaption: "모의투자 주문 및 체결",
    parityAriaLabel: (status) => `모의투자 정합성 ${status}`,
    parityTitle: "백테스트 정합성",
    performanceTitle: "일일 성과",
    reviewStrategiesLink: "전략 검토",
    sessionLabel: "세션",
    sessionTargetsCaption: "세션 목표",
    signalDivergencesCaption: "시그널 괴리",
    statusDivergent: "괴리",
    statusLabel: "상태",
    statusMatch: "일치",
    statusNotComparable: "비교 불가",
    stillBound: "바인딩 유지 중",
    sharedAccountLabel: (owner) => `공유 계좌 · ${owner}`,
    sharedAccountShortLabel: "공유 계좌",
    unavailableMessage:
      "모의투자 계좌 데이터를 불러오지 못했습니다. 서비스 상태를 확인한 후 다시 시도하세요.",
    unavailableTitle: "모의투자 계좌를 사용할 수 없습니다",
    yourAccountLabel: "내 계좌",
  },
};
