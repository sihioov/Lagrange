import type { LocaleDictionary } from "@/lib/i18n/locale";
import type { OwnerBetaSimulationStrategyId } from "@/lib/products/owner-beta-simulation-v2-contracts";

const ownerBetaSimulationEnglishLabels: Readonly<Record<OwnerBetaSimulationStrategyId, string>> = {
  buy_and_hold: "Buy and hold · 069500",
  trend_following: "Trend following · 50/200",
  relative_momentum: "Relative momentum · 12m / top 3",
  dual_momentum: "Dual momentum · 12m",
  inverse_volatility: "Inverse volatility · 60d",
};

const ownerBetaSimulationKoreanLabels: Readonly<Record<OwnerBetaSimulationStrategyId, string>> = {
  buy_and_hold: "매수 후 보유 · 069500",
  trend_following: "추세 추종 · 50/200",
  relative_momentum: "상대 모멘텀 · 12개월 / 상위 3개",
  dual_momentum: "듀얼 모멘텀 · 12개월",
  inverse_volatility: "역변동성 · 60일",
};

export type OwnerBetaSimulationDictionary = {
  readonly pageTitle: string;
  readonly pageDescription: string;
  readonly entrySectionLabel: string;
  readonly entryLinkLabel: string;
  readonly boundaryAriaLabel: string;
  readonly audienceLabel: string;
  readonly capabilityLabel: string;
  readonly vendorSnapshotLabel: string;
  readonly strictPitLabel: string;
  readonly boundaryExplanation: string;
  readonly priceReturnExplanation: string;
  readonly costExplanation: string;
  readonly executionExplanation: string;
  readonly datePhaseExplanation: string;
  readonly evaluationWindowExplanation: string;
  readonly defaultWindowExplanation: string;
  readonly disabledModeTitle: string;
  readonly disabledModeMessage: string;
  readonly formSectionLabel: string;
  readonly formTitle: string;
  readonly formDescription: string;
  readonly strategyConfigLabel: string;
  readonly selectConfigLabel: string;
  readonly configOptionLabel: (strategy: OwnerBetaSimulationStrategyId, date: string) => string;
  readonly strategyLabels: Readonly<Record<OwnerBetaSimulationStrategyId, string>>;
  readonly startDateLabel: string;
  readonly endDateLabel: string;
  readonly optionalLabel: string;
  readonly submitLabel: string;
  readonly submittingLabel: string;
  readonly invalidDateMessage: string;
  readonly configRequiredMessage: string;
  readonly noEligibleConfigTitle: string;
  readonly noEligibleConfigMessage: string;
  readonly configUnavailableTitle: string;
  readonly configUnavailableMessage: string;
  readonly historySectionLabel: string;
  readonly historyTitle: string;
  readonly historyCaption: string;
  readonly historyLoadingMessage: string;
  readonly startDateColumn: string;
  readonly endDateColumn: string;
  readonly historyStrategyColumn: string;
  readonly statusColumn: string;
  readonly selectRunLabel: (start: string, end: string, status: string) => string;
  readonly selectedRunLabel: string;
  readonly noRunsTitle: string;
  readonly noRunsMessage: string;
  readonly loadMoreLabel: string;
  readonly loadingMoreLabel: string;
  readonly loadMoreError: string;
  readonly statusSectionLabel: string;
  readonly statusTitle: string;
  readonly selectedPeriodLabel: (start: string, end: string) => string;
  readonly pendingTitle: string;
  readonly pendingMessage: string;
  readonly runningTitle: string;
  readonly runningMessage: string;
  readonly validatingTitle: string;
  readonly validatingMessage: string;
  readonly failedTitle: string;
  readonly failedMessage: string;
  readonly canceledTitle: string;
  readonly canceledMessage: string;
  readonly succeededMessage: string;
  readonly missingTitle: string;
  readonly missingMessage: string;
  readonly notReadyTitle: string;
  readonly notReadyMessage: string;
  readonly entitlementTitle: string;
  readonly entitlementMessage: string;
  readonly forbiddenTitle: string;
  readonly forbiddenMessage: string;
  readonly unavailableTitle: string;
  readonly unavailableMessage: string;
  readonly integrityTitle: string;
  readonly integrityMessage: string;
  readonly genericErrorTitle: string;
  readonly genericErrorMessage: string;
  readonly cancelLabel: string;
  readonly cancellingLabel: string;
  readonly cancelAcknowledgedMessage: string;
  readonly manualRefreshLabel: string;
  readonly refreshingLabel: string;
  readonly pollingExhaustedMessage: string;
  readonly historyLoadError: string;
  readonly reportSectionLabel: string;
  readonly reportTitle: string;
  readonly reportDescription: string;
  readonly illustrativeProfileTitle: string;
  readonly zeroCostProfileTitle: string;
  readonly strategyColumn: string;
  readonly benchmarkColumn: string;
  readonly metricColumn: string;
  readonly periodReturnLabel: string;
  readonly endingNavLabel: string;
  readonly endingCashLabel: string;
  readonly endingHoldingsLabel: string;
  readonly cumulativeCostsLabel: string;
  readonly buyNotionalLabel: string;
  readonly sellNotionalLabel: string;
  readonly maximumDrawdownLabel: string;
  readonly periodTurnoverLabel: string;
  readonly fillCountLabel: string;
  readonly benchmarkDifferenceLabel: string;
  readonly dailyReturnsTitle: string;
  readonly dailyReturnsCaption: (shown: number, total: number) => string;
  readonly dailyDateColumn: string;
  readonly dailyIllustrativeStrategyColumn: string;
  readonly dailyIllustrativeBenchmarkColumn: string;
  readonly dailyZeroStrategyColumn: string;
  readonly dailyZeroBenchmarkColumn: string;
  readonly exactValueLabel: string;
  readonly sharesLabel: string;
  readonly zeroSharesLabel: string;
  readonly notApplicableLabel: string;
};

export const ownerBetaSimulationDictionary: LocaleDictionary<OwnerBetaSimulationDictionary> = {
  en: {
    pageTitle: "Owner-only price-only simulation",
    pageDescription:
      "Compare one approved default strategy with its same-cost buy-and-hold benchmark.",
    entrySectionLabel: "Owner-only simulation",
    entryLinkLabel: "Open the owner-only price-only simulation",
    boundaryAriaLabel: "Simulation scope and limitations",
    audienceLabel: "OWNER_ONLY",
    capabilityLabel: "PRICE_RETURN_ONLY",
    vendorSnapshotLabel: "vendor_snapshot=true",
    strictPitLabel: "strict_pit=false",
    boundaryExplanation: "Owner-only research using the approved ETF11 historical price snapshot.",
    priceReturnExplanation:
      "Price return only. Dividends are excluded; this is not total return or investment advice.",
    costExplanation:
      "The illustrative profile assumes 10 basis points per side. It is not a broker fee or tax estimate; the zero-cost profile is shown beside it.",
    executionExplanation:
      "Execution is hypothetical at the next approved session open after a close-based signal.",
    datePhaseExplanation:
      "Ledger phases contain dates and phase names only; no observed or synthetic clock times are added.",
    evaluationWindowExplanation:
      "The server checks factor readiness for the selected strategy and returns its actual common evaluation period.",
    defaultWindowExplanation:
      "Leave dates blank to use the server-checked common period. The existing approved end limit is 2026-08-28.",
    disabledModeTitle: "Owner-only mode is required",
    disabledModeMessage:
      "This dedicated simulation is available only while the exact owner-only mode is active.",
    formSectionLabel: "Create an owner-only simulation",
    formTitle: "Queue a simulation",
    formDescription:
      "Choose one existing active canonical default. Each selection creates one run with both cost profiles.",
    strategyConfigLabel: "Eligible strategy configuration",
    selectConfigLabel: "Select one approved default strategy",
    configOptionLabel: (strategy, date) =>
      `${ownerBetaSimulationEnglishLabels[strategy]} · saved ${date}`,
    strategyLabels: ownerBetaSimulationEnglishLabels,
    startDateLabel: "Evaluation start date",
    endDateLabel: "Evaluation end date",
    optionalLabel: "Optional",
    submitLabel: "Queue simulation",
    submittingLabel: "Queueing simulation…",
    invalidDateMessage:
      "Enter valid dates in chronological order. The server checks the approved session calendar, warmup, and evaluation length.",
    configRequiredMessage: "Select an existing strategy configuration before queueing.",
    noEligibleConfigTitle: "No eligible strategy configuration",
    noEligibleConfigMessage:
      "No active configuration matches one of the five approved default strategies. Custom configurations are not accepted.",
    configUnavailableTitle: "Strategy configurations unavailable",
    configUnavailableMessage:
      "Existing configurations could not be loaded. No configuration was created or selected automatically.",
    historySectionLabel: "Owner-only simulation history",
    historyTitle: "Simulation history",
    historyCaption:
      "Your owner-scoped simulation runs. Select one to refresh its current status and inspect a complete successful pair.",
    historyLoadingMessage: "Loading owner-scoped simulation history.",
    startDateColumn: "Start",
    endDateColumn: "End",
    historyStrategyColumn: "Strategy",
    statusColumn: "Status",
    selectRunLabel: (start, end, status) => `Select ${start} to ${end}, ${status}`,
    selectedRunLabel: "Selected",
    noRunsTitle: "No simulations yet",
    noRunsMessage:
      "Queue a simulation with an existing eligible configuration to populate this history.",
    loadMoreLabel: "Load more runs",
    loadingMoreLabel: "Loading runs…",
    loadMoreError: "More history could not be loaded. You can try again.",
    statusSectionLabel: "Selected simulation status",
    statusTitle: "Selected simulation",
    selectedPeriodLabel: (start, end) => `Evaluation period: ${start} to ${end}`,
    pendingTitle: "Simulation queued",
    pendingMessage:
      "The server has queued this run. The status below comes from fresh metadata reads.",
    runningTitle: "Simulation running",
    runningMessage: "The server is still calculating. No partial profile is available.",
    validatingTitle: "Checking the complete result pair",
    validatingMessage:
      "The run is marked successful. Both original cost profiles must pass validation before any metrics appear.",
    failedTitle: "Simulation failed",
    failedMessage:
      "The server did not publish a complete successful pair. No performance metrics are available.",
    canceledTitle: "Simulation canceled",
    canceledMessage:
      "The server confirms this run is canceled. No performance metrics are available.",
    succeededMessage:
      "The server confirmed completion and both complete cost profiles passed validation.",
    missingTitle: "Selected run unavailable",
    missingMessage:
      "This selected run could not be found or is not available to this account. The selection has been kept.",
    notReadyTitle: "Complete results are not ready",
    notReadyMessage:
      "The server has not returned a complete pair for this selected run. No profile is shown.",
    entitlementTitle: "Simulation data is blocked",
    entitlementMessage:
      "Current Backtest data rights do not allow result access. Any displayed result has been cleared.",
    forbiddenTitle: "Simulation access is blocked",
    forbiddenMessage: "The server refused access to this selected run. No result profile is shown.",
    unavailableTitle: "Simulation service unavailable",
    unavailableMessage:
      "The service could not confirm the selected run. Refresh to check its current state.",
    integrityTitle: "Simulation result could not be verified",
    integrityMessage:
      "The complete pair did not match the frozen result contract. No metrics are shown.",
    genericErrorTitle: "Simulation request failed",
    genericErrorMessage:
      "The request could not be completed. Review the current status and try again.",
    cancelLabel: "Request cancellation",
    cancellingLabel: "Requesting cancellation…",
    cancelAcknowledgedMessage:
      "Cancellation was acknowledged. Refreshing the server's current run state.",
    manualRefreshLabel: "Refresh selected run",
    refreshingLabel: "Refreshing…",
    pollingExhaustedMessage:
      "Automatic status checks stopped after 60 attempts. Use manual refresh to check again.",
    historyLoadError: "Simulation history could not be loaded.",
    reportSectionLabel: "Complete paired simulation result",
    reportTitle: "Strategy and benchmark comparison",
    reportDescription:
      "Both cost profiles use the same strategy and the same-cost buy-and-hold benchmark. Returns are percentages; the benchmark difference is in percentage points. Period turnover is not annualized and uses half the buy-plus-sell notional.",
    illustrativeProfileTitle: "Illustrative 10 bp per side",
    zeroCostProfileTitle: "Zero-cost reference",
    strategyColumn: "Selected strategy",
    benchmarkColumn: "Buy-and-hold benchmark",
    metricColumn: "Allowed metric",
    periodReturnLabel: "Period return",
    endingNavLabel: "Ending NAV",
    endingCashLabel: "Ending cash",
    endingHoldingsLabel: "Ending holdings",
    cumulativeCostsLabel: "Cumulative costs",
    buyNotionalLabel: "Buy notional",
    sellNotionalLabel: "Sell notional",
    maximumDrawdownLabel: "Maximum drawdown",
    periodTurnoverLabel: "Period turnover (half-notional, not annualized)",
    fillCountLabel: "Fill count",
    benchmarkDifferenceLabel: "Benchmark return difference (percentage points)",
    dailyReturnsTitle: "Daily returns",
    dailyReturnsCaption: (shown, total) =>
      `Most recent ${shown} of ${total} evaluation sessions; dates only.`,
    dailyDateColumn: "Session date",
    dailyIllustrativeStrategyColumn: "Strategy · 10 bp",
    dailyIllustrativeBenchmarkColumn: "Benchmark · 10 bp",
    dailyZeroStrategyColumn: "Strategy · zero cost",
    dailyZeroBenchmarkColumn: "Benchmark · zero cost",
    exactValueLabel: "Exact stored value",
    sharesLabel: "shares",
    zeroSharesLabel: "0 shares",
    notApplicableLabel: "Not applicable",
  },
  ko: {
    pageTitle: "소유자 전용 가격수익률 시뮬레이션",
    pageDescription: "승인된 기본 전략 하나와 동일 비용의 매수 후 보유 벤치마크를 비교합니다.",
    entrySectionLabel: "소유자 전용 시뮬레이션",
    entryLinkLabel: "소유자 전용 가격수익률 시뮬레이션 열기",
    boundaryAriaLabel: "시뮬레이션 범위와 한계",
    audienceLabel: "OWNER_ONLY",
    capabilityLabel: "PRICE_RETURN_ONLY",
    vendorSnapshotLabel: "vendor_snapshot=true",
    strictPitLabel: "strict_pit=false",
    boundaryExplanation: "승인된 ETF11 과거 가격 스냅샷을 사용하는 소유자 전용 연구입니다.",
    priceReturnExplanation:
      "가격수익률만 포함합니다. 배당은 제외되며 총수익률이나 투자 조언이 아닙니다.",
    costExplanation:
      "예시 비용 프로필은 매수와 매도에 각각 10bp를 가정합니다. 실제 증권사 수수료나 세금 추정이 아니며, 비용 0 기준을 함께 표시합니다.",
    executionExplanation: "종가 신호 뒤 다음 승인 세션 시가에 가상 체결한다고 가정합니다.",
    datePhaseExplanation:
      "원장 단계에는 날짜와 단계 이름만 있으며 관측되거나 합성된 시각을 추가하지 않습니다.",
    evaluationWindowExplanation:
      "서버가 선택한 전략의 팩터 준비 상태를 확인하고 실제 공통 평가 기간을 반환합니다.",
    defaultWindowExplanation:
      "날짜를 비우면 서버가 확인한 공통 기간을 사용합니다. 기존 승인 종료 한도는 2026-08-28입니다.",
    disabledModeTitle: "소유자 전용 모드가 필요합니다",
    disabledModeMessage:
      "이 전용 시뮬레이션은 정확한 소유자 전용 모드가 활성화된 동안에만 사용할 수 있습니다.",
    formSectionLabel: "소유자 전용 시뮬레이션 생성",
    formTitle: "시뮬레이션 대기열 등록",
    formDescription:
      "활성 상태인 승인 기본 전략 하나를 선택하세요. 한 번 선택하면 두 비용 프로필을 포함한 실행 하나가 생성됩니다.",
    strategyConfigLabel: "사용 가능한 전략 설정",
    selectConfigLabel: "승인된 기본 전략 하나를 선택하세요",
    configOptionLabel: (strategy, date) =>
      `${ownerBetaSimulationKoreanLabels[strategy]} · 저장일 ${date}`,
    strategyLabels: ownerBetaSimulationKoreanLabels,
    startDateLabel: "평가 시작일",
    endDateLabel: "평가 종료일",
    optionalLabel: "선택 사항",
    submitLabel: "시뮬레이션 등록",
    submittingLabel: "시뮬레이션 등록 중…",
    invalidDateMessage:
      "유효한 날짜를 시간순으로 입력하세요. 승인된 세션 달력, 준비 기간 및 평가 길이는 서버가 확인합니다.",
    configRequiredMessage: "등록 전에 기존 전략 설정을 선택하세요.",
    noEligibleConfigTitle: "사용 가능한 전략 설정이 없습니다",
    noEligibleConfigMessage:
      "다섯 승인 기본 전략 중 하나와 일치하는 활성 설정이 없습니다. 사용자 정의 설정은 사용할 수 없습니다.",
    configUnavailableTitle: "전략 설정을 사용할 수 없습니다",
    configUnavailableMessage:
      "기존 설정을 불러오지 못했습니다. 설정을 생성하거나 자동 선택하지 않았습니다.",
    historySectionLabel: "소유자 전용 시뮬레이션 기록",
    historyTitle: "시뮬레이션 기록",
    historyCaption:
      "소유자에게 한정된 실행 목록입니다. 하나를 선택하면 현재 상태를 갱신하고 성공한 전체 결과 쌍을 확인합니다.",
    historyLoadingMessage: "소유자 전용 시뮬레이션 기록을 불러오는 중입니다.",
    startDateColumn: "시작일",
    endDateColumn: "종료일",
    historyStrategyColumn: "전략",
    statusColumn: "상태",
    selectRunLabel: (start, end, status) => `${start}부터 ${end}까지, ${status} 실행 선택`,
    selectedRunLabel: "선택됨",
    noRunsTitle: "시뮬레이션이 없습니다",
    noRunsMessage: "사용 가능한 기존 설정으로 시뮬레이션을 등록하면 기록이 표시됩니다.",
    loadMoreLabel: "이전 실행 더 보기",
    loadingMoreLabel: "실행 불러오는 중…",
    loadMoreError: "추가 기록을 불러오지 못했습니다. 다시 시도할 수 있습니다.",
    statusSectionLabel: "선택한 시뮬레이션 상태",
    statusTitle: "선택한 시뮬레이션",
    selectedPeriodLabel: (start, end) => `평가 기간: ${start} ~ ${end}`,
    pendingTitle: "시뮬레이션 대기 중",
    pendingMessage:
      "서버가 실행을 대기열에 등록했습니다. 아래 상태는 새 메타데이터 조회 결과입니다.",
    runningTitle: "시뮬레이션 실행 중",
    runningMessage: "서버가 계산 중입니다. 부분 결과는 제공되지 않습니다.",
    validatingTitle: "전체 결과 쌍 확인 중",
    validatingMessage:
      "실행 상태가 성공으로 확인되었습니다. 지표를 표시하기 전에 두 원본 비용 프로필을 모두 검증합니다.",
    failedTitle: "시뮬레이션 실패",
    failedMessage:
      "서버가 완전한 성공 결과 쌍을 게시하지 않았습니다. 성과 지표를 사용할 수 없습니다.",
    canceledTitle: "시뮬레이션 취소됨",
    canceledMessage: "서버가 이 실행의 취소를 확인했습니다. 성과 지표를 사용할 수 없습니다.",
    succeededMessage: "서버가 완료를 확인했고 두 비용 프로필 전체가 검증되었습니다.",
    missingTitle: "선택한 실행을 사용할 수 없습니다",
    missingMessage:
      "선택한 실행을 찾을 수 없거나 이 계정에서 사용할 수 없습니다. 선택은 유지됩니다.",
    notReadyTitle: "전체 결과가 준비되지 않았습니다",
    notReadyMessage:
      "서버가 선택한 실행의 완전한 결과 쌍을 반환하지 않았습니다. 프로필을 표시하지 않습니다.",
    entitlementTitle: "시뮬레이션 데이터가 차단되었습니다",
    entitlementMessage:
      "현재 Backtest 데이터 권한으로 결과를 볼 수 없습니다. 표시 중이던 결과는 지웠습니다.",
    forbiddenTitle: "시뮬레이션 접근이 차단되었습니다",
    forbiddenMessage: "서버가 선택한 실행 접근을 거절했습니다. 결과 프로필을 표시하지 않습니다.",
    unavailableTitle: "시뮬레이션 서비스를 사용할 수 없습니다",
    unavailableMessage:
      "서비스가 선택한 실행을 확인하지 못했습니다. 새로 고침하여 현재 상태를 확인하세요.",
    integrityTitle: "시뮬레이션 결과를 검증할 수 없습니다",
    integrityMessage:
      "전체 결과 쌍이 고정 결과 계약과 일치하지 않습니다. 지표를 표시하지 않습니다.",
    genericErrorTitle: "시뮬레이션 요청 실패",
    genericErrorMessage: "요청을 완료하지 못했습니다. 현재 상태를 확인한 뒤 다시 시도하세요.",
    cancelLabel: "취소 요청",
    cancellingLabel: "취소 요청 중…",
    cancelAcknowledgedMessage: "취소 접수를 확인했습니다. 서버의 현재 실행 상태를 새로 조회합니다.",
    manualRefreshLabel: "선택한 실행 새로 고침",
    refreshingLabel: "새로 고침 중…",
    pollingExhaustedMessage:
      "상태 자동 조회를 60회 뒤 멈췄습니다. 수동 새로 고침으로 다시 확인하세요.",
    historyLoadError: "시뮬레이션 기록을 불러오지 못했습니다.",
    reportSectionLabel: "검증된 전체 시뮬레이션 결과 쌍",
    reportTitle: "전략 및 벤치마크 비교",
    reportDescription:
      "두 비용 프로필은 같은 전략과 같은 비용의 매수 후 보유 벤치마크를 사용합니다. 수익률은 퍼센트이고 벤치마크 차이는 퍼센트포인트입니다. 기간 회전율은 연환산하지 않으며 매수·매도 원금 합계의 절반을 기준으로 합니다.",
    illustrativeProfileTitle: "예시 비용: 양쪽 10bp",
    zeroCostProfileTitle: "비용 0 기준",
    strategyColumn: "선택 전략",
    benchmarkColumn: "매수 후 보유 벤치마크",
    metricColumn: "허용 지표",
    periodReturnLabel: "기간 수익률",
    endingNavLabel: "기말 순자산",
    endingCashLabel: "기말 현금",
    endingHoldingsLabel: "기말 보유량",
    cumulativeCostsLabel: "누적 비용",
    buyNotionalLabel: "매수 원금",
    sellNotionalLabel: "매도 원금",
    maximumDrawdownLabel: "최대 낙폭",
    periodTurnoverLabel: "기간 회전율 (원금 절반 기준, 비연환산)",
    fillCountLabel: "체결 수",
    benchmarkDifferenceLabel: "벤치마크 수익률 차이 (퍼센트포인트)",
    dailyReturnsTitle: "일별 수익률",
    dailyReturnsCaption: (shown, total) =>
      `최근 평가 세션 ${shown}개 / 전체 ${total}개를 표시합니다. 날짜만 사용합니다.`,
    dailyDateColumn: "거래일",
    dailyIllustrativeStrategyColumn: "전략 · 10bp",
    dailyIllustrativeBenchmarkColumn: "벤치마크 · 10bp",
    dailyZeroStrategyColumn: "전략 · 비용 0",
    dailyZeroBenchmarkColumn: "벤치마크 · 비용 0",
    exactValueLabel: "저장된 정확한 값",
    sharesLabel: "주",
    zeroSharesLabel: "0주",
    notApplicableLabel: "해당 없음",
  },
};
