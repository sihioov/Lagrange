import type { LocaleDictionary } from "@/lib/i18n/locale";

export type RecommendationsDictionary = {
  readonly allCashAllocation: string;
  readonly asOf: (date: string) => string;
  readonly asOfDateLabel: string;
  readonly asOfDateRequired: string;
  readonly blockedDataTitle: string;
  readonly blockedRunMessage: string;
  readonly blockedRunTitle: string;
  readonly cashAllocation: (percentage: string) => string;
  readonly columnAsOf: string;
  readonly columnCreated: string;
  readonly columnEvidence: string;
  readonly columnFactorScores: string;
  readonly columnInstrument: string;
  readonly columnRank: string;
  readonly columnReason: string;
  readonly columnRunId: string;
  readonly columnSelectionReasons: string;
  readonly columnStatus: string;
  readonly columnTargetWeight: string;
  readonly datasetManifestLabel: string;
  readonly datasetVersionLabel: string;
  readonly entitlementBlockedMessage: string;
  readonly entitlementInactiveMessage: string;
  readonly excludedTableCaption: string;
  readonly exclusionsHeading: string;
  readonly factorSnapshotLabel: string;
  readonly failedRunMessage: string;
  readonly failedRunTitle: string;
  readonly generateRecommendation: string;
  readonly generateStrategyProposal: string;
  readonly generatingStrategyProposal: string;
  readonly historyCaption: string;
  readonly historyEmptyMessage: string;
  readonly historyEyebrow: string;
  readonly historyHeading: string;
  readonly newRunEyebrow: string;
  readonly newRunHelp: string;
  readonly noConfigMessage: string;
  readonly noConfigTitle: string;
  readonly noExclusionReason: string;
  readonly noInstrumentsExcluded: string;
  readonly noInstrumentsSelected: string;
  readonly noRecommendationMessage: string;
  readonly noRecommendationTitle: string;
  readonly notReported: string;
  readonly originLabel: string;
  readonly pendingRunMessage: string;
  readonly pendingRunTitle: string;
  readonly pollErrorFallback: string;
  readonly portfolioSnapshotLabel: string;
  readonly proposalDisclaimer: string;
  readonly provenanceHeading: string;
  readonly reportEyebrow: string;
  readonly reportHeading: string;
  readonly routeDescription: string;
  readonly routeTitle: string;
  readonly selectStrategyConfig: string;
  readonly selectedCandidatesHeading: string;
  readonly selectedTableCaption: string;
  readonly staleResultLabel: string;
  readonly strategyConfigurationLabel: string;
  readonly structuredServerEvidence: string;
  readonly syntheticDataLabel: string;
  readonly syntheticDataMessage: string;
  readonly unavailableMessage: string;
  readonly unavailableTitle: string;
  readonly universeSnapshotLabel: string;
  readonly warningsAriaLabel: string;
  readonly warningsLabel: string;
};

export const recommendationsDictionary: LocaleDictionary<RecommendationsDictionary> = {
  en: {
    allCashAllocation:
      "All-cash allocation: the governed constraints did not select an instrument for this proposal.",
    asOf: (date) => `As of ${date}`,
    asOfDateLabel: "As-of date",
    asOfDateRequired: "As-of date is required.",
    blockedDataTitle: "Recommendation data is blocked",
    blockedRunMessage: "The server blocked this run. Candidate payloads remain hidden.",
    blockedRunTitle: "Recommendation run blocked",
    cashAllocation: (percentage) => `Cash allocation: ${percentage}.`,
    columnAsOf: "As of",
    columnCreated: "Created",
    columnEvidence: "Evidence",
    columnFactorScores: "Factor scores",
    columnInstrument: "Instrument",
    columnRank: "Rank",
    columnReason: "Reason",
    columnRunId: "Run ID",
    columnSelectionReasons: "Selection reasons",
    columnStatus: "Status",
    columnTargetWeight: "Target weight",
    datasetManifestLabel: "Dataset manifest",
    datasetVersionLabel: "Dataset version",
    entitlementBlockedMessage:
      "The recommendation entitlement or dataset is blocked. Creation is disabled and proprietary candidate data is not rendered.",
    entitlementInactiveMessage:
      "The recommendation entitlement is inactive. Creation is disabled and proprietary candidate data is not rendered.",
    excludedTableCaption: "Excluded instruments and policy reasons",
    exclusionsHeading: "Exclusions",
    factorSnapshotLabel: "Factor snapshot",
    failedRunMessage:
      "The worker did not produce a recommendation. Candidate payloads for this run remain hidden.",
    failedRunTitle: "Recommendation failed",
    generateRecommendation: "Generate recommendation",
    generateStrategyProposal: "Generate strategy proposal",
    generatingStrategyProposal: "Generating strategy proposal",
    historyCaption: "Recommendation run history",
    historyEmptyMessage: "No historical recommendation runs are available.",
    historyEyebrow: "Historical runs",
    historyHeading: "Recommendation history",
    newRunEyebrow: "New governed run",
    newRunHelp: "The API validates the stored strategy configuration and as-of dataset.",
    noConfigMessage: "Save an allowed strategy configuration before creating a recommendation run.",
    noConfigTitle: "No strategy configuration is available",
    noExclusionReason: "No exclusion reason was reported.",
    noInstrumentsExcluded: "No instruments were excluded.",
    noInstrumentsSelected: "No instruments were selected.",
    noRecommendationMessage: "Generate a recommendation to inspect its governed proposal.",
    noRecommendationTitle: "No recommendation available",
    notReported: "Not reported",
    originLabel: "Origin",
    pendingRunMessage:
      "The server is producing the recommendation. The last successful proposal remains available.",
    pendingRunTitle: "Recommendation is in progress",
    pollErrorFallback: "Run status could not be refreshed.",
    portfolioSnapshotLabel: "Portfolio snapshot",
    proposalDisclaimer:
      "Strategy-based proposal, not investment advice. Review warnings and the as-of date.",
    provenanceHeading: "Run provenance",
    reportEyebrow: "Latest governed output",
    reportHeading: "Strategy-based proposal",
    routeDescription:
      "Inspect server-produced candidates, target weights, factor evidence, and exclusions.",
    routeTitle: "Recommendations",
    selectStrategyConfig: "Select a strategy configuration.",
    selectedCandidatesHeading: "Selected candidates",
    selectedTableCaption: "Selected instruments and target weights",
    staleResultLabel: "Stale result",
    strategyConfigurationLabel: "Strategy configuration",
    structuredServerEvidence: "Structured server evidence",
    syntheticDataLabel: "Synthetic QA data",
    syntheticDataMessage:
      "This proposal is based on synthetic QA data and is not a live market-data result.",
    unavailableMessage:
      "Recommendation data could not be loaded. Retry after checking the service status.",
    unavailableTitle: "Recommendations unavailable",
    universeSnapshotLabel: "Universe snapshot",
    warningsAriaLabel: "Recommendation warnings",
    warningsLabel: "Warnings",
  },
  ko: {
    allCashAllocation:
      "전액 현금 배분: 거버넌스 제약 조건이 이번 제안에 해당하는 종목을 선정하지 않았습니다.",
    asOf: (date) => `기준일: ${date}`,
    asOfDateLabel: "기준일",
    asOfDateRequired: "기준일은 필수입니다.",
    blockedDataTitle: "추천 데이터가 차단되었습니다",
    blockedRunMessage: "서버가 이 실행을 차단했습니다. 후보 데이터는 계속 숨겨집니다.",
    blockedRunTitle: "추천 실행이 차단되었습니다",
    cashAllocation: (percentage) => `현금 비중: ${percentage}.`,
    columnAsOf: "기준일",
    columnCreated: "생성일",
    columnEvidence: "근거",
    columnFactorScores: "팩터 점수",
    columnInstrument: "종목",
    columnRank: "순위",
    columnReason: "사유",
    columnRunId: "실행 ID",
    columnSelectionReasons: "선정 사유",
    columnStatus: "상태",
    columnTargetWeight: "목표 비중",
    datasetManifestLabel: "데이터셋 매니페스트",
    datasetVersionLabel: "데이터셋 버전",
    entitlementBlockedMessage:
      "추천 이용 권한 또는 데이터셋이 차단되었습니다. 생성이 비활성화되며 독점 후보 데이터는 표시되지 않습니다.",
    entitlementInactiveMessage:
      "추천 이용 권한이 비활성 상태입니다. 생성이 비활성화되며 독점 후보 데이터는 표시되지 않습니다.",
    excludedTableCaption: "제외된 종목 및 정책 사유",
    exclusionsHeading: "제외 내역",
    factorSnapshotLabel: "팩터 스냅샷",
    failedRunMessage: "워커가 추천을 생성하지 못했습니다. 이 실행의 후보 데이터는 계속 숨겨집니다.",
    failedRunTitle: "추천 생성에 실패했습니다",
    generateRecommendation: "추천 생성",
    generateStrategyProposal: "전략 제안 생성",
    generatingStrategyProposal: "전략 제안 생성 중",
    historyCaption: "추천 실행 이력",
    historyEmptyMessage: "이용 가능한 과거 추천 실행 내역이 없습니다.",
    historyEyebrow: "과거 실행",
    historyHeading: "추천 이력",
    newRunEyebrow: "새 거버넌스 실행",
    newRunHelp: "API가 저장된 전략 설정과 기준일 데이터셋을 검증합니다.",
    noConfigMessage: "추천 실행을 생성하기 전에 허용된 전략 설정을 저장하세요.",
    noConfigTitle: "이용 가능한 전략 설정이 없습니다",
    noExclusionReason: "제외 사유가 보고되지 않았습니다.",
    noInstrumentsExcluded: "제외된 종목이 없습니다.",
    noInstrumentsSelected: "선정된 종목이 없습니다.",
    noRecommendationMessage: "거버넌스 제안을 확인하려면 추천을 생성하세요.",
    noRecommendationTitle: "이용 가능한 추천이 없습니다",
    notReported: "보고되지 않음",
    originLabel: "출처",
    pendingRunMessage:
      "서버가 추천을 생성하는 중입니다. 마지막으로 성공한 제안은 계속 확인할 수 있습니다.",
    pendingRunTitle: "추천 생성이 진행 중입니다",
    pollErrorFallback: "실행 상태를 갱신하지 못했습니다.",
    portfolioSnapshotLabel: "포트폴리오 스냅샷",
    proposalDisclaimer:
      "이 제안은 전략에 기반한 제안이며 투자 자문이 아닙니다. 경고 문구와 기준일을 반드시 확인하세요.",
    provenanceHeading: "실행 출처 정보",
    reportEyebrow: "최신 거버넌스 결과",
    reportHeading: "전략 기반 제안",
    routeDescription: "서버가 산출한 후보, 목표 비중, 팩터 근거, 제외 내역을 확인하세요.",
    routeTitle: "추천",
    selectStrategyConfig: "전략 설정을 선택하세요.",
    selectedCandidatesHeading: "선정된 종목",
    selectedTableCaption: "선정된 종목 및 목표 비중",
    staleResultLabel: "결과가 오래되었습니다",
    strategyConfigurationLabel: "전략 설정",
    structuredServerEvidence: "구조화된 서버 근거",
    syntheticDataLabel: "합성 QA 데이터",
    syntheticDataMessage:
      "이 제안은 합성 QA 데이터를 기반으로 하며 실거래 시장 데이터 결과가 아닙니다.",
    unavailableMessage:
      "추천 데이터를 불러오지 못했습니다. 서비스 상태를 확인한 후 다시 시도하세요.",
    unavailableTitle: "추천을 이용할 수 없습니다",
    universeSnapshotLabel: "유니버스 스냅샷",
    warningsAriaLabel: "추천 경고",
    warningsLabel: "경고",
  },
};
