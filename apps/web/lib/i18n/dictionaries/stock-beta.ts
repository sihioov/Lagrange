import type { LocaleDictionary } from "@/lib/i18n/locale";

export type StockBetaDictionary = {
  readonly activityDescription: string;
  readonly activityHeading: string;
  readonly activityPolicy: string;
  readonly activityProxyLabel: string;
  readonly activityTabLabel: string;
  readonly betaLabel: string;
  readonly columnViewHeading: string;
  readonly columnViewLabel: string;
  readonly addInstrument: string;
  readonly addInstrumentDescription: string;
  readonly addInstrumentHeading: string;
  readonly addInstrumentSuccess: string;
  readonly addingInstrument: string;
  readonly activeInstrumentsLabel: string;
  readonly asOfLabel: string;
  readonly averageVolumeLabel: string;
  readonly backToWorkspace: string;
  readonly bearishLabel: string;
  readonly bullishLabel: string;
  readonly cancel: string;
  readonly capacityLabel: string;
  readonly conditionLabel: string;
  readonly conditionMatrixDescription: string;
  readonly conditionMatrixHeading: string;
  readonly conditionPolicy: string;
  readonly configuredResultsLabel: string;
  readonly contractFailureMessage: string;
  readonly coverageLabel: string;
  readonly coverageTargetLabel: string;
  readonly currentResultsDescription: string;
  readonly currentSnapshotLabel: string;
  readonly detailLinkLabel: string;
  readonly filtersEyebrow: string;
  readonly detailDescription: string;
  readonly detailEyebrow: string;
  readonly detailTitle: (instrument: string) => string;
  readonly disable: string;
  readonly disableConfirmation: string;
  readonly disablePrompt: string;
  readonly disableSuccess: string;
  readonly disabling: string;
  readonly drawdown120Label: string;
  readonly emptyMembershipsMessage: string;
  readonly emptyMembershipsTitle: string;
  readonly failureCodeLabel: string;
  readonly firstSessionLabel: string;
  readonly generationLabel: string;
  readonly genericUnavailableMessage: string;
  readonly genericUnavailableTitle: string;
  readonly instrumentCodeHint: string;
  readonly instrumentCodeLabel: string;
  readonly instrumentDetailLink: string;
  readonly instrumentHeaderDescription: string;
  readonly instrumentLabel: string;
  readonly instrumentNotFoundMessage: string;
  readonly instrumentNotFoundTitle: string;
  readonly integrityMessage: string;
  readonly integrityTitle: string;
  readonly invalidInstrumentCode: string;
  readonly lastSessionLabel: string;
  readonly lifecycleLabel: string;
  readonly lifecycleBackfilling: string;
  readonly lifecycleDisabled: string;
  readonly lifecycleFailed: string;
  readonly lifecycleInsufficientHistory: string;
  readonly lifecycleMaterializing: string;
  readonly lifecycleReady: string;
  readonly lifecycleRequested: string;
  readonly lifecycleValidating: string;
  readonly membershipStatusHeading: string;
  readonly minimumCoverageLabel: string;
  readonly neutralLabel: string;
  readonly nonPitPolicy: string;
  readonly noResultsMessage: string;
  readonly noSearchResultsMessage: string;
  readonly notReadyMessage: string;
  readonly notReadyTitle: string;
  readonly previewEmptyMessage: string;
  readonly observedCoverageLabel: string;
  readonly openDetailLabel: string;
  readonly originalPricePolicy: string;
  readonly ownerOnlyPolicy: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly policyAriaLabel: string;
  readonly policyBoundaryDescription: string;
  readonly policyBoundaryDetailsLabel: string;
  readonly policyBoundaryHeading: string;
  readonly policyBoundarySummary: string;
  readonly policyMaxActiveLabel: string;
  readonly pollErrorMessage: string;
  readonly pollingMessage: string;
  readonly provenanceDescription: string;
  readonly provenanceDisclosureLabel: string;
  readonly provenanceHeading: string;
  readonly publishedAtLabel: string;
  readonly rankLabel: string;
  readonly rankTableCaption: string;
  readonly rankTableHeading: string;
  readonly rawValueLabel: string;
  readonly readOnlyBadgeLabel: string;
  readonly remainingCapacityLabel: string;
  readonly requestFailure: (code: string) => string;
  readonly resultCountLabel: string;
  readonly retry: string;
  readonly retrying: string;
  readonly retrySuccess: string;
  readonly return120Label: string;
  readonly return20Label: string;
  readonly return60Label: string;
  readonly returnsDescription: string;
  readonly returnsHeading: string;
  readonly returnsTabLabel: string;
  readonly riskDescription: string;
  readonly riskHeading: string;
  readonly scoreLabel: string;
  readonly searchHint: string;
  readonly searchLabel: string;
  readonly searchMatchesLabel: string;
  readonly searchPlaceholder: string;
  readonly selectForPreview: string;
  readonly signalDecompositionDescription: string;
  readonly signalDecompositionHeading: string;
  readonly signalMetricsHeading: string;
  readonly signalProfileDescription: string;
  readonly signalProfileHeading: string;
  readonly signalUnavailableMessage: string;
  readonly signalUnavailableTitle: string;
  readonly signalsHeading: string;
  readonly sma20Label: string;
  readonly sma60Label: string;
  readonly snapshotDescription: string;
  readonly snapshotHeading: string;
  readonly snapshotIdLabel: string;
  readonly snapshotRowsLabel: string;
  readonly snapshotTapeDescription: string;
  readonly snapshotTapeHeading: string;
  readonly tableConditionLabel: string;
  readonly targetCoverageLabel: string;
  readonly terminalContextLabel: string;
  readonly topFiveHeading: string;
  readonly topFiveInTableLabel: string;
  readonly totalMembershipsLabel: string;
  readonly universeHashLabel: string;
  readonly universeLabel: string;
  readonly trendGroupHeading: string;
  readonly volatility120Label: string;
  readonly volatility20Label: string;
  readonly volatility60Label: string;
  readonly volatilityGroupHeading: string;
  readonly volatilityTabLabel: string;
  readonly volumeRatioLabel: string;
  readonly vendorSnapshotPolicy: string;
  readonly warningLabel: string;
  readonly zeroAxisLabel: string;
};

export const stockBetaDictionary: LocaleDictionary<StockBetaDictionary> = {
  en: {
    activityDescription:
      "Moving averages, volume, volume ratio, and trading-value activity from the V2 signal row.",
    activityHeading: "Trend and activity",
    activityPolicy:
      "Volume and trading value are activity/liquidity proxies, not execution liquidity.",
    activityProxyLabel: "20-session trading-value activity",
    activityTabLabel: "Activity",
    betaLabel: "BETA",
    columnViewHeading: "Visible ranking columns",
    columnViewLabel: "Columns",
    addInstrument: "Add instrument",
    addInstrumentDescription:
      "Enter one exact six-digit KRX code; capacity and preparation are server-managed.",
    addInstrumentHeading: "V2 universe management",
    addInstrumentSuccess: "The instrument was accepted and preparation has started.",
    addingInstrument: "Adding…",
    activeInstrumentsLabel: "Active",
    asOfLabel: "As of",
    averageVolumeLabel: "20-session average volume",
    backToWorkspace: "Back to stock signal beta",
    bearishLabel: "Bearish",
    bullishLabel: "Bullish",
    cancel: "Cancel",
    capacityLabel: "Policy capacity",
    conditionLabel: "Condition",
    conditionMatrixDescription:
      "Equal-area tiles preserve server order and the returned condition.",
    conditionMatrixHeading: "Condition matrix",
    conditionPolicy:
      "BULLISH / NEUTRAL / BEARISH are conditions, not probabilities, target prices, buy/sell calls, weights, or orders.",
    configuredResultsLabel: "Current V2 snapshot",
    contractFailureMessage:
      "The server returned data outside the approved V2 contract. No unverified content is shown.",
    coverageLabel: "Coverage",
    coverageTargetLabel: "target",
    currentResultsDescription: "Current V2 rows in server order.",
    currentSnapshotLabel: "Current snapshot",
    detailLinkLabel: "Detail",
    filtersEyebrow: "V2 universe",
    detailDescription: "Inspect one V2 price-and-volume signal without V1 fallback data.",
    detailEyebrow: "V2 signal",
    detailTitle: (instrument) => `Stock signal beta · ${instrument}`,
    disable: "Disable",
    disableConfirmation: "Confirm disable",
    disablePrompt:
      "Soft-disable this membership? Its evidence remains retained while its signal is hidden.",
    disableSuccess: "The membership was disabled.",
    disabling: "Disabling…",
    drawdown120Label: "120-session maximum drawdown",
    emptyMembershipsMessage:
      "No research instruments are configured. Add an exact six-digit KRX code.",
    emptyMembershipsTitle: "No configured instruments",
    failureCodeLabel: "Typed failure",
    firstSessionLabel: "First session",
    generationLabel: "Generation",
    genericUnavailableMessage:
      "Stock signal beta is unavailable. No stale, synthetic, or V1 fallback data is shown.",
    genericUnavailableTitle: "Stock signal beta unavailable",
    instrumentCodeHint: "Exactly six ASCII digits; names and arbitrary URLs are not accepted.",
    instrumentCodeLabel: "KRX code",
    instrumentDetailLink: "Open detail",
    instrumentHeaderDescription:
      "Server-supplied V2 instrument ID, generation, rank, score, condition, and as-of.",
    instrumentLabel: "Instrument ID",
    instrumentNotFoundMessage: "No current V2 signal matches this membership.",
    instrumentNotFoundTitle: "Instrument signal not found",
    integrityMessage: "The V2 snapshot failed its integrity boundary. No rows are shown.",
    integrityTitle: "Signal snapshot integrity failed",
    invalidInstrumentCode: "Enter exactly six ASCII digits.",
    lastSessionLabel: "Last session",
    lifecycleLabel: "Lifecycle",
    lifecycleBackfilling: "Backfilling",
    lifecycleDisabled: "Disabled",
    lifecycleFailed: "Failed",
    lifecycleInsufficientHistory: "Insufficient history",
    lifecycleMaterializing: "Materializing",
    lifecycleReady: "Ready",
    lifecycleRequested: "Requested",
    lifecycleValidating: "Validating",
    membershipStatusHeading: "Membership status",
    minimumCoverageLabel: "minimum",
    neutralLabel: "Neutral",
    nonPitPolicy: "This is not strict point-in-time evidence; dates identify the current snapshot.",
    noResultsMessage: "The current V2 snapshot has no signal rows.",
    noSearchResultsMessage: "No instrument ID in this response matches the search.",
    notReadyMessage: "Manage memberships while the first READY universe snapshot is prepared.",
    notReadyTitle: "Signals are not ready",
    observedCoverageLabel: "observed",
    openDetailLabel: "Open detail",
    originalPricePolicy:
      "Original/unadjusted prices are used; corporate actions can distort returns and drawdowns.",
    ownerOnlyPolicy:
      "Owner-only managed KRX research instruments; this is not index membership or the whole market.",
    previewEmptyMessage: "Select a V2 signal row to inspect its returned metrics.",
    pageDescription: "Private Owner-managed V2 price-and-volume research workspace.",
    pageTitle: "Stock signal beta",
    policyAriaLabel: "Stock signal beta policy boundary",
    policyBoundaryDescription: "Read-only V2 price-and-volume evidence.",
    policyBoundaryDetailsLabel: "Read the complete policy boundary",
    policyBoundaryHeading: "Research policy boundary",
    policyBoundarySummary: "Owner only · Read only · Original price · No account or order actions",
    policyMaxActiveLabel: "Maximum",
    pollErrorMessage: "Status refresh failed; the last validated membership state remains visible.",
    pollingMessage: "Refreshing lifecycle and snapshot state…",
    provenanceDescription: "Only the actual V2 snapshot identifiers returned by the API.",
    provenanceDisclosureLabel: "Show V2 snapshot fields",
    provenanceHeading: "V2 snapshot",
    publishedAtLabel: "Published",
    rankLabel: "Rank",
    rankTableCaption: "V2 ranked price-and-volume signal table",
    rankTableHeading: "Ranked signals",
    rawValueLabel: "API value",
    readOnlyBadgeLabel: "READ ONLY",
    remainingCapacityLabel: "Remaining",
    requestFailure: (code) => `Request failed with typed code ${code}.`,
    resultCountLabel: "Rows",
    retry: "Retry",
    retrying: "Retrying…",
    retrySuccess: "A new preparation request was accepted.",
    return120Label: "120-session return",
    return20Label: "20-session return",
    return60Label: "60-session return",
    returnsDescription: "Raw 20-, 60-, and 120-session returns from this V2 row.",
    returnsHeading: "Returns",
    returnsTabLabel: "Returns",
    riskDescription: "Raw volatility, drawdown, and moving-average values from this V2 row.",
    riskHeading: "Risk and price levels",
    scoreLabel: "Score",
    searchHint: "Instrument IDs in this response only",
    searchLabel: "Search signals",
    searchMatchesLabel: "matches",
    searchPlaceholder: "Instrument ID",
    selectForPreview: "Select signal",
    signalDecompositionDescription:
      "Returned score, condition, generation, and numeric row fields—without inferred factors.",
    signalDecompositionHeading: "Signal decomposition",
    signalMetricsHeading: "Signal metrics",
    signalProfileDescription: "Compare the exact V2 row values by horizon.",
    signalProfileHeading: "Selected signal profile",
    signalUnavailableMessage:
      "No current V2 snapshot is available. Manage memberships; no stale signals are shown.",
    signalUnavailableTitle: "Signal snapshot unavailable",
    signalsHeading: "Latest V2 signals",
    sma20Label: "20-session moving average",
    sma60Label: "60-session moving average",
    snapshotDescription: "Actual V2 snapshot identity and publication fields.",
    snapshotHeading: "Snapshot",
    snapshotIdLabel: "Snapshot ID",
    snapshotRowsLabel: "Rows",
    snapshotTapeDescription: "Current V2 leaders in the server-supplied order.",
    snapshotTapeHeading: "Current snapshot tape",
    tableConditionLabel: "Condition",
    targetCoverageLabel: "Target",
    terminalContextLabel: "OWNER ONLY · READ ONLY · ORIGINAL PRICE",
    topFiveHeading: "Top 5",
    topFiveInTableLabel: "API top-five rows are emphasized without reranking.",
    totalMembershipsLabel: "Memberships",
    trendGroupHeading: "Drawdown and moving averages",
    universeHashLabel: "Universe SHA-256",
    universeLabel: "Universe",
    volatility120Label: "120-session volatility",
    volatility20Label: "20-session volatility",
    volatility60Label: "60-session volatility",
    volatilityGroupHeading: "Volatility",
    volatilityTabLabel: "Volatility",
    volumeRatioLabel: "20/60 volume ratio",
    vendorSnapshotPolicy: "Signals are shown only from the current published V2 snapshot.",
    warningLabel: "Boundary",
    zeroAxisLabel: "Zero axis",
  },
  ko: {
    activityDescription: "V2 신호 행의 이동평균, 거래량, 거래량 비율 및 거래대금 활동성입니다.",
    activityHeading: "추세와 활동성",
    activityPolicy: "거래량과 거래대금은 활동성·유동성 프록시이며 체결 유동성이 아닙니다.",
    activityProxyLabel: "20거래일 거래대금 활동성",
    activityTabLabel: "활동성",
    betaLabel: "BETA",
    columnViewHeading: "표시할 순위 열",
    columnViewLabel: "열",
    addInstrument: "종목 추가",
    addInstrumentDescription:
      "정확한 KRX 코드 6자리 하나를 입력하세요. 용량과 준비 과정은 서버가 관리합니다.",
    addInstrumentHeading: "V2 유니버스 관리",
    addInstrumentSuccess: "종목을 접수했고 준비를 시작했습니다.",
    addingInstrument: "추가 중…",
    activeInstrumentsLabel: "활성",
    asOfLabel: "기준일",
    averageVolumeLabel: "20거래일 평균 거래량",
    backToWorkspace: "종목 신호 베타로 돌아가기",
    bearishLabel: "하락",
    bullishLabel: "상승",
    cancel: "취소",
    capacityLabel: "정책 용량",
    conditionLabel: "조건",
    conditionMatrixDescription: "동일 면적 타일이 서버 순서와 반환 조건을 보존합니다.",
    conditionMatrixHeading: "조건 매트릭스",
    conditionPolicy:
      "BULLISH/NEUTRAL/BEARISH는 조건이며 확률, 목표가, 매수·매도, 비중 또는 주문이 아닙니다.",
    configuredResultsLabel: "현재 V2 스냅샷",
    contractFailureMessage:
      "서버가 승인된 V2 계약 밖의 데이터를 반환했습니다. 검증되지 않은 내용은 표시하지 않습니다.",
    coverageLabel: "커버리지",
    coverageTargetLabel: "목표",
    currentResultsDescription: "서버 순서를 보존한 현재 V2 행입니다.",
    currentSnapshotLabel: "현재 스냅샷",
    detailLinkLabel: "상세",
    filtersEyebrow: "V2 유니버스",
    detailDescription: "V1 대체 데이터 없이 한 V2 가격·거래량 신호를 확인합니다.",
    detailEyebrow: "V2 신호",
    detailTitle: (instrument) => `종목 신호 베타 · ${instrument}`,
    disable: "비활성화",
    disableConfirmation: "비활성화 확인",
    disablePrompt: "이 멤버십을 소프트 비활성화할까요? 근거는 보존되고 신호는 즉시 숨깁니다.",
    disableSuccess: "멤버십을 비활성화했습니다.",
    disabling: "비활성화 중…",
    drawdown120Label: "120거래일 최대 낙폭",
    emptyMembershipsMessage: "구성된 연구 종목이 없습니다. 정확한 KRX 코드 6자리를 추가하세요.",
    emptyMembershipsTitle: "구성 종목 없음",
    failureCodeLabel: "유형화된 실패",
    firstSessionLabel: "첫 세션",
    generationLabel: "세대",
    genericUnavailableMessage:
      "종목 신호 베타를 사용할 수 없습니다. 오래된·합성·V1 대체 데이터는 표시하지 않습니다.",
    genericUnavailableTitle: "종목 신호 베타 사용 불가",
    instrumentCodeHint: "ASCII 숫자 6자리만 허용하며 이름이나 임의 URL은 받지 않습니다.",
    instrumentCodeLabel: "KRX 코드",
    instrumentDetailLink: "상세 열기",
    instrumentHeaderDescription:
      "서버가 반환한 V2 종목 ID, 세대, 순위, 점수, 조건 및 기준일입니다.",
    instrumentLabel: "종목 ID",
    instrumentNotFoundMessage: "이 멤버십과 일치하는 현재 V2 신호가 없습니다.",
    instrumentNotFoundTitle: "종목 신호 없음",
    integrityMessage: "V2 스냅샷이 무결성 경계를 통과하지 못했습니다. 행을 표시하지 않습니다.",
    integrityTitle: "신호 스냅샷 무결성 실패",
    invalidInstrumentCode: "ASCII 숫자 6자리를 정확히 입력하세요.",
    lastSessionLabel: "마지막 세션",
    lifecycleLabel: "수명주기",
    lifecycleBackfilling: "백필 중",
    lifecycleDisabled: "비활성",
    lifecycleFailed: "실패",
    lifecycleInsufficientHistory: "이력 부족",
    lifecycleMaterializing: "구체화 중",
    lifecycleReady: "준비됨",
    lifecycleRequested: "요청됨",
    lifecycleValidating: "검증 중",
    membershipStatusHeading: "멤버십 상태",
    minimumCoverageLabel: "최소",
    neutralLabel: "중립",
    nonPitPolicy: "엄격한 PIT 근거가 아니며 날짜는 현재 스냅샷을 식별합니다.",
    noResultsMessage: "현재 V2 스냅샷에 신호 행이 없습니다.",
    noSearchResultsMessage: "현재 응답에서 검색어와 일치하는 종목 ID가 없습니다.",
    notReadyMessage: "첫 READY 유니버스 스냅샷이 준비되는 동안 멤버십을 관리하세요.",
    notReadyTitle: "신호 준비 중",
    observedCoverageLabel: "관측",
    openDetailLabel: "상세 열기",
    originalPricePolicy:
      "원주가(비조정 가격)를 사용하며 기업행사로 수익률과 낙폭이 왜곡될 수 있습니다.",
    ownerOnlyPolicy:
      "오너 전용 관리형 KRX 연구 종목이며 지수 편입이나 전체 시장을 뜻하지 않습니다.",
    previewEmptyMessage: "반환 지표를 보려면 V2 신호 행을 선택하세요.",
    pageDescription: "오너 관리형 V2 가격·거래량 비공개 연구 워크스페이스입니다.",
    pageTitle: "종목 신호 베타",
    policyAriaLabel: "종목 신호 베타 정책 경계",
    policyBoundaryDescription: "읽기 전용 V2 가격·거래량 근거입니다.",
    policyBoundaryDetailsLabel: "전체 정책 경계 읽기",
    policyBoundaryHeading: "연구 정책 경계",
    policyBoundarySummary: "오너 전용 · 읽기 전용 · 원주가 · 계좌/주문 없음",
    policyMaxActiveLabel: "최대",
    pollErrorMessage: "상태 갱신에 실패해 마지막으로 검증된 멤버십 상태를 표시합니다.",
    pollingMessage: "수명주기와 스냅샷 상태 갱신 중…",
    provenanceDescription: "API가 반환한 실제 V2 스냅샷 식별자만 표시합니다.",
    provenanceDisclosureLabel: "V2 스냅샷 필드 보기",
    provenanceHeading: "V2 스냅샷",
    publishedAtLabel: "발행",
    rankLabel: "순위",
    rankTableCaption: "V2 가격·거래량 신호 순위 표",
    rankTableHeading: "신호 순위",
    rawValueLabel: "API 값",
    readOnlyBadgeLabel: "읽기 전용",
    remainingCapacityLabel: "잔여",
    requestFailure: (code) => `유형화된 코드 ${code}로 요청이 실패했습니다.`,
    resultCountLabel: "행",
    retry: "재시도",
    retrying: "재시도 중…",
    retrySuccess: "새 준비 요청을 접수했습니다.",
    return120Label: "120거래일 수익률",
    return20Label: "20거래일 수익률",
    return60Label: "60거래일 수익률",
    returnsDescription: "이 V2 행의 원본 20·60·120거래일 수익률입니다.",
    returnsHeading: "수익률",
    returnsTabLabel: "수익률",
    riskDescription: "이 V2 행의 원본 변동성, 낙폭 및 이동평균 값입니다.",
    riskHeading: "위험과 가격 수준",
    scoreLabel: "점수",
    searchHint: "현재 응답의 종목 ID만",
    searchLabel: "신호 검색",
    searchMatchesLabel: "개 일치",
    searchPlaceholder: "종목 ID",
    selectForPreview: "신호 선택",
    signalDecompositionDescription: "추론 팩터 없이 반환된 점수, 조건, 세대 및 숫자 행 필드입니다.",
    signalDecompositionHeading: "신호 분해",
    signalMetricsHeading: "신호 지표",
    signalProfileDescription: "정확한 V2 행 값을 기간별로 비교합니다.",
    signalProfileHeading: "선택 신호 프로필",
    signalUnavailableMessage:
      "현재 V2 스냅샷이 없습니다. 멤버십을 관리할 수 있으며 오래된 신호는 표시하지 않습니다.",
    signalUnavailableTitle: "신호 스냅샷 없음",
    signalsHeading: "최신 V2 신호",
    sma20Label: "20거래일 이동평균",
    sma60Label: "60거래일 이동평균",
    snapshotDescription: "실제 V2 스냅샷 식별 및 발행 필드입니다.",
    snapshotHeading: "스냅샷",
    snapshotIdLabel: "스냅샷 ID",
    snapshotRowsLabel: "행 수",
    snapshotTapeDescription: "서버가 제공한 순서의 현재 V2 선두 행입니다.",
    snapshotTapeHeading: "현재 스냅샷 테이프",
    tableConditionLabel: "조건",
    targetCoverageLabel: "목표",
    terminalContextLabel: "오너 전용 · 읽기 전용 · 원주가",
    topFiveHeading: "Top 5",
    topFiveInTableLabel: "API Top 5 행을 재순위 없이 강조합니다.",
    totalMembershipsLabel: "멤버십",
    trendGroupHeading: "낙폭과 이동평균",
    universeHashLabel: "유니버스 SHA-256",
    universeLabel: "유니버스",
    volatility120Label: "120거래일 변동성",
    volatility20Label: "20거래일 변동성",
    volatility60Label: "60거래일 변동성",
    volatilityGroupHeading: "변동성",
    volatilityTabLabel: "변동성",
    volumeRatioLabel: "20/60 거래량 비율",
    vendorSnapshotPolicy: "현재 발행된 V2 스냅샷의 신호만 표시합니다.",
    warningLabel: "경계",
    zeroAxisLabel: "0축",
  },
};
