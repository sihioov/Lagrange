import type { LocaleDictionary } from "@/lib/i18n/locale";
import type { OwnerBetaReasonCode } from "@/lib/products/owner-beta-contracts";
import type {
  OwnerBetaDailyReasonCode,
  OwnerBetaDailyStateCode,
} from "@/lib/products/owner-beta-daily-contracts";

export type RecommendationsDictionary = {
  readonly dailyEyebrow: string;
  readonly dailyHeading: string;
  readonly dailyDatesHelp: string;
  readonly dailyUnverified: string;
  readonly dailyLastObservation: string;
  readonly dailyExpectedSession: string;
  readonly dailySupportEnd: string;
  readonly dailyPublishedAt: string;
  readonly dailyDueAt: string;
  readonly dailyObservedAt: string;
  readonly dailyReservedRun: string;
  readonly dailyRefresh: string;
  readonly dailyConfigHeading: string;
  readonly dailySelectConfig: string;
  readonly dailyManualUnavailableTitle: string;
  readonly dailyManualUnavailableMessage: string;
  readonly dailyPreviousResult: string;
  readonly dailyPreviousResultHelp: string;
  readonly dailyProvenanceLabel: string;
  readonly dailyManual: string;
  readonly dailyAutomatic: string;
  readonly dailyContentHash: string;
  readonly dailyManifestHash: string;
  readonly dailyAdmissionHash: string;
  readonly dailyPolicyHash: string;
  readonly dailyBasePins: string;
  readonly dailyHistoryIncomplete: string;
  readonly dailyOlderRuns: string;
  readonly dailyResultAsOf: string;
  readonly dailyStates: Readonly<Record<OwnerBetaDailyStateCode, string>>;
  readonly dailyReasons: Readonly<Record<OwnerBetaDailyReasonCode, string>>;
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
  readonly ownerBetaAudienceLabel: string;
  readonly ownerBetaAudienceValue: string;
  readonly ownerBetaCapabilityLabel: string;
  readonly ownerBetaCapabilityValue: string;
  readonly ownerBetaCanceledMessage: string;
  readonly ownerBetaCanceledTitle: string;
  readonly ownerBetaActionManifestHashLabel: string;
  readonly ownerBetaApprovalRegistryHashLabel: string;
  readonly ownerBetaArtifactManifestHashLabel: string;
  readonly ownerBetaAssetClassMissing: string;
  readonly ownerBetaAuditDetails: string;
  readonly ownerBetaHistoryCaption: string;
  readonly ownerBetaHistoryHeading: string;
  readonly ownerBetaInputWarning: string;
  readonly ownerBetaCandidateContentHashLabel: string;
  readonly ownerBetaFactorEvidence: string;
  readonly ownerBetaFinishedAtLabel: string;
  readonly ownerBetaItemsCaption: string;
  readonly ownerBetaItemsHeading: string;
  readonly ownerBetaInstrumentNameMissing: string;
  readonly ownerBetaJobIdLabel: string;
  readonly ownerBetaLatestSupportedAsOfLabel: string;
  readonly ownerBetaNoRunsMessage: string;
  readonly ownerBetaNoFactorEvidence: string;
  readonly ownerBetaReportEyebrow: string;
  readonly ownerBetaReportHeading: string;
  readonly ownerBetaReasonExplanations: Readonly<Record<OwnerBetaReasonCode, string>>;
  readonly ownerBetaRouteDescription: string;
  readonly ownerBetaRouteTitle: string;
  readonly ownerBetaRunningMessage: string;
  readonly ownerBetaRunningTitle: string;
  readonly ownerBetaRunAsOfLabel: string;
  readonly ownerBetaRunIdLabel: string;
  readonly ownerBetaSealedAsOfLabel: string;
  readonly ownerBetaStartedAtLabel: string;
  readonly ownerBetaStrictPitLabel: string;
  readonly ownerBetaStrictPitValue: string;
  readonly ownerBetaStrategyConfigIdLabel: string;
  readonly ownerBetaStrategyConfigHashLabel: string;
  readonly ownerBetaStrategyIdentityLabel: string;
  readonly ownerBetaTargetSnapshotHashLabel: string;
  readonly ownerBetaSubmit: string;
  readonly ownerBetaSubmitting: string;
  readonly ownerBetaStage5ManifestHashLabel: string;
  readonly ownerBetaTrackingIndexMissing: string;
  readonly ownerBetaTrendFactorLabel: (window: string) => string;
  readonly ownerBetaMomentumFactorLabel: string;
  readonly ownerBetaReturnFactorLabel: string;
  readonly ownerBetaVolatilityFactorLabel: (window: string) => string;
  readonly ownerBetaUnsupportedAsOf: string;
  readonly ownerBetaUpdatedAtLabel: string;
  readonly ownerBetaVendorSnapshotLabel: string;
  readonly ownerBetaVendorSnapshotValue: string;
};

export const recommendationsDictionary: LocaleDictionary<RecommendationsDictionary> = {
  en: {
    dailyEyebrow: "Automatic daily updates",
    dailyHeading: "Daily recommendation status",
    dailyDatesHelp:
      "The expected session, input coverage, published result date and publication time are separate. A manual historical run does not confirm a daily update.",
    dailyUnverified: "Current status not verified",
    dailyLastObservation:
      "Dates below are the last observation. Refresh to verify the current status.",
    dailyExpectedSession: "Expected trading session",
    dailySupportEnd: "Daily input coverage ends",
    dailyPublishedAt: "Published at (KST)",
    dailyDueAt: "Update due (KST)",
    dailyObservedAt: "Observed at (KST)",
    dailyReservedRun: "View the reserved run",
    dailyRefresh: "Refresh daily status",
    dailyConfigHeading: "Configuration for daily updates",
    dailySelectConfig: "View this configuration",
    dailyManualUnavailableTitle: "Manual historical runs unavailable",
    dailyManualUnavailableMessage:
      "Historical input discovery is unavailable. Daily status and existing results remain separate.",
    dailyPreviousResult: "Previous successful result",
    dailyPreviousResultHelp:
      "This result is retained while the selected run is pending or unsuccessful. Its date has not advanced.",
    dailyProvenanceLabel: "Run origin",
    dailyManual: "Manual historical run",
    dailyAutomatic: "Automatic daily run",
    dailyContentHash: "Daily content commitment",
    dailyManifestHash: "Daily manifest commitment",
    dailyAdmissionHash: "Daily admission commitment",
    dailyPolicyHash: "Daily policy commitment",
    dailyBasePins: "Historical base commitments",
    dailyHistoryIncomplete:
      "No successful result was found in this loaded window. Older runs may still contain a result.",
    dailyOlderRuns: "Older runs",
    dailyResultAsOf: "Latest daily result as-of",
    dailyStates: {
      DISABLED: "Automatic updates off",
      UNVERIFIABLE: "Unable to verify",
      INPUT_PENDING: "Waiting for input",
      INPUT_BLOCKED: "Input blocked",
      QUEUED: "Queued",
      RUNNING: "Computing",
      RETRYING: "Waiting to retry",
      FAILED: "Run failed",
      CURRENT: "Expected session published",
      STALE: "Update overdue",
    },
    dailyReasons: {
      RUNTIME_DISABLED: "Daily execution is disabled for this release or service.",
      CONFIG_DISABLED: "Automatic updates are off for this configuration.",
      CONFIG_INACTIVE: "This strategy configuration is inactive.",
      CONTROL_UNAVAILABLE: "Daily execution settings cannot be verified.",
      CALENDAR_COVERAGE_GAP: "Trading calendar evidence is incomplete.",
      VERIFICATION_EXPIRED: "Input verification has expired.",
      INPUT_UNAVAILABLE: "Approved daily input is unavailable.",
      INPUT_NOT_READY: "Daily input is not ready.",
      BEFORE_ACTIVATION: "The expected session precedes daily activation.",
      AWAITING_SCHEDULE: "Input is ready; waiting for reservation.",
      JOB_QUEUED: "The daily run is queued.",
      JOB_RUNNING: "The daily run is computing.",
      JOB_RETRYING: "The daily run is waiting for a bounded retry.",
      JOB_FAILED: "The daily run failed; previous results are retained.",
      CAPACITY_EXCEEDED: "The current execution limit has been reached.",
      SUPERSEDED: "A newer session superseded this reservation.",
      CONFIGURATION_CHANGED: "The configuration changed after reservation.",
      FUTURE_EVIDENCE: "Input contains evidence beyond the permitted date.",
      SOURCE_CONFLICT: "Input sources conflict.",
      SEALED_SOURCE_CHANGED: "Previously sealed source evidence changed.",
      SOURCE_REVOKED: "Source evidence was revoked.",
      SOURCE_INTEGRITY: "Source integrity could not be verified.",
      CALENDAR_CONFLICT: "Calendar evidence conflicts.",
      NONCASH_ACTION: "A non-cash corporate action blocks this price-only input.",
    },
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
    ownerBetaAudienceLabel: "Audience",
    ownerBetaAudienceValue: "Owner-only",
    ownerBetaCapabilityLabel: "Capability",
    ownerBetaCapabilityValue: "Price-return only",
    ownerBetaCanceledMessage: "The Owner-only run was canceled before it produced an output.",
    ownerBetaCanceledTitle: "Owner-only run canceled",
    ownerBetaActionManifestHashLabel: "Action manifest hash",
    ownerBetaApprovalRegistryHashLabel: "Approval registry hash",
    ownerBetaArtifactManifestHashLabel: "Artifact manifest hash",
    ownerBetaAssetClassMissing: "Asset class not provided",
    ownerBetaAuditDetails: "Audit details",
    ownerBetaCandidateContentHashLabel: "Candidate content hash",
    ownerBetaFactorEvidence: "Factor evidence",
    ownerBetaFinishedAtLabel: "Finished",
    ownerBetaHistoryCaption: "Owner-only price-return recommendation runs",
    ownerBetaHistoryHeading: "Owner-only run history",
    ownerBetaInputWarning:
      "This beta uses a vendor snapshot and is not strict point-in-time data. It is for the Owner only and reports price returns, not total returns.",
    ownerBetaInstrumentNameMissing: "Instrument name not provided",
    ownerBetaItemsCaption: "ETF11 items and price-return targets",
    ownerBetaItemsHeading: "ETF11 items",
    ownerBetaJobIdLabel: "Job ID",
    ownerBetaLatestSupportedAsOfLabel: "Latest supported as-of date",
    ownerBetaNoFactorEvidence: "No factor evidence (buy and hold strategy)",
    ownerBetaNoRunsMessage: "No owner-only recommendation runs are available.",
    ownerBetaReportEyebrow: "Owner-only price-return output",
    ownerBetaReportHeading: "Owner-only recommendation",
    ownerBetaReasonExplanations: {
      SELECTED_TOP_N: "Selected under the selection criteria. Rank is shown separately.",
      NOT_SELECTED_BEYOND_TOP_N: "Did not meet the selection criteria.",
      EXCLUDED_MANDATORY_FACTOR_NULL: "Excluded because a required factor value was unavailable.",
      ALL_CASH_NO_ELIGIBLE: "No eligible instruments; the portfolio remains fully in cash.",
      WEIGHT_CAPPED_AT_MAX: "Applied the maximum weight cap.",
      WEIGHT_ROUNDING_RESIDUE_TO_CASH: "Allocated rounding residue to cash.",
      CASH_FLOOR_APPLIED: "Maintained the minimum cash weight.",
      BENCHMARK_HELD: "Held as the benchmark. Weight is shown separately as target weight.",
      TREND_POSITIVE: "Met the positive trend condition.",
      TREND_NEGATIVE_CASH: "Did not meet the trend condition; cash is maintained.",
      ABSOLUTE_MOMENTUM_PASSED: "Met the absolute momentum condition.",
      DEFENSIVE_CASH_SELECTED:
        "Did not meet the absolute momentum condition; cash was selected defensively.",
      INVERSE_VOL_WEIGHTED: "Allocated weight using inverse volatility.",
      NOT_SELECTED_BY_STRATEGY: "This fixed-universe instrument was not selected by the strategy.",
    },
    ownerBetaRouteDescription:
      "Review the Owner-only price-return recommendation produced from the sealed vendor snapshot.",
    ownerBetaRouteTitle: "Owner-only recommendations",
    ownerBetaRunningMessage: "The Owner-only worker is producing this price-return output.",
    ownerBetaRunningTitle: "Owner-only run is in progress",
    ownerBetaReturnFactorLabel: "12-month return",
    ownerBetaRunAsOfLabel: "Run as-of date",
    ownerBetaRunIdLabel: "Run ID",
    ownerBetaSealedAsOfLabel: "Sealed input as-of date",
    ownerBetaStartedAtLabel: "Started",
    ownerBetaStrictPitLabel: "Point-in-time mode",
    ownerBetaStrategyConfigIdLabel: "Strategy config ID",
    ownerBetaStrictPitValue: "Non-strict PIT",
    ownerBetaStrategyConfigHashLabel: "Strategy config hash",
    ownerBetaStrategyIdentityLabel: "Strategy identity",
    ownerBetaTargetSnapshotHashLabel: "Target snapshot hash",
    ownerBetaSubmit: "Generate owner-only recommendation",
    ownerBetaSubmitting: "Starting owner-only run",
    ownerBetaStage5ManifestHashLabel: "Stage5 manifest hash",
    ownerBetaTrackingIndexMissing: "Tracking index: not provided",
    ownerBetaTrendFactorLabel: (window) => `${window}-day price deviation from moving average`,
    ownerBetaMomentumFactorLabel: "12-month momentum excluding the most recent month",
    ownerBetaVolatilityFactorLabel: (window) =>
      `${window}-trading-day annualized realized volatility`,
    ownerBetaUnsupportedAsOf: "Select a supported sealed input date.",
    ownerBetaUpdatedAtLabel: "Updated",
    ownerBetaVendorSnapshotLabel: "Input source",
    ownerBetaVendorSnapshotValue: "Vendor snapshot",
  },
  ko: {
    dailyEyebrow: "자동 일일 갱신",
    dailyHeading: "일일 추천 상태",
    dailyDatesHelp:
      "기대 거래일, 입력 지원 종료일, 발행된 추천 기준일과 발행 시각은 서로 다릅니다. 수동 과거 실행은 일일 갱신의 성공을 뜻하지 않습니다.",
    dailyUnverified: "현재 상태 확인 안 됨",
    dailyLastObservation:
      "아래 날짜는 마지막 관측값입니다. 새로 조회해야 현재 상태를 확인할 수 있습니다.",
    dailyExpectedSession: "기대 거래일",
    dailySupportEnd: "일일 데이터 지원 종료일",
    dailyPublishedAt: "발행 시각 (KST)",
    dailyDueAt: "갱신 기한 (KST)",
    dailyObservedAt: "관측 시각 (KST)",
    dailyReservedRun: "예약된 실행 보기",
    dailyRefresh: "일일 상태 새로 조회",
    dailyConfigHeading: "자동 갱신 대상 설정",
    dailySelectConfig: "이 설정 보기",
    dailyManualUnavailableTitle: "수동 과거 실행 이용 불가",
    dailyManualUnavailableMessage:
      "과거 입력의 지원일을 확인할 수 없습니다. 일일 상태와 기존 결과는 별도로 표시합니다.",
    dailyPreviousResult: "이전 성공 결과",
    dailyPreviousResultHelp:
      "선택한 실행이 대기 중이거나 성공하지 않아 이전 결과를 유지합니다. 결과 기준일이 갱신된 것은 아닙니다.",
    dailyProvenanceLabel: "실행 출처",
    dailyManual: "수동 과거 실행",
    dailyAutomatic: "자동 일일 실행",
    dailyContentHash: "일일 내용 해시",
    dailyManifestHash: "일일 매니페스트 해시",
    dailyAdmissionHash: "일일 입력 승인 해시",
    dailyPolicyHash: "일일 정책 해시",
    dailyBasePins: "기준 과거 입력 해시",
    dailyHistoryIncomplete:
      "불러온 범위에서 성공 결과를 찾지 못했습니다. 더 오래된 실행에 결과가 있을 수 있습니다.",
    dailyOlderRuns: "이전 실행 더 보기",
    dailyResultAsOf: "최근 일일 추천 기준일",
    dailyStates: {
      DISABLED: "자동 갱신 꺼짐",
      UNVERIFIABLE: "상태 확인 불가",
      INPUT_PENDING: "입력 대기",
      INPUT_BLOCKED: "입력 차단",
      QUEUED: "실행 대기",
      RUNNING: "계산 중",
      RETRYING: "재시도 대기",
      FAILED: "실행 실패",
      CURRENT: "기대 거래일 추천 발행됨",
      STALE: "갱신 지연",
    },
    dailyReasons: {
      RUNTIME_DISABLED: "릴리스 또는 서비스에서 일일 실행이 꺼져 있습니다.",
      CONFIG_DISABLED: "이 설정의 자동 갱신이 꺼져 있습니다.",
      CONFIG_INACTIVE: "전략 설정이 비활성 상태입니다.",
      CONTROL_UNAVAILABLE: "일일 실행 설정을 확인할 수 없습니다.",
      CALENDAR_COVERAGE_GAP: "거래일 달력 근거가 불완전합니다.",
      VERIFICATION_EXPIRED: "입력 검증의 유효 시간이 지났습니다.",
      INPUT_UNAVAILABLE: "승인된 일일 입력을 이용할 수 없습니다.",
      INPUT_NOT_READY: "일일 입력이 아직 준비되지 않았습니다.",
      BEFORE_ACTIVATION: "기대 거래일이 일일 실행 시작일보다 이전입니다.",
      AWAITING_SCHEDULE: "입력이 준비되어 실행 예약을 기다립니다.",
      JOB_QUEUED: "일일 실행이 대기열에 있습니다.",
      JOB_RUNNING: "일일 실행을 계산 중입니다.",
      JOB_RETRYING: "일일 실행이 정해진 범위의 재시도를 기다립니다.",
      JOB_FAILED: "일일 실행이 실패했습니다. 이전 결과를 유지합니다.",
      CAPACITY_EXCEEDED: "현재 실행 한도에 도달했습니다.",
      SUPERSEDED: "새 거래일이 이 예약을 대체했습니다.",
      CONFIGURATION_CHANGED: "예약 이후 설정이 변경되었습니다.",
      FUTURE_EVIDENCE: "허용 날짜 이후의 입력 근거가 포함되어 있습니다.",
      SOURCE_CONFLICT: "입력 출처가 서로 충돌합니다.",
      SEALED_SOURCE_CHANGED: "봉인된 출처 근거가 변경되었습니다.",
      SOURCE_REVOKED: "출처 근거가 철회되었습니다.",
      SOURCE_INTEGRITY: "출처 무결성을 확인할 수 없습니다.",
      CALENDAR_CONFLICT: "거래일 달력 근거가 충돌합니다.",
      NONCASH_ACTION: "비현금 기업행사로 가격 수익률 전용 입력이 차단되었습니다.",
    },
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
    ownerBetaAudienceLabel: "대상 범위",
    ownerBetaAudienceValue: "소유자 전용",
    ownerBetaCapabilityLabel: "데이터 능력",
    ownerBetaCapabilityValue: "가격 수익률 전용",
    ownerBetaCanceledMessage: "소유자 전용 실행이 결과를 만들기 전에 취소되었습니다.",
    ownerBetaCanceledTitle: "소유자 전용 실행 취소",
    ownerBetaActionManifestHashLabel: "액션 매니페스트 해시",
    ownerBetaApprovalRegistryHashLabel: "승인 레지스트리 해시",
    ownerBetaArtifactManifestHashLabel: "아티팩트 매니페스트 해시",
    ownerBetaAssetClassMissing: "자산군 미제공",
    ownerBetaAuditDetails: "감사 세부정보",
    ownerBetaCandidateContentHashLabel: "후보 콘텐츠 해시",
    ownerBetaFactorEvidence: "팩터 근거",
    ownerBetaFinishedAtLabel: "종료 시각",
    ownerBetaHistoryCaption: "소유자 전용 가격 수익률 추천 실행",
    ownerBetaHistoryHeading: "소유자 전용 실행 이력",
    ownerBetaInputWarning:
      "이 베타는 공급자 스냅샷을 사용하며 엄격한 시점 일치(PIT) 데이터가 아닙니다. 소유자 전용이며 가격 수익률만 보고하고 총수익률은 제공하지 않습니다.",
    ownerBetaInstrumentNameMissing: "종목명 미제공",
    ownerBetaItemsCaption: "ETF11 종목 및 가격 수익률 목표 비중",
    ownerBetaItemsHeading: "ETF11 종목",
    ownerBetaJobIdLabel: "작업 ID",
    ownerBetaLatestSupportedAsOfLabel: "지원되는 최신 기준일",
    ownerBetaNoFactorEvidence: "팩터 근거 없음(매수·보유 전략)",
    ownerBetaNoRunsMessage: "이용 가능한 소유자 전용 추천 실행이 없습니다.",
    ownerBetaReportEyebrow: "소유자 전용 가격 수익률 결과",
    ownerBetaReportHeading: "소유자 전용 추천",
    ownerBetaReasonExplanations: {
      SELECTED_TOP_N: "선정 기준에 따라 선택되었습니다. 순위는 별도 순위 필드에 표시합니다.",
      NOT_SELECTED_BEYOND_TOP_N: "선정 기준에 들지 않았습니다.",
      EXCLUDED_MANDATORY_FACTOR_NULL: "필수 팩터 값이 없어 제외되었습니다.",
      ALL_CASH_NO_ELIGIBLE: "선정 가능한 종목이 없어 전액 현금을 유지합니다.",
      WEIGHT_CAPPED_AT_MAX: "최대 비중 상한을 적용했습니다.",
      WEIGHT_ROUNDING_RESIDUE_TO_CASH: "반올림 잔여를 현금으로 배분했습니다.",
      CASH_FLOOR_APPLIED: "현금 최소 비중을 유지했습니다.",
      BENCHMARK_HELD: "이 종목을 벤치마크로 보유합니다. 비중은 별도 목표 비중 필드에 표시합니다.",
      TREND_POSITIVE: "상승 추세 조건을 충족했습니다.",
      TREND_NEGATIVE_CASH: "추세 조건을 충족하지 않아 현금을 유지합니다.",
      ABSOLUTE_MOMENTUM_PASSED: "절대 모멘텀 조건을 충족했습니다.",
      DEFENSIVE_CASH_SELECTED: "절대 모멘텀 조건을 충족하지 않아 방어적으로 현금을 선택했습니다.",
      INVERSE_VOL_WEIGHTED: "변동성을 고려한 역변동성 방식으로 비중을 배정했습니다.",
      NOT_SELECTED_BY_STRATEGY: "이 고정 유니버스 종목은 해당 전략에서 선택하지 않았습니다.",
    },
    ownerBetaRouteDescription:
      "봉인된 공급자 스냅샷으로 산출한 소유자 전용 가격 수익률 추천을 확인하세요.",
    ownerBetaRouteTitle: "소유자 전용 추천",
    ownerBetaRunningMessage: "소유자 전용 워커가 가격 수익률 결과를 생성하고 있습니다.",
    ownerBetaRunningTitle: "소유자 전용 실행 진행 중",
    ownerBetaReturnFactorLabel: "12개월 수익률",
    ownerBetaRunAsOfLabel: "실행 기준일",
    ownerBetaRunIdLabel: "실행 ID",
    ownerBetaSealedAsOfLabel: "봉인된 입력 기준일",
    ownerBetaStartedAtLabel: "시작 시각",
    ownerBetaStrictPitLabel: "시점 일치 모드",
    ownerBetaStrategyConfigIdLabel: "전략 설정 ID",
    ownerBetaStrictPitValue: "비엄격 PIT",
    ownerBetaStrategyConfigHashLabel: "전략 설정 해시",
    ownerBetaStrategyIdentityLabel: "전략 식별자",
    ownerBetaTargetSnapshotHashLabel: "목표 스냅샷 해시",
    ownerBetaSubmit: "소유자 전용 추천 생성",
    ownerBetaSubmitting: "소유자 전용 실행 시작 중",
    ownerBetaStage5ManifestHashLabel: "Stage5 매니페스트 해시",
    ownerBetaTrackingIndexMissing: "추적지수: 미제공",
    ownerBetaTrendFactorLabel: (window) => `${window}일 이동평균 대비 가격 괴리`,
    ownerBetaMomentumFactorLabel: "12개월-최근 1개월 제외 모멘텀",
    ownerBetaVolatilityFactorLabel: (window) => `${window}거래일 연율화 실현변동성`,
    ownerBetaUnsupportedAsOf: "지원되는 봉인된 입력 기준일을 선택하세요.",
    ownerBetaUpdatedAtLabel: "갱신 시각",
    ownerBetaVendorSnapshotLabel: "입력 출처",
    ownerBetaVendorSnapshotValue: "공급자 스냅샷",
  },
};
