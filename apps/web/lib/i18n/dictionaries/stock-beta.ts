import type { LocaleDictionary } from "@/lib/i18n/locale";

export type StockBetaDictionary = {
  readonly activityLabel: string;
  readonly activityPolicy: string;
  readonly activityProxyLabel: string;
  readonly applyFilters: string;
  readonly asOfLabel: string;
  readonly artifactHashLabel: string;
  readonly audienceLabel: string;
  readonly backToWorkspace: string;
  readonly batchIdLabel: string;
  readonly bearishLabel: string;
  readonly bullishLabel: string;
  readonly capabilityLabel: string;
  readonly clearFilters: string;
  readonly conditionLabel: string;
  readonly conditionPolicy: string;
  readonly conditionReasonsDescription: string;
  readonly conditionReasonsHeading: string;
  readonly detailDescription: string;
  readonly detailEyebrow: string;
  readonly detailTitle: (instrument: string) => string;
  readonly drawdown120Label: string;
  readonly entitlementHashLabel: string;
  readonly factorLabel: string;
  readonly factorVersionLabel: string;
  readonly fixedListPolicy: string;
  readonly filtersDescription: string;
  readonly filtersEyebrow: string;
  readonly filtersHeading: string;
  readonly genericUnavailableMessage: string;
  readonly genericUnavailableTitle: string;
  readonly indexMembershipLabel: string;
  readonly instrumentLabel: string;
  readonly integrityMessage: string;
  readonly integrityTitle: string;
  readonly interpretationLabel: string;
  readonly invalidFiltersMessage: string;
  readonly invalidFiltersTitle: string;
  readonly instrumentNotFoundMessage: string;
  readonly instrumentNotFoundTitle: string;
  readonly latestDescription: string;
  readonly maxLabel: string;
  readonly materializationStatusLabel: string;
  readonly minLabel: string;
  readonly noReasons: string;
  readonly noResultsMessage: string;
  readonly noResultsTitle: string;
  readonly neutralLabel: string;
  readonly notReported: string;
  readonly originalPriceLabel: string;
  readonly originalPricePolicy: string;
  readonly provenanceDescription: string;
  readonly provenanceHeading: string;
  readonly publicationStatusLabel: string;
  readonly rankLabel: string;
  readonly rankTableCaption: string;
  readonly rankTableEyebrow: string;
  readonly rankTableHeading: string;
  readonly registryHashLabel: string;
  readonly redistributionLabel: string;
  readonly registrationStatusLabel: string;
  readonly return120Label: string;
  readonly return20Label: string;
  readonly return60Label: string;
  readonly scoreLabel: string;
  readonly selectionBasisLabel: string;
  readonly signalMetricsHeading: string;
  readonly signalUnavailableMessage: string;
  readonly signalUnavailableTitle: string;
  readonly snapshotHashLabel: string;
  readonly sma20Label: string;
  readonly sma60Label: string;
  readonly strictPitLabel: string;
  readonly tableConditionLabel: string;
  readonly topFiveDescription: string;
  readonly topFiveEyebrow: string;
  readonly topFiveHeading: string;
  readonly trendLabel: string;
  readonly trendDownLabel: string;
  readonly trendUpLabel: string;
  readonly universeHashLabel: string;
  readonly valueLabel: string;
  readonly vendorSnapshotLabel: string;
  readonly volatility120Label: string;
  readonly volatility20Label: string;
  readonly volatility60Label: string;
  readonly volumeRatioLabel: string;
  readonly averageVolumeLabel: string;
  readonly warningLabel: string;
  readonly yes: string;
  readonly no: string;
  readonly pageDescription: string;
  readonly pageTitle: string;
  readonly policyAriaLabel: string;
};

export const stockBetaDictionary: LocaleDictionary<StockBetaDictionary> = {
  en: {
    activityLabel: "20-session trading-value activity proxy",
    activityPolicy: "Volume and activity are an activity/liquidity proxy, not execution liquidity.",
    activityProxyLabel: "Activity proxy",
    applyFilters: "Apply filters",
    asOfLabel: "As of",
    artifactHashLabel: "Artifact content hash",
    audienceLabel: "Audience",
    backToWorkspace: "Back to stock signal beta",
    batchIdLabel: "Batch ID",
    bearishLabel: "Bearish",
    bullishLabel: "Bullish",
    capabilityLabel: "Capability",
    clearFilters: "Clear filters",
    conditionLabel: "Scenario condition",
    conditionPolicy:
      "BULLISH / NEUTRAL / BEARISH are condition labels, not probabilities, target prices, buy/sell calls, weights, or orders.",
    conditionReasonsDescription:
      "These reasons are returned by the API for the selected condition. They are evidence clauses, not forecasts.",
    conditionReasonsHeading: "Exact condition reasons",
    detailDescription:
      "Inspect the price-and-volume factors, condition evidence, rank, and API provenance for one configured instrument.",
    detailEyebrow: "Instrument detail",
    detailTitle: (instrument) => `Stock signal beta · ${instrument}`,
    drawdown120Label: "120-session maximum drawdown",
    entitlementHashLabel: "Entitlement hash",
    factorLabel: "Factor",
    factorVersionLabel: "Factor version",
    fixedListPolicy:
      "Owner-only configured fixed observation list — not current or historical index membership, and not the whole market.",
    filtersDescription:
      "Filters are encoded in the URL and applied as a read-only query over the immutable ranked snapshot.",
    filtersEyebrow: "Read-only screen",
    filtersHeading: "Filter the ranked snapshot",
    genericUnavailableMessage:
      "The stock signal beta could not be loaded. No signal rows are shown and no stale or synthetic fallback is substituted.",
    genericUnavailableTitle: "Stock signal beta unavailable",
    indexMembershipLabel: "Index membership",
    instrumentLabel: "Instrument",
    integrityMessage:
      "The approved signal snapshot failed its integrity check. No signal rows are shown and no fallback data is substituted.",
    integrityTitle: "Signal snapshot integrity failed",
    interpretationLabel: "Interpretation",
    invalidFiltersMessage:
      "Check the scenario, trend, and numeric range values, then submit the GET filter form again.",
    invalidFiltersTitle: "Stock signal filters are invalid",
    instrumentNotFoundMessage: "No approved signal row matches this configured instrument.",
    instrumentNotFoundTitle: "Instrument signal not found",
    latestDescription: "The latest approved read-only price-and-volume signal snapshot.",
    maxLabel: "Maximum",
    materializationStatusLabel: "Materialization status",
    minLabel: "Minimum",
    noReasons: "No condition reason was reported by the API.",
    noResultsMessage: "No configured instruments match these filters.",
    noResultsTitle: "No signals match",
    neutralLabel: "Neutral",
    notReported: "Not reported",
    originalPriceLabel: "Original price",
    originalPricePolicy:
      "Original/unadjusted price data is used; corporate actions can distort returns and drawdowns.",
    provenanceDescription:
      "Only the provenance returned by the API is shown; this view does not imply publication beyond that record.",
    provenanceHeading: "API provenance",
    publicationStatusLabel: "Publication status",
    rankLabel: "Rank",
    rankTableCaption: "Full ranked 30-row price-and-volume signal table",
    rankTableEyebrow: "All configured instruments",
    rankTableHeading: "Full ranked table",
    registryHashLabel: "Approval registry hash",
    redistributionLabel: "Redistribution",
    registrationStatusLabel: "Registration status",
    return120Label: "120-session return",
    return20Label: "20-session return",
    return60Label: "60-session return",
    scoreLabel: "Score",
    selectionBasisLabel: "Selection basis",
    signalMetricsHeading: "Signal values",
    signalUnavailableMessage:
      "The approved signal snapshot is unavailable. No signal rows are shown and no fallback data is substituted.",
    signalUnavailableTitle: "Signal data unavailable",
    snapshotHashLabel: "Snapshot content hash",
    sma20Label: "20-session moving average",
    sma60Label: "60-session moving average",
    strictPitLabel: "Strict PIT",
    tableConditionLabel: "Condition",
    topFiveDescription: "The first five rows of the server-ranked snapshot, with rank preserved.",
    topFiveEyebrow: "Priority view",
    topFiveHeading: "Top 5",
    trendLabel: "Trend",
    trendDownLabel: "Trend down",
    trendUpLabel: "Trend up",
    universeHashLabel: "Universe hash",
    valueLabel: "Value",
    vendorSnapshotLabel: "Vendor snapshot",
    volatility120Label: "120-session volatility",
    volatility20Label: "20-session volatility",
    volatility60Label: "60-session volatility",
    volumeRatioLabel: "20/60 volume ratio",
    averageVolumeLabel: "20-session average volume",
    warningLabel: "Policy boundary",
    yes: "Yes",
    no: "No",
    pageDescription:
      "A private Owner workspace for a fixed, read-only price-and-volume research beta.",
    pageTitle: "Stock signal beta",
    policyAriaLabel: "Stock signal beta policy boundary",
  },
  ko: {
    activityLabel: "20거래일 거래대금 활동성 프록시",
    activityPolicy: "거래량과 활동성은 활동·유동성 프록시일 뿐 체결 유동성을 뜻하지 않습니다.",
    activityProxyLabel: "활동성 프록시",
    applyFilters: "필터 적용",
    asOfLabel: "기준일",
    artifactHashLabel: "아티팩트 콘텐츠 해시",
    audienceLabel: "대상",
    backToWorkspace: "종목 신호 베타로 돌아가기",
    batchIdLabel: "배치 ID",
    bearishLabel: "하락",
    bullishLabel: "상승",
    capabilityLabel: "기능",
    clearFilters: "필터 지우기",
    conditionLabel: "시나리오 조건",
    conditionPolicy:
      "BULLISH/NEUTRAL/BEARISH는 조건 레이블이며 확률, 목표가, 매수·매도 신호, 비중 또는 주문이 아닙니다.",
    conditionReasonsDescription:
      "선택한 조건에 대해 API가 반환한 사유입니다. 예측이 아닌 증거 절입니다.",
    conditionReasonsHeading: "정확한 조건 사유",
    detailDescription:
      "구성된 한 종목의 가격·거래량 팩터, 조건 근거, 순위 및 API 출처를 확인하세요.",
    detailEyebrow: "종목 상세",
    detailTitle: (instrument) => `종목 신호 베타 · ${instrument}`,
    drawdown120Label: "120거래일 최대 낙폭",
    entitlementHashLabel: "권한 해시",
    factorLabel: "팩터",
    factorVersionLabel: "팩터 버전",
    fixedListPolicy:
      "오너 전용으로 구성된 고정 관찰 목록입니다. 현재 또는 과거 지수 편입 종목이나 전체 시장을 뜻하지 않습니다.",
    filtersDescription:
      "필터는 URL에 인코딩되며 변경 불가능한 순위 스냅샷에 대한 읽기 전용 조회로 적용됩니다.",
    filtersEyebrow: "읽기 전용 스크리닝",
    filtersHeading: "순위 스냅샷 필터",
    genericUnavailableMessage:
      "종목 신호 베타를 불러오지 못했습니다. 신호 행은 표시하지 않으며 오래된 데이터나 합성 대체 데이터를 사용하지 않습니다.",
    genericUnavailableTitle: "종목 신호 베타를 사용할 수 없습니다",
    indexMembershipLabel: "지수 편입",
    instrumentLabel: "종목",
    integrityMessage:
      "승인된 신호 스냅샷의 무결성 검사가 실패했습니다. 신호 행은 표시하지 않으며 대체 데이터를 사용하지 않습니다.",
    integrityTitle: "신호 스냅샷 무결성 검사 실패",
    interpretationLabel: "해석",
    invalidFiltersMessage:
      "시나리오, 추세 및 숫자 범위를 확인한 후 GET 필터 양식을 다시 제출하세요.",
    invalidFiltersTitle: "종목 신호 필터가 올바르지 않습니다",
    instrumentNotFoundMessage: "구성된 종목과 일치하는 승인 신호 행이 없습니다.",
    instrumentNotFoundTitle: "종목 신호를 찾을 수 없습니다",
    latestDescription: "최신 승인 읽기 전용 가격·거래량 신호 스냅샷입니다.",
    maxLabel: "최대",
    materializationStatusLabel: "구체화 상태",
    minLabel: "최소",
    noReasons: "API가 조건 사유를 보고하지 않았습니다.",
    noResultsMessage: "이 필터와 일치하는 구성 종목이 없습니다.",
    noResultsTitle: "일치하는 신호가 없습니다",
    neutralLabel: "중립",
    notReported: "보고되지 않음",
    originalPriceLabel: "원주가",
    originalPricePolicy:
      "원주가(비조정 가격)를 사용하며 기업행사로 수익률과 낙폭이 왜곡될 수 있습니다.",
    provenanceDescription:
      "API가 반환한 출처만 표시하며, 해당 기록을 넘어선 공개를 의미하지 않습니다.",
    provenanceHeading: "API 출처",
    publicationStatusLabel: "공개 상태",
    rankLabel: "순위",
    rankTableCaption: "구성된 전체 30종목 가격·거래량 신호 순위 표",
    rankTableEyebrow: "전체 구성 종목",
    rankTableHeading: "전체 순위 표",
    registryHashLabel: "승인 레지스트리 해시",
    redistributionLabel: "재배포",
    registrationStatusLabel: "등록 상태",
    return120Label: "120거래일 수익률",
    return20Label: "20거래일 수익률",
    return60Label: "60거래일 수익률",
    scoreLabel: "점수",
    selectionBasisLabel: "선정 기준",
    signalMetricsHeading: "신호 값",
    signalUnavailableMessage:
      "승인된 신호 스냅샷을 사용할 수 없습니다. 신호 행은 표시하지 않으며 대체 데이터를 사용하지 않습니다.",
    signalUnavailableTitle: "신호 데이터를 사용할 수 없습니다",
    snapshotHashLabel: "스냅샷 콘텐츠 해시",
    sma20Label: "20거래일 이동평균",
    sma60Label: "60거래일 이동평균",
    strictPitLabel: "엄격한 PIT",
    tableConditionLabel: "조건",
    topFiveDescription: "서버 순위를 보존한 스냅샷 첫 5개 행입니다.",
    topFiveEyebrow: "우선 보기",
    topFiveHeading: "Top 5",
    trendLabel: "추세",
    trendDownLabel: "하락 추세",
    trendUpLabel: "상승 추세",
    universeHashLabel: "관찰 목록 해시",
    valueLabel: "값",
    vendorSnapshotLabel: "벤더 스냅샷",
    volatility120Label: "120거래일 변동성",
    volatility20Label: "20거래일 변동성",
    volatility60Label: "60거래일 변동성",
    volumeRatioLabel: "20/60 거래량 비율",
    averageVolumeLabel: "20거래일 평균 거래량",
    warningLabel: "정책 경계",
    yes: "예",
    no: "아니오",
    pageDescription:
      "고정된 읽기 전용 가격·거래량 연구 베타를 위한 비공개 오너 워크스페이스입니다.",
    pageTitle: "종목 신호 베타",
    policyAriaLabel: "종목 신호 베타 정책 경계",
  },
};
