import type { LocaleDictionary } from "@/lib/i18n/locale";

export type PaperDictionary = {
  readonly ownerBeta: OwnerBetaPaperDictionary;
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
  readonly columnAction: string;
  readonly columnAveragePrice: string;
  readonly columnBacktest: string;
  readonly columnBacktestWeight: string;
  readonly columnBound: string;
  readonly columnCash: string;
  readonly columnCommission: string;
  readonly columnComputedOn: string;
  readonly columnCurrentQuantity: string;
  readonly columnCurrentValue: string;
  readonly columnCurrentWeight: string;
  readonly columnDailyReturn: string;
  readonly columnDate: string;
  readonly columnDelivery: string;
  readonly columnDeltaValue: string;
  readonly columnEquity: string;
  readonly columnEstimatedPrice: string;
  readonly columnExecuted: string;
  readonly columnExecutesAt: string;
  readonly columnField: string;
  readonly columnInstrument: string;
  readonly columnKind: string;
  readonly columnNotice: string;
  readonly columnNotional: string;
  readonly columnOrder: string;
  readonly columnPaper: string;
  readonly columnPaperWeight: string;
  readonly columnPositions: string;
  readonly columnPrice: string;
  readonly columnQuantity: string;
  readonly columnRaised: string;
  readonly columnRawPrice: string;
  readonly columnSide: string;
  readonly columnSkipReason: string;
  readonly columnSlippage: string;
  readonly columnState: string;
  readonly columnStrategy: string;
  readonly columnTargetValue: string;
  readonly columnTargetWeight: string;
  readonly columnTax: string;
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
  readonly rebalanceApplyButton: string;
  readonly rebalanceApplyFailedTitle: string;
  readonly rebalanceAppliedMessage: (effectiveDate: string) => string;
  readonly rebalanceApplyingButton: string;
  readonly rebalanceAvailableCashLabel: string;
  readonly rebalanceBuyNotionalLabel: string;
  readonly rebalanceCashBeforeLabel: string;
  readonly rebalanceCreateAriaLabel: string;
  readonly rebalanceCreateButton: string;
  readonly rebalanceCreatingButton: string;
  readonly rebalanceDecisionsCaption: string;
  readonly rebalanceEquityLabel: string;
  readonly rebalanceErrorCodeLabel: string;
  readonly rebalanceEyebrow: string;
  readonly rebalanceExplicitFeesLabel: string;
  readonly rebalanceFailedTitle: string;
  readonly rebalanceHeading: string;
  readonly rebalanceIndicativeWarningMessage: string;
  readonly rebalanceIndicativeWarningTitle: string;
  readonly rebalanceLeftoverCashLabel: string;
  readonly rebalanceLineageAccountStateSha: string;
  readonly rebalanceLineageAccountStateVersion: string;
  readonly rebalanceLineageCaption: string;
  readonly rebalanceLineageCuratedVersion: string;
  readonly rebalanceLineageDatasetManifest: string;
  readonly rebalanceLineageDatasetVersion: string;
  readonly rebalanceLineageRecommendationRun: string;
  readonly rebalanceLineageStrategyConfig: string;
  readonly rebalanceLineageTargetPortfolio: string;
  readonly rebalanceLineageTargetPortfolioSha: string;
  readonly rebalanceNoRunsMessage: string;
  readonly rebalanceNoSkipReason: string;
  readonly rebalanceOrdersCaption: string;
  readonly rebalancePollingMessage: string;
  readonly rebalanceRunLabel: string;
  readonly rebalanceSellNotionalLabel: string;
  readonly rebalanceStatusLabel: (status: string) => string;
  readonly rebalanceTimedOutMessage: string;
  readonly rebalanceTimedOutTitle: string;
  readonly rebalanceTotalsCaption: string;
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

export type OwnerBetaPaperDictionary = {
  readonly eyebrow: string;
  readonly title: string;
  readonly stateLabel: string;
  readonly stateName: (state: string) => string;
  readonly stateDescription: (state: string) => string;
  readonly disabledTitle: string;
  readonly disabledMessage: string;
  readonly unavailableMessage: string;
  readonly errorMessage: (code: string) => string;
  readonly retry: string;
  readonly consentTitle: string;
  readonly consentMessage: string;
  readonly consentCheckbox: string;
  readonly consentInButton: string;
  readonly consentOutButton: string;
  readonly consentSaved: string;
  readonly activationNote: string;
  readonly autoBindingNote: string;
  readonly sessionsTitle: string;
  readonly noSessions: string;
  readonly runLabel: string;
  readonly noRuns: string;
  readonly previewButton: string;
  readonly previewCreating: string;
  readonly previewWaiting: string;
  readonly previewFailed: string;
  readonly previewApplied: string;
  readonly previewApplyButton: string;
  readonly previewApplying: string;
  readonly unknownCommit: string;
  readonly previewTitle: string;
  readonly previewStatus: string;
  readonly previewSignal: string;
  readonly previewEffective: string;
  readonly costProfile: string;
  readonly costProfileVersion: string;
  readonly signalDateHelp: string;
  readonly effectiveDateHelp: string;
  readonly previewAmount: string;
  readonly previewState: (state: string) => string;
  readonly planStatus: (status: string) => string;
  readonly cashBefore: string;
  readonly availableCash: string;
  readonly leftoverCash: string;
  readonly buyNotional: string;
  readonly sellNotional: string;
  readonly explicitFees: string;
  readonly informationalSlippage: string;
  readonly previewTimedOut: string;
  readonly previewOrders: string;
  readonly columnSide: string;
  readonly columnQuantity: string;
  readonly columnPrice: string;
  readonly previewOrderPrice: string;
  readonly priceObservation: string;
  readonly actualPrice: string;
  readonly actualDifferenceTitle: string;
  readonly actualDifferenceCount: (proposed: number, matched: number) => string;
  readonly decimalExactNote: string;
  readonly proposedQuantity: string;
  readonly actualQuantity: string;
  readonly actualOrderCount: (count: number) => string;
  readonly actualOrderDetailsCount: (matched: number, references: number) => string;
  readonly noActualOrders: string;
  readonly sessionDetails: string;
  readonly sessionState: (state: string) => string;
  readonly processingOrigin: (origin: string) => string;
  readonly sessionSignal: string;
  readonly sessionEffective: string;
  readonly sourceRetrievedAt: string;
  readonly admittedAt: string;
  readonly usableAt: string;
  readonly settledAt: string;
  readonly valuedAt: string;
  readonly valuationPending: string;
  readonly settlementKind: string;
  readonly settlementKindLabel: (kind: string | null) => string;
  readonly reasonCode: string;
  readonly actualAmounts: string;
  readonly cashAfter: string;
  readonly valuationCash: string;
  readonly positionsValue: string;
  readonly equity: string;
  readonly orderReferences: string;
  readonly comparison: string;
  readonly comparisonDisposition: (disposition: string) => string;
  readonly comparisonReason: string;
  readonly comparisonSummary: string;
  readonly notComparable: string;
  readonly noBacktestPerformance: string;
  readonly noValue: string;
  readonly refresh: string;
};

function paperLabels(labels: Readonly<Record<string, string>>): Readonly<Record<string, string>> {
  return labels;
}

export const paperDictionary: LocaleDictionary<PaperDictionary> = {
  en: {
    ownerBeta: {
      eyebrow: "Owner-only Paper beta",
      title: "Owner-beta Paper control and session review",
      stateLabel: "Control state",
      stateName: (state) =>
        paperLabels({
          DISABLED: "Disabled",
          UNBOUND: "Not bound",
          OFF: "Off",
          ENABLED: "Enabled",
          DRAINING: "Stopping",
          RESTORED_OFF: "Restored off",
          BLOCKED: "Blocked",
        })[state] ?? "Unknown state",
      stateDescription: (state) =>
        paperLabels({
          DISABLED:
            "Owner-beta Paper is disabled. This status was read without opening the protected Paper runtime.",
          UNBOUND: "No eligible Owner Paper account and strategy are bound to this control.",
          OFF: "The control is off. Consent does not turn it on; activation is a separate protected operation.",
          ENABLED:
            "The protected control reports enabled. New work still checks current rights, source evidence, and runtime state.",
          DRAINING:
            "The control is draining existing work. Wait for the status to settle before taking another action.",
          RESTORED_OFF:
            "The control was restored in the off state. It will not resume automatically.",
          BLOCKED:
            "A runtime or source check blocked this control. Review the session reason and current evidence.",
        })[state] ?? "The current control state is not recognized.",
      disabledTitle: "Owner-beta Paper is disabled",
      disabledMessage: "Protected runtime and Paper ledger requests were not made.",
      unavailableMessage: "Owner-beta Paper status could not be loaded. Refresh to try again.",
      errorMessage: (code) =>
        paperLabels({
          PAPER_UNBOUND: "No Owner account is currently bound to this Paper control.",
          PAPER_STALE:
            "The account or source evidence changed. Refresh state and create a new preview.",
          PAPER_CONFLICT: "This request conflicts with the current pending target.",
          PAPER_NOT_READY: "Current source evidence or capacity is not sufficient to continue.",
          PAPER_RIGHTS_BLOCKED: "Current Paper or recommendation rights do not allow this action.",
          PAPER_SOURCE_BLOCKED:
            "Input validation rejected this source evidence. Review the session reason before retrying.",
          PAPER_DISABLED: "Owner-beta Paper is disabled.",
          PAPER_COMMIT_UNKNOWN:
            "The commit result is uncertain. Read back state before taking any further action.",
          STATE_UNAVAILABLE: "Owner-beta Paper state could not be loaded. Refresh to try again.",
        })[code] ??
        "The request could not be completed. Refresh state and review the current evidence.",
      retry: "Refresh status",
      consentTitle: "Owner consent",
      consentMessage:
        "Consent records an explicit choice for the bound Owner account. It does not activate the Paper runtime.",
      consentCheckbox:
        "I understand that consent is not activation and that Paper uses delayed EOD simulation.",
      consentInButton: "Record consent",
      consentOutButton: "Withdraw consent",
      consentSaved: "Consent was recorded. Control state remains separately managed.",
      activationNote: "There is no public activation control on this page.",
      autoBindingNote:
        "Consent binds the adopted AUTO behavior. No manual/automatic mode switch is exposed.",
      sessionsTitle: "Owner-beta sessions",
      noSessions: "No beta sessions are listed yet.",
      runLabel: "Completed recommendation run",
      noRuns: "No completed run for the bound strategy is available.",
      previewButton: "Create Paper preview",
      previewCreating: "Creating Paper preview…",
      previewWaiting: "Preview is being prepared. This page checks its persisted status.",
      previewFailed:
        "Preview was rejected or failed. Check the reason code and current evidence before making another preview.",
      previewApplied: "Preview is applied to the pending target shown below.",
      previewApplyButton: "Apply this preview",
      previewApplying: "Applying preview…",
      unknownCommit:
        "The apply result is uncertain. A status readback was attempted; do not repeat the economic action. Refresh state to confirm the pending target.",
      previewTitle: "Preview details",
      previewStatus: "Preview status",
      previewSignal: "Signal date T",
      previewEffective: "Economic session E",
      costProfile: "Cost model",
      costProfileVersion: "Cost model version",
      signalDateHelp: "T is the date of the recommendation signal.",
      effectiveDateHelp:
        "E is the later economic session. Preview order prices use T's close as an estimate; the delayed simulation settles from E's raw open after EOD data is admitted. This is not a fill at 09:00. The E price may change quantities and costs.",
      previewAmount: "Preview amounts",
      previewState: (state) =>
        paperLabels({
          PENDING: "Queued",
          RUNNING: "Computing",
          READY: "Ready",
          FAILED: "Failed",
          APPLIED: "Applied",
        })[state] ?? "Unknown preview state",
      planStatus: (status) =>
        paperLabels({ REBALANCE: "Rebalance proposed", NO_TRADE: "No trade proposed" })[status] ??
        "Unknown plan",
      cashBefore: "Cash before preview",
      availableCash: "Available cash",
      leftoverCash: "Cash left after preview",
      buyNotional: "Proposed buy amount",
      sellNotional: "Proposed sell amount",
      explicitFees: "Explicit modeled fees",
      informationalSlippage: "Informational slippage estimate",
      previewTimedOut:
        "The preview is still pending after the bounded status checks. Refresh status to continue.",
      previewOrders: "Proposed orders",
      columnSide: "Side",
      columnQuantity: "Proposed quantity",
      columnPrice: "Input price",
      previewOrderPrice: "T close → indicative price",
      priceObservation: "Signal observation T",
      actualPrice: "Paper order price",
      actualDifferenceTitle: "Preview and settled result",
      actualDifferenceCount: (proposed, matched) =>
        `${proposed} proposed line(s); ${matched} matching actual order record(s) appear in the returned account page.`,
      decimalExactNote: "Decimal values are shown exactly as returned and are not rounded here.",
      proposedQuantity: "Proposed quantity",
      actualQuantity: "Settled order quantity",
      actualOrderCount: (count) =>
        `${count} actual order reference(s) are recorded for this session.`,
      actualOrderDetailsCount: (matched, references) =>
        `${matched} of ${references} order references matched records on this account page; missing rows may be outside the page or unavailable.`,
      noActualOrders: "No actual order references were recorded for this session.",
      sessionDetails: "Session detail",
      sessionState: (state) =>
        paperLabels({
          PENDING: "Pending",
          WAITING_PRICE: "Waiting for price data",
          WAITING_ADMISSION: "Waiting for data admission",
          HELD: "Held",
          BLOCKED: "Blocked",
          RECOVERY_REQUIRED: "Recovery required",
          SETTLED_TRADE: "Trade settled",
          SETTLED_NO_TRADE: "No-trade settled",
          VALUED: "Valued",
        })[state] ?? "Unknown session state",
      processingOrigin: (origin) =>
        paperLabels({
          AUTO: "Automatic processing",
          MANUAL: "Manual processing",
          RECOVERY: "Recovery processing",
        })[origin] ?? "Unknown processing origin",
      sessionSignal: "Signal date T",
      sessionEffective: "Economic session E",
      sourceRetrievedAt: "Original source retrieved at",
      admittedAt: "Input admitted at",
      usableAt: "Price input became usable at",
      settledAt: "Settlement processed at",
      valuedAt: "Valuation processed at",
      valuationPending: "Settlement is complete, but end-of-day valuation is still pending.",
      settlementKind: "Settlement result",
      settlementKindLabel: (kind) =>
        paperLabels({ TRADE: "Trade", NO_TRADE: "No trade" })[kind ?? ""] ?? "Not reported",
      reasonCode: "Reason code",
      actualAmounts: "Settled / valued amounts",
      cashAfter: "Cash after settlement",
      valuationCash: "Cash at valuation",
      positionsValue: "Position value",
      equity: "Equity",
      orderReferences: "Actual order and fill references",
      comparison: "Backtest comparison",
      comparisonDisposition: (disposition) =>
        paperLabels({ NOT_COMPARABLE: "Not comparable", PRICE_RETURN_ONLY: "Price return only" })[
          disposition
        ] ?? "Unknown comparison",
      comparisonReason: "Comparison reason",
      comparisonSummary:
        "Owner-beta Paper and backtest are NOT_COMPARABLE: NO_MATCHED_BACKTEST_EXECUTION_CONTRACT.",
      notComparable: "Not comparable",
      noBacktestPerformance: "Backtest performance is not Paper performance.",
      noValue: "Not available",
      refresh: "Refresh state",
    },
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
    columnAction: "Action",
    columnAveragePrice: "Average price",
    columnBacktest: "Backtest",
    columnBacktestWeight: "Backtest weight",
    columnBound: "Bound",
    columnCash: "Cash",
    columnCommission: "Commission",
    columnComputedOn: "Computed on",
    columnCurrentQuantity: "Current quantity",
    columnCurrentValue: "Current value",
    columnCurrentWeight: "Current weight",
    columnDailyReturn: "Daily return",
    columnDate: "Date",
    columnDelivery: "Delivery",
    columnDeltaValue: "Delta value",
    columnEquity: "Equity",
    columnEstimatedPrice: "Estimated execution price",
    columnExecuted: "Executed",
    columnExecutesAt: "Executes at",
    columnField: "Field",
    columnInstrument: "Instrument",
    columnKind: "Kind",
    columnNotice: "Notice",
    columnNotional: "Notional",
    columnOrder: "Order",
    columnPaper: "Paper",
    columnPaperWeight: "Paper weight",
    columnPositions: "Positions",
    columnPrice: "Price",
    columnQuantity: "Quantity",
    columnRaised: "Raised",
    columnRawPrice: "Raw price",
    columnSide: "Side",
    columnSkipReason: "Skip reason",
    columnSlippage: "Informational slippage",
    columnState: "State",
    columnStrategy: "Strategy",
    columnTargetValue: "Target value",
    columnTargetWeight: "Target weight",
    columnTax: "Tax",
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
    rebalanceApplyButton: "Apply preview",
    rebalanceApplyFailedTitle: "Apply failed",
    rebalanceAppliedMessage: (effectiveDate) =>
      `Applied. A pending target now targets the ${effectiveDate} session.`,
    rebalanceApplyingButton: "Applying preview",
    rebalanceAvailableCashLabel: "Available cash",
    rebalanceBuyNotionalLabel: "Buy notional",
    rebalanceCashBeforeLabel: "Cash before",
    rebalanceCreateAriaLabel: "Create rebalance preview",
    rebalanceCreateButton: "Create preview",
    rebalanceCreatingButton: "Creating preview",
    rebalanceDecisionsCaption: "Rebalance decisions",
    rebalanceEquityLabel: "Equity",
    rebalanceErrorCodeLabel: "Error code",
    rebalanceEyebrow: "Rebalancing preview",
    rebalanceExplicitFeesLabel: "Explicit fees",
    rebalanceFailedTitle: "Preview failed",
    rebalanceHeading: "Preview a rebalance",
    rebalanceIndicativeWarningMessage:
      "This preview prices decisions at the recommendation close, but the account will actually execute at the next session's open. The plan may need to be recomputed before it executes.",
    rebalanceIndicativeWarningTitle: "Indicative only — next-open replan required",
    rebalanceLeftoverCashLabel: "Leftover cash",
    rebalanceLineageAccountStateSha: "Account state SHA-256",
    rebalanceLineageAccountStateVersion: "Account state version",
    rebalanceLineageCaption: "Preview lineage",
    rebalanceLineageCuratedVersion: "Curated version",
    rebalanceLineageDatasetManifest: "Dataset manifest SHA-256",
    rebalanceLineageDatasetVersion: "Dataset version",
    rebalanceLineageRecommendationRun: "Recommendation run",
    rebalanceLineageStrategyConfig: "Strategy configuration",
    rebalanceLineageTargetPortfolio: "Target portfolio",
    rebalanceLineageTargetPortfolioSha: "Target portfolio SHA-256",
    rebalanceNoRunsMessage: "No completed recommendation run is available to preview yet.",
    rebalanceNoSkipReason: "—",
    rebalanceOrdersCaption: "Proposed orders",
    rebalancePollingMessage: "Computing preview — checking again shortly.",
    rebalanceRunLabel: "Recommendation run",
    rebalanceSellNotionalLabel: "Sell notional",
    rebalanceStatusLabel: (status) => `Preview status: ${status}`,
    rebalanceTimedOutMessage:
      "The preview did not finish computing within the polling window. Try creating it again.",
    rebalanceTimedOutTitle: "Preview timed out",
    rebalanceTotalsCaption: "Preview totals",
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
    ownerBeta: {
      eyebrow: "Owner 전용 Paper 베타",
      title: "Owner-beta Paper 제어와 세션 검토",
      stateLabel: "제어 상태",
      stateName: (state) =>
        paperLabels({
          DISABLED: "비활성",
          UNBOUND: "미바인딩",
          OFF: "꺼짐",
          ENABLED: "활성",
          DRAINING: "종료 처리 중",
          RESTORED_OFF: "복원 후 꺼짐",
          BLOCKED: "차단됨",
        })[state] ?? "알 수 없는 상태",
      stateDescription: (state) =>
        paperLabels({
          DISABLED:
            "Owner-beta Paper가 비활성입니다. 보호된 Paper 런타임을 열지 않고 상태를 확인했습니다.",
          UNBOUND: "이 제어에 연결된 Owner Paper 계좌와 전략이 없습니다.",
          OFF: "제어가 꺼져 있습니다. 동의는 제어를 켜지 않으며 활성화는 별도의 보호된 작업입니다.",
          ENABLED:
            "보호된 제어가 활성 상태입니다. 새 작업은 현재 권리, 소스 증거, 런타임 상태를 다시 확인합니다.",
          DRAINING:
            "기존 작업을 정리하고 있습니다. 다른 작업 전에 상태가 안정될 때까지 기다리세요.",
          RESTORED_OFF: "꺼진 상태로 복원되었습니다. 자동으로 다시 시작하지 않습니다.",
          BLOCKED:
            "런타임 또는 소스 검증이 이 제어를 차단했습니다. 세션 사유와 최신 증거를 확인하세요.",
        })[state] ?? "현재 제어 상태를 확인할 수 없습니다.",
      disabledTitle: "Owner-beta Paper 비활성",
      disabledMessage: "보호 런타임과 Paper 원장 요청을 보내지 않았습니다.",
      unavailableMessage:
        "Owner-beta Paper 상태를 불러오지 못했습니다. 새로고침해 다시 시도하세요.",
      errorMessage: (code) =>
        paperLabels({
          PAPER_UNBOUND: "현재 이 Paper 제어에 연결된 Owner 계좌가 없습니다.",
          PAPER_STALE:
            "계좌 또는 소스 증거가 변경됐습니다. 상태를 새로고침하고 새 미리보기를 만드세요.",
          PAPER_CONFLICT: "현재 대기 대상과 요청이 충돌합니다.",
          PAPER_NOT_READY: "현재 소스 증거나 처리 용량이 부족합니다.",
          PAPER_RIGHTS_BLOCKED: "현재 Paper 또는 추천 권리로는 이 작업을 할 수 없습니다.",
          PAPER_SOURCE_BLOCKED:
            "소스 입력 검증에서 거절됐습니다. 재시도 전에 세션 사유를 확인하세요.",
          PAPER_DISABLED: "Owner-beta Paper가 비활성입니다.",
          PAPER_COMMIT_UNKNOWN: "커밋 결과가 불확실합니다. 추가 작업 전에 상태를 다시 읽으세요.",
          STATE_UNAVAILABLE:
            "Owner-beta Paper 상태를 불러오지 못했습니다. 새로고침해 다시 시도하세요.",
        })[code] ?? "요청을 완료하지 못했습니다. 상태를 새로고침하고 현재 증거를 확인하세요.",
      retry: "상태 새로고침",
      consentTitle: "Owner 동의",
      consentMessage:
        "동의는 연결된 Owner 계좌에 대한 명시적 선택을 기록합니다. Paper 런타임을 활성화하지 않습니다.",
      consentCheckbox: "동의는 활성화가 아니며 Paper가 지연 EOD 모의를 사용함을 이해했습니다.",
      consentInButton: "동의 기록",
      consentOutButton: "동의 철회",
      consentSaved: "동의를 기록했습니다. 제어 상태는 별도로 관리됩니다.",
      activationNote: "이 화면에는 공개 활성화 제어가 없습니다.",
      autoBindingNote: "동의는 채택된 AUTO 동작에 연결됩니다. 수동/자동 전환은 제공하지 않습니다.",
      sessionsTitle: "Owner-beta 세션",
      noSessions: "아직 beta 세션이 없습니다.",
      runLabel: "완료된 추천 실행",
      noRuns: "연결된 전략의 완료 실행을 찾지 못했습니다.",
      previewButton: "Paper 미리보기 생성",
      previewCreating: "Paper 미리보기를 생성하고 있습니다…",
      previewWaiting: "미리보기를 준비하고 있습니다. 저장된 상태를 확인합니다.",
      previewFailed:
        "미리보기가 거절되었거나 실패했습니다. 새 미리보기 전에 사유 코드와 현재 증거를 확인하세요.",
      previewApplied: "미리보기를 아래 대기 대상에 적용했습니다.",
      previewApplyButton: "이 미리보기 적용",
      previewApplying: "미리보기를 적용하고 있습니다…",
      unknownCommit:
        "적용 결과가 불확실합니다. 상태 조회를 시도했습니다. 경제적 작업을 반복하지 말고 상태를 새로고침해 대기 대상을 확인하세요.",
      previewTitle: "미리보기 상세",
      previewStatus: "미리보기 상태",
      previewSignal: "신호일 T",
      previewEffective: "경제적 세션 E",
      costProfile: "비용 모형",
      costProfileVersion: "비용 모형 버전",
      signalDateHelp: "T는 추천 신호의 기준일입니다.",
      effectiveDateHelp:
        "E는 이후 경제 세션입니다. 미리보기 가격은 T 종가 기준 참고값이며, 지연 모의 정산은 EOD 데이터 승인 뒤 E의 시가를 사용합니다. 09:00 체결이 아니며 E 가격에 따라 수량과 비용이 달라질 수 있습니다.",
      previewAmount: "미리보기 금액",
      previewState: (state) =>
        paperLabels({
          PENDING: "대기열",
          RUNNING: "계산 중",
          READY: "준비 완료",
          FAILED: "실패",
          APPLIED: "적용 완료",
        })[state] ?? "알 수 없는 미리보기 상태",
      planStatus: (status) =>
        paperLabels({ REBALANCE: "리밸런싱 제안", NO_TRADE: "무거래 제안" })[status] ??
        "알 수 없는 계획",
      cashBefore: "미리보기 전 현금",
      availableCash: "사용 가능 현금",
      leftoverCash: "미리보기 후 잔여 현금",
      buyNotional: "제안 매수 금액",
      sellNotional: "제안 매도 금액",
      explicitFees: "모형 명시 비용",
      informationalSlippage: "참고 슬리피지 추정",
      previewTimedOut:
        "제한된 상태 확인 뒤에도 미리보기가 대기 중입니다. 상태를 새로고침해 계속 확인하세요.",
      previewOrders: "제안 주문",
      columnSide: "매매 방향",
      columnQuantity: "제안 수량",
      columnPrice: "입력 가격",
      previewOrderPrice: "T 종가 → 참고 체결가",
      priceObservation: "신호 관측 T",
      actualPrice: "Paper 주문 가격",
      actualDifferenceTitle: "미리보기와 정산 결과",
      actualDifferenceCount: (proposed, matched) =>
        `제안 행 ${proposed}개, 반환된 계좌 주문 페이지에서 연결된 실제 주문 ${matched}개입니다.`,
      decimalExactNote: "소수 값은 반환된 문자열 그대로 표시하며 여기서 반올림하지 않습니다.",
      proposedQuantity: "제안 수량",
      actualQuantity: "정산 주문 수량",
      actualOrderCount: (count) => `이 세션에 실제 주문 참조 ${count}개가 기록되었습니다.`,
      actualOrderDetailsCount: (matched, references) =>
        `주문 참조 ${references}개 중 ${matched}개가 계좌 페이지의 주문과 연결됐습니다. 나머지는 페이지 밖에 있거나 조회되지 않았을 수 있습니다.`,
      noActualOrders: "이 세션에 실제 주문 참조가 기록되지 않았습니다.",
      sessionDetails: "세션 상세",
      sessionState: (state) =>
        paperLabels({
          PENDING: "대기",
          WAITING_PRICE: "가격 데이터 대기",
          WAITING_ADMISSION: "데이터 승인 대기",
          HELD: "보류",
          BLOCKED: "차단",
          RECOVERY_REQUIRED: "복구 필요",
          SETTLED_TRADE: "거래 정산 완료",
          SETTLED_NO_TRADE: "무거래 정산 완료",
          VALUED: "평가 완료",
        })[state] ?? "알 수 없는 세션 상태",
      processingOrigin: (origin) =>
        paperLabels({ AUTO: "자동 처리", MANUAL: "수동 처리", RECOVERY: "복구 처리" })[origin] ??
        "알 수 없는 처리 경로",
      sessionSignal: "신호일 T",
      sessionEffective: "경제적 세션 E",
      sourceRetrievedAt: "원본 소스 수신 시각",
      admittedAt: "입력 승인 시각",
      usableAt: "가격 입력 사용 가능 시각",
      settledAt: "정산 처리 시각",
      valuedAt: "평가 처리 시각",
      valuationPending: "정산은 완료됐지만 종가 평가는 아직 대기 중입니다.",
      settlementKind: "정산 결과",
      settlementKindLabel: (kind) =>
        paperLabels({ TRADE: "거래", NO_TRADE: "무거래" })[kind ?? ""] ?? "기록 없음",
      reasonCode: "사유 코드",
      actualAmounts: "정산/평가 금액",
      cashAfter: "정산 후 현금",
      valuationCash: "평가 시 현금",
      positionsValue: "보유 포지션 가치",
      equity: "자산",
      orderReferences: "실제 주문 및 체결 참조",
      comparison: "백테스트 비교",
      comparisonDisposition: (disposition) =>
        paperLabels({ NOT_COMPARABLE: "비교 불가", PRICE_RETURN_ONLY: "가격 수익률만" })[
          disposition
        ] ?? "알 수 없는 비교",
      comparisonReason: "비교 사유",
      comparisonSummary:
        "Owner-beta Paper와 백테스트는 비교 불가입니다: NO_MATCHED_BACKTEST_EXECUTION_CONTRACT.",
      notComparable: "비교 불가",
      noBacktestPerformance: "백테스트 성과를 Paper 성과로 표시하지 않습니다.",
      noValue: "없음",
      refresh: "상태 새로고침",
    },
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
    columnAction: "액션",
    columnAveragePrice: "평균 단가",
    columnBacktest: "백테스트",
    columnBacktestWeight: "백테스트 비중",
    columnBound: "바인딩",
    columnCash: "현금",
    columnCommission: "수수료",
    columnComputedOn: "계산일",
    columnCurrentQuantity: "현재 수량",
    columnCurrentValue: "현재 가치",
    columnCurrentWeight: "현재 비중",
    columnDailyReturn: "일일 수익률",
    columnDate: "날짜",
    columnDelivery: "전송",
    columnDeltaValue: "변동 금액",
    columnEquity: "자산",
    columnEstimatedPrice: "예상 체결가",
    columnExecuted: "실행됨",
    columnExecutesAt: "실행일",
    columnField: "항목",
    columnInstrument: "종목",
    columnKind: "종류",
    columnNotice: "알림",
    columnNotional: "명목 금액",
    columnOrder: "주문",
    columnPaper: "모의투자",
    columnPaperWeight: "모의투자 비중",
    columnPositions: "포지션",
    columnPrice: "가격",
    columnQuantity: "수량",
    columnRaised: "발생 시각",
    columnRawPrice: "원시 가격",
    columnSide: "매매 구분",
    columnSkipReason: "건너뜀 사유",
    columnSlippage: "참고용 슬리피지",
    columnState: "상태",
    columnStrategy: "전략",
    columnTargetValue: "목표 가치",
    columnTargetWeight: "목표 비중",
    columnTax: "세금",
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
    rebalanceApplyButton: "미리보기 적용",
    rebalanceApplyFailedTitle: "적용 실패",
    rebalanceAppliedMessage: (effectiveDate) =>
      `적용되었습니다. 대기 중인 목표가 ${effectiveDate} 세션을 대상으로 합니다.`,
    rebalanceApplyingButton: "미리보기 적용 중",
    rebalanceAvailableCashLabel: "가용 현금",
    rebalanceBuyNotionalLabel: "매수 명목 금액",
    rebalanceCashBeforeLabel: "이전 현금",
    rebalanceCreateAriaLabel: "리밸런싱 미리보기 생성",
    rebalanceCreateButton: "미리보기 생성",
    rebalanceCreatingButton: "미리보기 생성 중",
    rebalanceDecisionsCaption: "리밸런싱 결정 사항",
    rebalanceEquityLabel: "자산",
    rebalanceErrorCodeLabel: "오류 코드",
    rebalanceEyebrow: "리밸런싱 미리보기",
    rebalanceExplicitFeesLabel: "명시적 수수료",
    rebalanceFailedTitle: "미리보기 실패",
    rebalanceHeading: "리밸런싱 미리보기",
    rebalanceIndicativeWarningMessage:
      "이 미리보기는 추천 마감가로 가격이 책정되지만, 실제 실행은 다음 세션 시가에 이루어집니다. 실행 전에 계획을 다시 계산해야 할 수 있습니다.",
    rebalanceIndicativeWarningTitle: "참고용 — 다음 시가 재계산 필요",
    rebalanceLeftoverCashLabel: "잔여 현금",
    rebalanceLineageAccountStateSha: "계좌 상태 SHA-256",
    rebalanceLineageAccountStateVersion: "계좌 상태 버전",
    rebalanceLineageCaption: "미리보기 계보",
    rebalanceLineageCuratedVersion: "큐레이션 버전",
    rebalanceLineageDatasetManifest: "데이터셋 매니페스트 SHA-256",
    rebalanceLineageDatasetVersion: "데이터셋 버전",
    rebalanceLineageRecommendationRun: "추천 실행",
    rebalanceLineageStrategyConfig: "전략 구성",
    rebalanceLineageTargetPortfolio: "목표 포트폴리오",
    rebalanceLineageTargetPortfolioSha: "목표 포트폴리오 SHA-256",
    rebalanceNoRunsMessage: "미리볼 수 있는 완료된 추천 실행이 아직 없습니다.",
    rebalanceNoSkipReason: "—",
    rebalanceOrdersCaption: "제안된 주문",
    rebalancePollingMessage: "미리보기 계산 중 — 잠시 후 다시 확인합니다.",
    rebalanceRunLabel: "추천 실행",
    rebalanceSellNotionalLabel: "매도 명목 금액",
    rebalanceStatusLabel: (status) => `미리보기 상태: ${status}`,
    rebalanceTimedOutMessage:
      "폴링 시간 내에 미리보기 계산이 끝나지 않았습니다. 다시 생성해 보세요.",
    rebalanceTimedOutTitle: "미리보기 시간 초과",
    rebalanceTotalsCaption: "미리보기 합계",
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
