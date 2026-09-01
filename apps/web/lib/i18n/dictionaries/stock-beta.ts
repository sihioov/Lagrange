import type { LocaleDictionary } from "@/lib/i18n/locale";

export type StockBetaDictionary = {
  readonly activityPolicy: string;
  readonly activityProxyLabel: string;
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
  readonly conditionPolicy: string;
  readonly contractFailureMessage: string;
  readonly coverageLabel: string;
  readonly coverageTargetLabel: string;
  readonly detailDescription: string;
  readonly detailEyebrow: string;
  readonly detailTitle: (instrument: string) => string;
  readonly disable: string;
  readonly disableConfirmation: string;
  readonly disablePrompt: string;
  readonly disableSuccess: string;
  readonly disabling: string;
  readonly disabledLabel: string;
  readonly drawdown120Label: string;
  readonly emptyMembershipsMessage: string;
  readonly emptyMembershipsTitle: string;
  readonly failureCodeLabel: string;
  readonly failedLabel: string;
  readonly firstSessionLabel: string;
  readonly genericUnavailableMessage: string;
  readonly genericUnavailableTitle: string;
  readonly informationUnavailable: string;
  readonly instrumentCodeHint: string;
  readonly instrumentCodeLabel: string;
  readonly instrumentDetailLink: string;
  readonly instrumentNotFoundMessage: string;
  readonly instrumentNotFoundTitle: string;
  readonly integrityMessage: string;
  readonly integrityTitle: string;
  readonly invalidInstrumentCode: string;
  readonly lastSessionLabel: string;
  readonly lifecycleLabel: string;
  readonly lifecycleMaterializing: string;
  readonly lifecycleBackfilling: string;
  readonly lifecycleDisabled: string;
  readonly lifecycleFailed: string;
  readonly lifecycleInsufficientHistory: string;
  readonly lifecycleReady: string;
  readonly lifecycleRequested: string;
  readonly lifecycleValidating: string;
  readonly minimumCoverageLabel: string;
  readonly no: string;
  readonly nonPitPolicy: string;
  readonly notReadyMessage: string;
  readonly notReadyTitle: string;
  readonly ownerOnlyPolicy: string;
  readonly observedCoverageLabel: string;
  readonly originalPricePolicy: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly policyAriaLabel: string;
  readonly policyCapacityDescription: string;
  readonly policyMaxActiveLabel: string;
  readonly pollingMessage: string;
  readonly pollErrorMessage: string;
  readonly publishedAtLabel: string;
  readonly rankLabel: string;
  readonly rankTableCaption: string;
  readonly rankTableHeading: string;
  readonly readySignalsDescription: string;
  readonly remainingCapacityLabel: string;
  readonly requestFailure: (code: string) => string;
  readonly requestedAtLabel: string;
  readonly retry: string;
  readonly retryNotAvailable: string;
  readonly retrying: string;
  readonly retrySuccess: string;
  readonly return120Label: string;
  readonly return20Label: string;
  readonly return60Label: string;
  readonly scoreLabel: string;
  readonly signalUnavailableMessage: string;
  readonly signalUnavailableTitle: string;
  readonly signalsHeading: string;
  readonly snapshotHashLabel: string;
  readonly snapshotRowsLabel: string;
  readonly snapshotHeading: string;
  readonly snapshotDescription: string;
  readonly sma20Label: string;
  readonly sma60Label: string;
  readonly strictPitLabel: string;
  readonly tableConditionLabel: string;
  readonly targetCoverageLabel: string;
  readonly totalMembershipsLabel: string;
  readonly universeHashLabel: string;
  readonly validatingCode: string;
  readonly vendorSnapshotPolicy: string;
  readonly volatility120Label: string;
  readonly volatility20Label: string;
  readonly volatility60Label: string;
  readonly volumeRatioLabel: string;
  readonly warningLabel: string;
  readonly yes: string;
  readonly noMembershipsYet: string;
  readonly neutralLabel: string;
};

export const stockBetaDictionary: LocaleDictionary<StockBetaDictionary> = {
  en: {
    activityPolicy:
      "Volume and trading value are activity/liquidity proxies, not execution liquidity.",
    activityProxyLabel: "Activity proxy",
    addInstrument: "Add instrument",
    addInstrumentDescription:
      "Enter an exact six-digit KRX stock code. The server validates availability and policy capacity.",
    addInstrumentHeading: "Manage research instruments",
    addInstrumentSuccess: "The instrument was accepted and is now being prepared.",
    addingInstrument: "Adding instrument…",
    activeInstrumentsLabel: "Active",
    asOfLabel: "As of",
    averageVolumeLabel: "20-session average volume",
    backToWorkspace: "Back to stock signal beta",
    bearishLabel: "Bearish",
    bullishLabel: "Bullish",
    cancel: "Cancel",
    capacityLabel: "Policy capacity",
    conditionLabel: "Condition",
    conditionPolicy:
      "BULLISH / NEUTRAL / BEARISH are condition labels, not probabilities, target prices, buy/sell calls, weights, or orders.",
    contractFailureMessage:
      "The server returned data outside the approved contract. No unverified content is shown.",
    coverageLabel: "History coverage",
    coverageTargetLabel: "target",
    detailDescription:
      "Inspect the approved price-and-volume signal snapshot for one configured research instrument.",
    detailEyebrow: "Instrument detail",
    detailTitle: (instrument) => `Stock signal beta · ${instrument}`,
    disable: "Disable",
    disableConfirmation: "Confirm disable",
    disablePrompt:
      "Disable this instrument? This is a soft disable; the record remains visible and no new collection is requested.",
    disableSuccess: "The instrument was soft-disabled.",
    disabling: "Disabling…",
    disabledLabel: "Disabled",
    drawdown120Label: "120-session maximum drawdown",
    emptyMembershipsMessage:
      "No research instruments are configured yet. Add an exact six-digit KRX stock code to begin.",
    emptyMembershipsTitle: "No configured instruments",
    failureCodeLabel: "Typed failure",
    failedLabel: "Failed",
    firstSessionLabel: "First session",
    genericUnavailableMessage:
      "The stock signal beta is unavailable. No stale, synthetic, or unverified fallback data is shown.",
    genericUnavailableTitle: "Stock signal beta unavailable",
    informationUnavailable: "Information unavailable",
    instrumentCodeHint: "Six digits only; names and arbitrary URLs are not accepted.",
    instrumentCodeLabel: "KRX stock code",
    instrumentDetailLink: "Open signal detail",
    instrumentNotFoundMessage: "No approved signal row matches this configured instrument.",
    instrumentNotFoundTitle: "Instrument signal not found",
    integrityMessage:
      "The approved signal snapshot failed its integrity check. No signal rows are shown.",
    integrityTitle: "Signal snapshot integrity failed",
    invalidInstrumentCode: "Enter exactly six ASCII digits.",
    lastSessionLabel: "Last session",
    lifecycleLabel: "Lifecycle",
    lifecycleMaterializing: "Materializing",
    lifecycleBackfilling: "Backfilling",
    lifecycleDisabled: "Disabled",
    lifecycleFailed: "Failed",
    lifecycleInsufficientHistory: "Insufficient history",
    lifecycleReady: "Ready",
    lifecycleRequested: "Requested",
    lifecycleValidating: "Validating",
    minimumCoverageLabel: "minimum",
    no: "No",
    nonPitPolicy:
      "This view is not point-in-time (PIT): dates identify the snapshot, not historical availability of every observation.",
    notReadyMessage:
      "Signals will appear after at least one configured instrument reaches READY and an approved snapshot is published.",
    notReadyTitle: "Signals are not ready",
    ownerOnlyPolicy:
      "Owner-only configured research instrument universe. It is not current or historical index membership, and it is not the whole market.",
    observedCoverageLabel: "observed",
    originalPricePolicy:
      "Original/unadjusted prices are used; corporate actions can distort returns and drawdowns.",
    pageDescription:
      "A private Owner workspace for a managed, read-only price-and-volume research universe.",
    pageTitle: "Stock signal beta",
    policyAriaLabel: "Stock signal beta policy boundary",
    policyCapacityDescription:
      "Capacity and history targets below come from the active server policy.",
    policyMaxActiveLabel: "Maximum active",
    pollingMessage: "Updating lifecycle status…",
    pollErrorMessage:
      "Lifecycle status could not be refreshed. The last validated state remains visible.",
    publishedAtLabel: "Published at",
    rankLabel: "Rank",
    rankTableCaption: "Full ranked price-and-volume signal table",
    rankTableHeading: "Ranked signal table",
    readySignalsDescription: "Server-ranked rows from the latest approved snapshot.",
    remainingCapacityLabel: "Remaining",
    requestFailure: (code) => `The request was not accepted. Typed failure: ${code}.`,
    requestedAtLabel: "Requested at",
    retry: "Retry preparation",
    retryNotAvailable: "Retry is not available for this lifecycle state.",
    retrying: "Retrying…",
    retrySuccess: "A new preparation request was accepted.",
    return120Label: "120-session return",
    return20Label: "20-session return",
    return60Label: "60-session return",
    scoreLabel: "Score",
    signalUnavailableMessage:
      "The approved signal snapshot is unavailable. No signal rows are shown and no fallback data is substituted.",
    signalUnavailableTitle: "Signal data unavailable",
    signalsHeading: "Latest signals",
    snapshotHashLabel: "Universe hash",
    snapshotRowsLabel: "Rows",
    snapshotHeading: "Snapshot",
    snapshotDescription: "Snapshot metadata is shown with the rows it governs.",
    sma20Label: "20-session moving average",
    sma60Label: "60-session moving average",
    strictPitLabel: "Strict PIT",
    tableConditionLabel: "Condition",
    targetCoverageLabel: "Target coverage",
    totalMembershipsLabel: "Configured",
    universeHashLabel: "Universe hash",
    validatingCode: "Checking…",
    vendorSnapshotPolicy:
      "Signals use a vendor snapshot and are shown only within the approved Owner surface.",
    volatility120Label: "120-session volatility",
    volatility20Label: "20-session volatility",
    volatility60Label: "60-session volatility",
    volumeRatioLabel: "20/60 volume ratio",
    warningLabel: "Policy boundary",
    yes: "Yes",
    noMembershipsYet: "No memberships yet",
    neutralLabel: "Neutral",
  },
  ko: {
    activityPolicy: "거래량과 거래대금은 활동성·유동성 프록시일 뿐 체결 유동성을 뜻하지 않습니다.",
    activityProxyLabel: "활동성 프록시",
    addInstrument: "종목 추가",
    addInstrumentDescription:
      "정확히 6자리인 KRX 종목코드를 입력하세요. 사용 가능 여부와 정책 용량은 서버가 확인합니다.",
    addInstrumentHeading: "연구 종목 관리",
    addInstrumentSuccess: "종목이 접수되었으며 준비를 시작했습니다.",
    addingInstrument: "종목 추가 중…",
    activeInstrumentsLabel: "활성",
    asOfLabel: "기준일",
    averageVolumeLabel: "20거래일 평균 거래량",
    backToWorkspace: "종목 신호 베타로 돌아가기",
    bearishLabel: "하락",
    bullishLabel: "상승",
    cancel: "취소",
    capacityLabel: "정책 용량",
    conditionLabel: "조건",
    conditionPolicy:
      "BULLISH/NEUTRAL/BEARISH는 조건 레이블이며 확률, 목표가, 매수·매도 호출, 비중 또는 주문이 아닙니다.",
    contractFailureMessage:
      "서버가 승인된 계약을 벗어난 데이터를 반환했습니다. 검증되지 않은 내용은 표시하지 않습니다.",
    coverageLabel: "이력 커버리지",
    coverageTargetLabel: "목표",
    detailDescription: "구성된 한 연구 종목의 승인 가격·거래량 신호 스냅샷을 확인합니다.",
    detailEyebrow: "종목 상세",
    detailTitle: (instrument) => `종목 신호 베타 · ${instrument}`,
    disable: "비활성화",
    disableConfirmation: "비활성화 확인",
    disablePrompt:
      "이 종목을 비활성화할까요? 소프트 비활성화이므로 기록은 보이며 새 수집은 요청하지 않습니다.",
    disableSuccess: "종목을 소프트 비활성화했습니다.",
    disabling: "비활성화 중…",
    disabledLabel: "비활성화됨",
    drawdown120Label: "120거래일 최대 낙폭",
    emptyMembershipsMessage:
      "아직 구성된 연구 종목이 없습니다. 정확히 6자리인 KRX 종목코드를 추가하세요.",
    emptyMembershipsTitle: "구성된 종목 없음",
    failureCodeLabel: "유형화된 실패",
    failedLabel: "실패",
    firstSessionLabel: "첫 세션",
    genericUnavailableMessage:
      "종목 신호 베타를 사용할 수 없습니다. 오래된 데이터나 합성·검증되지 않은 대체 데이터는 표시하지 않습니다.",
    genericUnavailableTitle: "종목 신호 베타를 사용할 수 없습니다",
    informationUnavailable: "정보 없음",
    instrumentCodeHint: "숫자 6자리만 허용하며 이름이나 임의 URL은 받지 않습니다.",
    instrumentCodeLabel: "KRX 종목코드",
    instrumentDetailLink: "신호 상세 열기",
    instrumentNotFoundMessage: "이 구성 종목과 일치하는 승인 신호 행이 없습니다.",
    instrumentNotFoundTitle: "종목 신호를 찾을 수 없습니다",
    integrityMessage:
      "승인된 신호 스냅샷의 무결성 검사가 실패했습니다. 신호 행은 표시하지 않습니다.",
    integrityTitle: "신호 스냅샷 무결성 검사 실패",
    invalidInstrumentCode: "ASCII 숫자 6자리를 정확히 입력하세요.",
    lastSessionLabel: "마지막 세션",
    lifecycleLabel: "수명주기",
    lifecycleMaterializing: "구체화 중",
    lifecycleBackfilling: "백필 중",
    lifecycleDisabled: "비활성화됨",
    lifecycleFailed: "실패",
    lifecycleInsufficientHistory: "이력 부족",
    lifecycleReady: "준비됨",
    lifecycleRequested: "요청됨",
    lifecycleValidating: "검증 중",
    minimumCoverageLabel: "최소",
    no: "아니오",
    nonPitPolicy:
      "이 화면은 PIT(시점 일치)를 보장하지 않습니다. 날짜는 스냅샷을 식별할 뿐 모든 관측값의 과거 가용성을 뜻하지 않습니다.",
    notReadyMessage: "구성 종목 하나 이상이 READY가 되고 승인 스냅샷이 발행되면 신호가 표시됩니다.",
    notReadyTitle: "신호 준비 중",
    ownerOnlyPolicy:
      "오너 전용으로 구성된 연구 종목군입니다. 현재 또는 과거 지수 편입 종목이나 전체 시장을 뜻하지 않습니다.",
    observedCoverageLabel: "관측",
    originalPricePolicy:
      "원주가(비조정 가격)를 사용하며 기업행사로 수익률과 낙폭이 왜곡될 수 있습니다.",
    pageDescription:
      "관리되는 읽기 전용 가격·거래량 연구 종목군을 위한 비공개 오너 워크스페이스입니다.",
    pageTitle: "종목 신호 베타",
    policyAriaLabel: "종목 신호 베타 정책 경계",
    policyCapacityDescription: "아래 용량과 이력 목표는 활성 서버 정책에서 가져옵니다.",
    policyMaxActiveLabel: "활성 최대",
    pollingMessage: "수명주기 상태 갱신 중…",
    pollErrorMessage: "수명주기 상태를 갱신하지 못했습니다. 마지막 검증 상태를 표시합니다.",
    publishedAtLabel: "발행 시각",
    rankLabel: "순위",
    rankTableCaption: "전체 가격·거래량 신호 순위 표",
    rankTableHeading: "신호 순위 표",
    readySignalsDescription: "최신 승인 스냅샷의 서버 순위 행입니다.",
    remainingCapacityLabel: "잔여",
    requestFailure: (code) => `요청을 접수하지 못했습니다. 유형화된 실패: ${code}.`,
    requestedAtLabel: "요청 시각",
    retry: "준비 재시도",
    retryNotAvailable: "이 수명주기 상태에서는 재시도할 수 없습니다.",
    retrying: "재시도 중…",
    retrySuccess: "새 준비 요청을 접수했습니다.",
    return120Label: "120거래일 수익률",
    return20Label: "20거래일 수익률",
    return60Label: "60거래일 수익률",
    scoreLabel: "점수",
    signalUnavailableMessage:
      "승인된 신호 스냅샷을 사용할 수 없습니다. 신호 행은 표시하지 않으며 대체 데이터를 사용하지 않습니다.",
    signalUnavailableTitle: "신호 데이터를 사용할 수 없습니다",
    signalsHeading: "최신 신호",
    snapshotHashLabel: "관찰 목록 해시",
    snapshotRowsLabel: "행 수",
    snapshotHeading: "스냅샷",
    snapshotDescription: "스냅샷 메타데이터를 해당 행과 함께 표시합니다.",
    sma20Label: "20거래일 이동평균",
    sma60Label: "60거래일 이동평균",
    strictPitLabel: "엄격한 PIT",
    tableConditionLabel: "조건",
    targetCoverageLabel: "목표 커버리지",
    totalMembershipsLabel: "구성됨",
    universeHashLabel: "관찰 목록 해시",
    validatingCode: "확인 중…",
    vendorSnapshotPolicy: "신호는 벤더 스냅샷을 사용하며 승인된 오너 화면에서만 표시합니다.",
    volatility120Label: "120거래일 변동성",
    volatility20Label: "20거래일 변동성",
    volatility60Label: "60거래일 변동성",
    volumeRatioLabel: "20/60 거래량 비율",
    warningLabel: "정책 경계",
    yes: "예",
    noMembershipsYet: "구성된 종목 없음",
    neutralLabel: "중립",
  },
};
