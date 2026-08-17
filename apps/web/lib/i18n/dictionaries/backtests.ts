import type { LocaleDictionary } from "@/lib/i18n/locale";

export type BacktestsDictionary = {
  readonly asOfLabel: (value: string) => string;
  readonly backtestQueuedMessage: (id: string) => string;
  readonly blockedTitle: string;
  readonly cancelBacktestButton: string;
  readonly canceledRunsMessage: string;
  readonly cancellationRequestedMessage: string;
  readonly compareRunsAriaLabel: string;
  readonly compareRunsHeading: string;
  readonly compareSelectedRunsButton: string;
  readonly comparisonEyebrow: string;
  readonly completedRunsLegend: string;
  readonly costColumnHeader: string;
  readonly costStressLabel: string;
  readonly createBacktestLabel: string;
  readonly createEyebrow: string;
  readonly creatingBacktestLabel: string;
  readonly creationUnavailableMessage: string;
  readonly creationUnavailableTitle: string;
  readonly datasetBlockedMessage: string;
  readonly dateColumnHeader: string;
  readonly drawdownColumnHeader: string;
  readonly drawdownCurveCaption: string;
  readonly emptyMessage: string;
  readonly emptyTitle: string;
  readonly endDateLabel: string;
  readonly endingEquityLabel: string;
  readonly entitlementInactiveMessage: string;
  readonly equityColumnHeader: string;
  readonly equityCurveCaption: string;
  readonly equityDrawdownHeading: string;
  readonly failedMessage: string;
  readonly failedTitle: string;
  readonly historyCaption: string;
  readonly historyEyebrow: string;
  readonly historyHeading: string;
  readonly initialCashLabel: string;
  readonly instrumentColumnHeader: string;
  readonly invalidDateRangeMessage: string;
  readonly licenseStateNotReported: string;
  readonly maximumDrawdownLabel: string;
  readonly monthColumnHeader: string;
  readonly monthlyReturnsCaption: string;
  readonly monthlyReturnsHeading: string;
  readonly notReported: string;
  readonly noWarnings: string;
  readonly openDateLabel: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly parameterSensitivityLabel: string;
  readonly percentComplete: (progress: string) => string;
  readonly periodColumnHeader: string;
  readonly periodRange: (start: string, end: string) => string;
  readonly progressEyebrow: string;
  readonly progressHeading: string;
  readonly progressNotReported: string;
  readonly quantityColumnHeader: string;
  readonly queueingRobustnessLabel: string;
  readonly reportEyebrow: string;
  readonly reportHeading: string;
  readonly reportSubheading: string;
  readonly requestingCancellationLabel: string;
  readonly returnColumnHeader: string;
  readonly robustnessEvidenceHeading: string;
  readonly robustnessQueuedMessage: string;
  readonly runColumnHeader: string;
  readonly runComparisonHeading: string;
  readonly runIdColumnHeader: string;
  readonly runRobustnessButton: string;
  readonly selectTwoRunsMessage: string;
  readonly serverQueuesMessage: string;
  readonly sideColumnHeader: string;
  readonly startDateLabel: string;
  readonly statusColumnHeader: string;
  readonly supportingCopy: (
    executionProfile: string,
    costProfileId: string,
    benchmark: string,
  ) => string;
  readonly timeColumnHeader: string;
  readonly totalReturnDeltaLabel: string;
  readonly tradeColumnHeader: string;
  readonly tradesCaption: string;
  readonly tradesCostsHeading: string;
  readonly tradesSummary: (count: string, cost: string) => string;
  readonly unavailableMessage: string;
  readonly unavailableTitle: string;
  readonly validationPeriodsLabel: string;
  readonly warningsAriaLabel: string;
  readonly warningsHeading: string;
};

export const backtestsDictionary: LocaleDictionary<BacktestsDictionary> = {
  en: {
    asOfLabel: (value) => `As of ${value}`,
    backtestQueuedMessage: (id) => `Backtest queued (${id}).`,
    blockedTitle: "Backtest data is blocked",
    cancelBacktestButton: "Cancel backtest",
    canceledRunsMessage: "Canceled and failed runs do not expose result payloads.",
    cancellationRequestedMessage:
      "Cancellation requested. The server will preserve the job audit trail.",
    compareRunsAriaLabel: "Compare backtest runs",
    compareRunsHeading: "Compare runs",
    compareSelectedRunsButton: "Compare selected runs",
    comparisonEyebrow: "Server comparison",
    completedRunsLegend: "Completed runs",
    costColumnHeader: "Cost",
    costStressLabel: "Cost stress",
    createBacktestLabel: "Create backtest",
    createEyebrow: "Version-pinned simulation",
    creatingBacktestLabel: "Creating backtest",
    creationUnavailableMessage:
      "The server did not provide the versioned strategy and dataset defaults required for creation.",
    creationUnavailableTitle: "Backtest creation is unavailable",
    datasetBlockedMessage:
      "The entitlement or dataset is blocked. Creation is disabled and proprietary results are not rendered.",
    dateColumnHeader: "Date",
    drawdownColumnHeader: "Drawdown",
    drawdownCurveCaption: "Server-provided drawdown curve",
    emptyMessage: "Create a version-pinned backtest to populate this history.",
    emptyTitle: "No backtests available",
    endDateLabel: "End date",
    endingEquityLabel: "Ending equity",
    entitlementInactiveMessage:
      "The backtest entitlement is inactive. Creation is disabled and proprietary results are not rendered.",
    equityColumnHeader: "Equity",
    equityCurveCaption: "Server-provided equity curve",
    equityDrawdownHeading: "Equity and drawdown",
    failedMessage:
      "The worker did not produce a verified result. Review the run status before retrying.",
    failedTitle: "Backtest failed",
    historyCaption: "Backtest jobs and result availability",
    historyEyebrow: "Private run history",
    historyHeading: "Backtest runs",
    initialCashLabel: "Initial cash (KRW)",
    instrumentColumnHeader: "Instrument",
    invalidDateRangeMessage: "Enter a valid date range and a positive KRW amount.",
    licenseStateNotReported: "NOT REPORTED",
    maximumDrawdownLabel: "Maximum drawdown",
    monthColumnHeader: "Month",
    monthlyReturnsCaption: "Server-provided monthly returns",
    monthlyReturnsHeading: "Monthly returns",
    notReported: "Not reported",
    noWarnings: "No server warnings.",
    openDateLabel: "Open",
    pageDescription:
      "Create reproducible simulations and inspect performance, cost, drawdown, and robustness evidence.",
    pageTitle: "Backtests",
    parameterSensitivityLabel: "Parameter sensitivity",
    percentComplete: (progress) => `${progress}% complete`,
    periodColumnHeader: "Period",
    periodRange: (start, end) => `${start} to ${end}`,
    progressEyebrow: "Queued execution",
    progressHeading: "Backtest progress",
    progressNotReported: "Progress not reported",
    quantityColumnHeader: "Quantity",
    queueingRobustnessLabel: "Queueing robustness evidence",
    reportEyebrow: "Verified server result",
    reportHeading: "Backtest result",
    reportSubheading: "Historical strategy simulation. Review execution assumptions and warnings.",
    requestingCancellationLabel: "Requesting cancellation",
    returnColumnHeader: "Return",
    robustnessEvidenceHeading: "Robustness evidence",
    robustnessQueuedMessage:
      "Robustness queued. Existing evidence remains visible while the server runs it.",
    runColumnHeader: "Run",
    runComparisonHeading: "Run comparison",
    runIdColumnHeader: "Run ID",
    runRobustnessButton: "Run robustness evidence",
    selectTwoRunsMessage: "Select exactly two completed runs.",
    serverQueuesMessage: "Only the server queues and calculates backtest results.",
    sideColumnHeader: "Side",
    startDateLabel: "Start date",
    statusColumnHeader: "Status",
    supportingCopy: (executionProfile, costProfileId, benchmark) =>
      `The server applies ${executionProfile}, ${costProfileId}, and benchmark ${benchmark}.`,
    timeColumnHeader: "Time",
    totalReturnDeltaLabel: "Total return delta",
    tradeColumnHeader: "Trade",
    tradesCaption: "Executed trades and server-calculated costs",
    tradesCostsHeading: "Trades and costs",
    tradesSummary: (count, cost) => `${count} trades. Total cost ${cost}.`,
    unavailableMessage:
      "Backtest data could not be loaded. Retry after checking the service status.",
    unavailableTitle: "Backtests unavailable",
    validationPeriodsLabel: "Validation periods",
    warningsAriaLabel: "Backtest warnings",
    warningsHeading: "Warnings",
  },
  ko: {
    asOfLabel: (value) => `기준일: ${value}`,
    backtestQueuedMessage: (id) => `백테스트가 대기열에 등록되었습니다 (${id}).`,
    blockedTitle: "백테스트 데이터가 차단되었습니다",
    cancelBacktestButton: "백테스트 취소",
    canceledRunsMessage: "취소되거나 실패한 실행은 결과 데이터를 제공하지 않습니다.",
    cancellationRequestedMessage: "취소가 요청되었습니다. 서버는 작업 감사 기록을 보존합니다.",
    compareRunsAriaLabel: "백테스트 실행 비교",
    compareRunsHeading: "실행 비교",
    compareSelectedRunsButton: "선택한 실행 비교",
    comparisonEyebrow: "서버 비교",
    completedRunsLegend: "완료된 실행",
    costColumnHeader: "비용",
    costStressLabel: "비용 스트레스",
    createBacktestLabel: "백테스트 생성",
    createEyebrow: "버전 고정 시뮬레이션",
    creatingBacktestLabel: "백테스트 생성 중",
    creationUnavailableMessage:
      "생성에 필요한 버전 지정 전략 및 데이터셋 기본값을 서버가 제공하지 않았습니다.",
    creationUnavailableTitle: "백테스트 생성을 사용할 수 없습니다",
    datasetBlockedMessage:
      "권한 또는 데이터셋이 차단되었습니다. 생성이 비활성화되며 독점 결과는 표시되지 않습니다.",
    dateColumnHeader: "날짜",
    drawdownColumnHeader: "낙폭",
    drawdownCurveCaption: "서버가 제공한 낙폭 곡선",
    emptyMessage: "버전 고정 백테스트를 생성하여 이 기록을 채우세요.",
    emptyTitle: "사용 가능한 백테스트가 없습니다",
    endDateLabel: "종료일",
    endingEquityLabel: "기말 자산",
    entitlementInactiveMessage:
      "백테스트 권한이 비활성 상태입니다. 생성이 비활성화되며 독점 결과는 표시되지 않습니다.",
    equityColumnHeader: "자산",
    equityCurveCaption: "서버가 제공한 자산 곡선",
    equityDrawdownHeading: "자산 및 낙폭",
    failedMessage:
      "워커가 검증된 결과를 생성하지 못했습니다. 다시 시도하기 전에 실행 상태를 확인하세요.",
    failedTitle: "백테스트 실패",
    historyCaption: "백테스트 작업 및 결과 제공 여부",
    historyEyebrow: "비공개 실행 기록",
    historyHeading: "백테스트 실행",
    initialCashLabel: "초기 자금 (KRW)",
    instrumentColumnHeader: "종목",
    invalidDateRangeMessage: "유효한 기간과 양수의 원화 금액을 입력하세요.",
    licenseStateNotReported: "보고되지 않음",
    maximumDrawdownLabel: "최대 낙폭",
    monthColumnHeader: "월",
    monthlyReturnsCaption: "서버가 제공한 월별 수익률",
    monthlyReturnsHeading: "월별 수익률",
    notReported: "보고되지 않음",
    noWarnings: "서버 경고가 없습니다.",
    openDateLabel: "미지정",
    pageDescription:
      "재현 가능한 시뮬레이션을 생성하고 성과, 비용, 낙폭, 견고성 근거를 검토하세요.",
    pageTitle: "백테스트",
    parameterSensitivityLabel: "파라미터 민감도",
    percentComplete: (progress) => `${progress}% 완료`,
    periodColumnHeader: "기간",
    periodRange: (start, end) => `${start} ~ ${end}`,
    progressEyebrow: "대기 중인 실행",
    progressHeading: "백테스트 진행 상황",
    progressNotReported: "진행률이 보고되지 않았습니다",
    quantityColumnHeader: "수량",
    queueingRobustnessLabel: "견고성 근거 대기열 등록 중",
    reportEyebrow: "검증된 서버 결과",
    reportHeading: "백테스트 결과",
    reportSubheading: "과거 전략 시뮬레이션입니다. 실행 가정과 경고를 검토하세요.",
    requestingCancellationLabel: "취소 요청 중",
    returnColumnHeader: "수익률",
    robustnessEvidenceHeading: "견고성 근거",
    robustnessQueuedMessage:
      "견고성 검증이 대기열에 등록되었습니다. 서버가 실행하는 동안 기존 근거는 계속 표시됩니다.",
    runColumnHeader: "실행",
    runComparisonHeading: "실행 비교 결과",
    runIdColumnHeader: "실행 ID",
    runRobustnessButton: "견고성 근거 실행",
    selectTwoRunsMessage: "완료된 실행을 정확히 두 개 선택하세요.",
    serverQueuesMessage: "백테스트 결과의 대기열 등록과 계산은 서버만 수행합니다.",
    sideColumnHeader: "매매 구분",
    startDateLabel: "시작일",
    statusColumnHeader: "상태",
    supportingCopy: (executionProfile, costProfileId, benchmark) =>
      `서버는 ${executionProfile}, ${costProfileId}, 벤치마크 ${benchmark}를 적용합니다.`,
    timeColumnHeader: "시각",
    totalReturnDeltaLabel: "총수익률 차이",
    tradeColumnHeader: "거래",
    tradesCaption: "체결된 거래 및 서버 계산 비용",
    tradesCostsHeading: "거래 및 비용",
    tradesSummary: (count, cost) => `거래 ${count}건. 총 비용 ${cost}.`,
    unavailableMessage:
      "백테스트 데이터를 불러오지 못했습니다. 서비스 상태를 확인한 후 다시 시도하세요.",
    unavailableTitle: "백테스트를 사용할 수 없습니다",
    validationPeriodsLabel: "검증 기간",
    warningsAriaLabel: "백테스트 경고",
    warningsHeading: "경고",
  },
};
