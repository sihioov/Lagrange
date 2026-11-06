//! Pure Owner-beta Paper calculations over sealed Daily price evidence.
//!
//! The snapshots in this module are local immutable evidence only. They do
//! not grant database, rights, activation, or runtime authority.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset, Utc};
use domain::{
    ContentHash, Currency, FixedPoint, InstrumentId, Money, Price, Quantity, TradingDate, Weight,
};
use market_data::{
    HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS,
    daily_price_only::{
        DailyBaseV1Wire, DailyPriceManifestV1Wire, DailyPricePinsV1Wire, DailySessionTypeV1,
        calendar::CalendarRangeProof, input::AdmittedDailyPriceInputV1,
    },
};
use portfolio_model::{
    CostProfile, LedgerState, PendingTarget, SessionOpenPlan, SizingReport, SkipReason,
    TargetAllocation, close_valuation_event, plan_rebalance, plan_session_open,
    sizing::SizingInput,
};
use thiserror::Error;
use uuid::Uuid;

use crate::owner_beta::daily_input::{
    OwnerBetaDailyRecommendationInputV1, OwnerBetaDailyRecommendationPins,
};
use crate::owner_beta::daily_target::OwnerBetaDailyTargetSnapshotV1;

const ADOPTED_INSTRUMENT: &str = "069500.KRX";
const ADOPTED_STRATEGY: &str = "trend_following";
const ADOPTED_STRATEGY_VERSION: &str = "1.0.0";
const ADOPTED_COST_PROFILE: &str = "KRX_ETF_DEFAULT";
const PRICE_SCALE: u8 = 4;
const KRW: Currency = Currency::KRW;

/// Stable, payload-free reasons for a rejected or non-order calculation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaperCalculationReasonCode {
    InputInvalid,
    PriceInvalid,
    PriceMissing,
    DuplicatePrice,
    PriceNotFourDigitExact,
    RawAdjustedMismatch,
    PinMismatch,
    PrefixMismatch,
    CalendarInvalid,
    StrategyUnsupported,
    AssetUnsupported,
    CostProfileMismatch,
    AccountStateInvalid,
    ArithmeticInvalid,
    PriceUnavailable,
    FuturePrice,
    ProcessingDateMismatch,
    RecoveryRequired,
}

impl PaperCalculationReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputInvalid => "INPUT_INVALID",
            Self::PriceInvalid => "PRICE_INVALID",
            Self::PriceMissing => "PRICE_MISSING",
            Self::DuplicatePrice => "DUPLICATE_PRICE",
            Self::PriceNotFourDigitExact => "PRICE_NOT_FOUR_DIGIT_EXACT",
            Self::RawAdjustedMismatch => "RAW_ADJUSTED_MISMATCH",
            Self::PinMismatch => "PIN_MISMATCH",
            Self::PrefixMismatch => "PREFIX_MISMATCH",
            Self::CalendarInvalid => "CALENDAR_INVALID",
            Self::StrategyUnsupported => "STRATEGY_UNSUPPORTED",
            Self::AssetUnsupported => "ASSET_UNSUPPORTED",
            Self::CostProfileMismatch => "COST_PROFILE_MISMATCH",
            Self::AccountStateInvalid => "ACCOUNT_STATE_INVALID",
            Self::ArithmeticInvalid => "ARITHMETIC_INVALID",
            Self::PriceUnavailable => "PRICE_UNAVAILABLE",
            Self::FuturePrice => "FUTURE_PRICE",
            Self::ProcessingDateMismatch => "PROCESSING_DATE_MISMATCH",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
        }
    }
}

/// Calculation failures never contain provider bodies, paths, or free-form messages.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("owner-beta Paper calculation rejected: {0:?}")]
pub struct PaperCalculationError(pub PaperCalculationReasonCode);

impl From<PaperCalculationReasonCode> for PaperCalculationError {
    fn from(value: PaperCalculationReasonCode) -> Self {
        Self(value)
    }
}

type CalcResult<T> = Result<T, PaperCalculationError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RawPricePair {
    open: Price,
    close: Price,
}

/// A price snapshot can only be built from a verified admitted Daily input.
/// It retains exact bars internally for descendant-prefix comparison.
pub struct PaperPriceSnapshotV1 {
    as_of: TradingDate,
    pins: DailyPricePinsV1Wire,
    base: DailyBaseV1Wire,
    policy_sha256: ContentHash,
    bars: BTreeMap<TradingDate, BTreeMap<InstrumentId, HistoricalPriceOnlyBar>>,
}

impl std::fmt::Debug for PaperPriceSnapshotV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperPriceSnapshotV1")
            .field("as_of", &self.as_of)
            .field(
                "bar_count",
                &self.bars.values().map(BTreeMap::len).sum::<usize>(),
            )
            .finish_non_exhaustive()
    }
}

impl PaperPriceSnapshotV1 {
    /// Revalidates the admitted file evidence before and after extracting rows.
    pub fn from_admitted(admitted: &AdmittedDailyPriceInputV1<'_, '_>) -> CalcResult<Self> {
        admitted
            .revalidate()
            .map_err(|_| PaperCalculationReasonCode::InputInvalid)?;
        let snapshot = Self::from_verified_parts(
            admitted.manifest(),
            admitted.pins(),
            admitted.bars(),
            admitted.as_of(),
        )?;
        admitted
            .revalidate()
            .map_err(|_| PaperCalculationReasonCode::InputInvalid)?;
        Ok(snapshot)
    }

    /// A checked view is borrow-scoped; its owner must remain alive and be
    /// revalidated by the integration caller for the full calculation.
    #[cfg(target_os = "linux")]
    pub fn from_checked_view(
        view: &market_data::daily_price_only::admission::CheckedSealedView<'_>,
    ) -> CalcResult<Self> {
        Self::from_verified_parts(
            view.manifest(),
            view.pins(),
            view.bars(),
            view.manifest().range_end,
        )
    }

    fn from_verified_parts(
        manifest: &DailyPriceManifestV1Wire,
        pins: &DailyPricePinsV1Wire,
        bars: &[HistoricalPriceOnlyBar],
        as_of: TradingDate,
    ) -> CalcResult<Self> {
        if manifest.range_end != as_of
            || manifest.content_sha256 != pins.content_sha256
            || manifest.policy_sha256 != pins.policy_sha256
            || manifest.range_start > manifest.range_end
        {
            return Err(PaperCalculationReasonCode::PinMismatch.into());
        }
        let by_date = group_bars(bars, as_of)?;
        let snapshot = Self {
            as_of,
            pins: pins.clone(),
            base: manifest.base.clone(),
            policy_sha256: manifest.policy_sha256.clone(),
            bars: by_date,
        };
        snapshot.raw_prices_at(as_of)?;
        Ok(snapshot)
    }

    pub fn as_of(&self) -> TradingDate {
        self.as_of
    }

    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }

    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy_sha256
    }

    /// Raw closes only. These are descriptive values, not an action permit.
    fn raw_closes(&self, date: TradingDate) -> CalcResult<BTreeMap<InstrumentId, Price>> {
        self.raw_prices_at(date).map(|prices| {
            prices
                .into_iter()
                .map(|(instrument, pair)| (instrument, pair.close))
                .collect()
        })
    }

    /// Raw opens only. These are descriptive values, not an action permit.
    fn raw_opens(&self, date: TradingDate) -> CalcResult<BTreeMap<InstrumentId, Price>> {
        self.raw_prices_at(date).map(|prices| {
            prices
                .into_iter()
                .map(|(instrument, pair)| (instrument, pair.open))
                .collect()
        })
    }

    fn raw_prices_at(&self, date: TradingDate) -> CalcResult<BTreeMap<InstrumentId, RawPricePair>> {
        if date > self.as_of {
            return Err(PaperCalculationReasonCode::FuturePrice.into());
        }
        let rows = self
            .bars
            .get(&date)
            .ok_or(PaperCalculationReasonCode::PriceMissing)?;
        let allowed = etf11_instruments()?;
        if rows.keys().cloned().collect::<BTreeSet<_>>() != allowed {
            return Err(PaperCalculationReasonCode::PriceMissing.into());
        }
        rows.iter()
            .map(|(instrument, bar)| {
                let fields = [bar.raw_open, bar.raw_high, bar.raw_low, bar.raw_close];
                let adjusted = [
                    bar.adjusted_open,
                    bar.adjusted_high,
                    bar.adjusted_low,
                    bar.adjusted_close,
                ];
                if fields != adjusted {
                    return Err(PaperCalculationReasonCode::RawAdjustedMismatch.into());
                }
                let open = checked_price(bar.raw_open)?;
                let close = checked_price(bar.raw_close)?;
                Ok((instrument.clone(), RawPricePair { open, close }))
            })
            .collect()
    }
}

fn group_bars(
    bars: &[HistoricalPriceOnlyBar],
    as_of: TradingDate,
) -> CalcResult<BTreeMap<TradingDate, BTreeMap<InstrumentId, HistoricalPriceOnlyBar>>> {
    let allowed = etf11_instruments()?;
    let mut by_date =
        BTreeMap::<TradingDate, BTreeMap<InstrumentId, HistoricalPriceOnlyBar>>::new();
    for bar in bars {
        if bar.session_date > as_of || !allowed.contains(&bar.instrument_id) {
            return Err(PaperCalculationReasonCode::InputInvalid.into());
        }
        if by_date
            .entry(bar.session_date)
            .or_default()
            .insert(bar.instrument_id.clone(), bar.clone())
            .is_some()
        {
            return Err(PaperCalculationReasonCode::DuplicatePrice.into());
        }
    }
    Ok(by_date)
}

fn checked_price(value: FixedPoint) -> CalcResult<Price> {
    let rounded = value
        .with_scale(PRICE_SCALE)
        .map_err(|_| PaperCalculationReasonCode::PriceInvalid)?;
    if rounded.with_scale(value.scale()).ok() != Some(value) {
        return Err(PaperCalculationReasonCode::PriceNotFourDigitExact.into());
    }
    if !rounded.is_positive() {
        return Err(PaperCalculationReasonCode::PriceInvalid.into());
    }
    Price::from_fixed(rounded).map_err(|_| PaperCalculationReasonCode::PriceInvalid.into())
}

fn etf11_instruments() -> CalcResult<BTreeSet<InstrumentId>> {
    KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            InstrumentId::parse(&format!("{symbol}.KRX"))
                .map_err(|_| PaperCalculationReasonCode::InputInvalid.into())
        })
        .collect()
}

/// Confirms the full civil-date range is witnessed and T→E are adjacent
/// trading sessions under the same opaque calendar owner and policy.
pub fn validate_next_trading_session(
    t: &PaperPriceSnapshotV1,
    e: &PaperPriceSnapshotV1,
    proof: &CalendarRangeProof,
    expected_calendar_owner: Uuid,
    expected_calendar_policy: &ContentHash,
) -> CalcResult<()> {
    if proof.owner() != expected_calendar_owner
        || t.policy_sha256 != e.policy_sha256
        || proof.policy_sha256() != expected_calendar_policy
    {
        return Err(PaperCalculationReasonCode::CalendarInvalid.into());
    }
    validate_calendar_days(
        t.as_of,
        e.as_of,
        proof
            .days()
            .iter()
            .map(|day| (day.date(), day.session_type())),
    )
}

fn validate_calendar_days(
    t: TradingDate,
    e: TradingDate,
    days: impl IntoIterator<Item = (TradingDate, DailySessionTypeV1)>,
) -> CalcResult<()> {
    if t >= e {
        return Err(PaperCalculationReasonCode::CalendarInvalid.into());
    }
    let days = days.into_iter().collect::<Vec<_>>();
    let length = usize::try_from((e.as_naive_date() - t.as_naive_date()).num_days() + 1)
        .map_err(|_| PaperCalculationReasonCode::CalendarInvalid)?;
    if days.len() != length {
        return Err(PaperCalculationReasonCode::CalendarInvalid.into());
    }
    for (index, (date, session_type)) in days.iter().enumerate() {
        let expected = t
            .checked_add_days(
                i64::try_from(index).map_err(|_| PaperCalculationReasonCode::CalendarInvalid)?,
            )
            .map_err(|_| PaperCalculationReasonCode::CalendarInvalid)?;
        if *date != expected {
            return Err(PaperCalculationReasonCode::CalendarInvalid.into());
        }
        let is_endpoint = index == 0 || index + 1 == length;
        if is_endpoint != (*session_type == DailySessionTypeV1::Trading) {
            return Err(PaperCalculationReasonCode::CalendarInvalid.into());
        }
    }
    Ok(())
}

/// Confirms T and E share policy/base and E contains the exact T prefix.
pub fn verify_t_e_prefix(t: &PaperPriceSnapshotV1, e: &PaperPriceSnapshotV1) -> CalcResult<()> {
    if t.as_of >= e.as_of
        || t.policy_sha256 != e.policy_sha256
        || t.base != e.base
        || t.pins.content_sha256 == e.pins.content_sha256
        || t.pins.manifest_sha256 == e.pins.manifest_sha256
        || t.pins.admission_sha256 == e.pins.admission_sha256
    {
        return Err(PaperCalculationReasonCode::PinMismatch.into());
    }
    let t_prefix = t
        .bars
        .range(..=t.as_of)
        .flat_map(|(date, rows)| {
            rows.iter()
                .map(move |(instrument, bar)| ((*date, instrument), bar))
        })
        .collect::<BTreeMap<_, _>>();
    let e_prefix = e
        .bars
        .range(..=t.as_of)
        .flat_map(|(date, rows)| {
            rows.iter()
                .map(move |(instrument, bar)| ((*date, instrument), bar))
        })
        .collect::<BTreeMap<_, _>>();
    if t_prefix != e_prefix {
        return Err(PaperCalculationReasonCode::PrefixMismatch.into());
    }
    Ok(())
}

/// Provider timestamps are observations supplied by integration, not proof
/// of rights or runtime authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceAvailabilityObservation {
    pub source_original_retrieved_at: DateTime<Utc>,
    pub input_admitted_at: DateTime<Utc>,
    pub usable_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceUse {
    SignalT,
    ApplyT,
    SettleE,
    ValueE,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvailabilityDecision {
    Ready,
    RecoveryRequired,
}

/// Validates observed availability and the exact KST session date. It never
/// invents a market-open or market-close timestamp.
pub fn validate_price_availability(
    session: TradingDate,
    effective_session: TradingDate,
    processing_at: DateTime<Utc>,
    observation: PriceAvailabilityObservation,
    purpose: PriceUse,
) -> CalcResult<AvailabilityDecision> {
    match purpose {
        PriceUse::SignalT | PriceUse::ApplyT if session >= effective_session => {
            return Err(PaperCalculationReasonCode::CalendarInvalid.into());
        }
        PriceUse::SettleE | PriceUse::ValueE if session != effective_session => {
            return Err(PaperCalculationReasonCode::CalendarInvalid.into());
        }
        _ => {}
    }
    if observation.usable_at < observation.source_original_retrieved_at
        || observation.usable_at < observation.input_admitted_at
        || observation.source_original_retrieved_at > processing_at
        || observation.input_admitted_at > processing_at
        || observation.usable_at > processing_at
    {
        return Err(PaperCalculationReasonCode::PriceUnavailable.into());
    }
    let kst =
        FixedOffset::east_opt(9 * 60 * 60).ok_or(PaperCalculationReasonCode::ArithmeticInvalid)?;
    let processing_date = processing_at.with_timezone(&kst).date_naive();
    let session_date = session.as_naive_date();
    if session_date > processing_date {
        return Err(PaperCalculationReasonCode::FuturePrice.into());
    }
    if matches!(purpose, PriceUse::SettleE | PriceUse::ValueE)
        && processing_date > effective_session.as_naive_date()
    {
        return Ok(AvailabilityDecision::RecoveryRequired);
    }
    if processing_date != session_date {
        return Err(PaperCalculationReasonCode::ProcessingDateMismatch.into());
    }
    let next_midnight = session_date
        .succ_opt()
        .ok_or(PaperCalculationReasonCode::ArithmeticInvalid)?
        .and_hms_opt(0, 0, 0)
        .ok_or(PaperCalculationReasonCode::ArithmeticInvalid)?;
    let exclusive_end = next_midnight
        .and_local_timezone(kst)
        .single()
        .ok_or(PaperCalculationReasonCode::ArithmeticInvalid)?
        .with_timezone(&Utc);
    if processing_at >= exclusive_end {
        return Err(PaperCalculationReasonCode::ProcessingDateMismatch.into());
    }
    Ok(AvailabilityDecision::Ready)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetaAccountStateIdentity {
    pub account_id: Uuid,
    pub state_version: i64,
    pub state_sha256: ContentHash,
}

/// Daily price and target lineage plus the caller's actual account readback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetaPreviewLineageV1 {
    run_id: Uuid,
    strategy_config_id: Uuid,
    strategy_id: String,
    strategy_version: String,
    strategy_config_sha256: ContentHash,
    factor_snapshot_sha256: ContentHash,
    target_snapshot_sha256: ContentHash,
    price_pins: OwnerBetaDailyRecommendationPins,
    account: BetaAccountStateIdentity,
}

impl BetaPreviewLineageV1 {
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
    pub fn strategy_config_id(&self) -> Uuid {
        self.strategy_config_id
    }
    pub fn strategy_id(&self) -> &str {
        &self.strategy_id
    }
    pub fn strategy_version(&self) -> &str {
        &self.strategy_version
    }
    pub fn strategy_config_sha256(&self) -> &ContentHash {
        &self.strategy_config_sha256
    }
    pub fn factor_snapshot_sha256(&self) -> &ContentHash {
        &self.factor_snapshot_sha256
    }
    pub fn target_snapshot_sha256(&self) -> &ContentHash {
        &self.target_snapshot_sha256
    }
    pub fn price_pins(&self) -> &OwnerBetaDailyRecommendationPins {
        &self.price_pins
    }
    pub fn account(&self) -> &BetaAccountStateIdentity {
        &self.account
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetaNoTradeReason {
    SignalUnchanged,
    CashTarget,
    Sizer(SkipReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetaPlanStatus {
    Rebalance,
    NoTrade,
}

/// Non-serializable calculation result; persistence and authorization belong
/// to the integration layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetaPreviewResultV1 {
    status: BetaPlanStatus,
    no_trade_reasons: Vec<BetaNoTradeReason>,
    report: Option<SizingReport>,
    lineage: BetaPreviewLineageV1,
    raw_close_prices: BTreeMap<InstrumentId, Price>,
}

impl BetaPreviewResultV1 {
    pub fn status(&self) -> BetaPlanStatus {
        self.status
    }
    pub fn no_trade_reasons(&self) -> &[BetaNoTradeReason] {
        &self.no_trade_reasons
    }
    pub fn report(&self) -> Option<&SizingReport> {
        self.report.as_ref()
    }
    pub fn lineage(&self) -> &BetaPreviewLineageV1 {
        &self.lineage
    }
    pub fn raw_close_prices(&self) -> &BTreeMap<InstrumentId, Price> {
        &self.raw_close_prices
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BetaExecutionDisposition {
    Executed,
    NoTrade(Vec<BetaNoTradeReason>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BetaExecutionResultV1 {
    disposition: BetaExecutionDisposition,
    plan: Option<SessionOpenPlan>,
    state: LedgerState,
    raw_open_prices: BTreeMap<InstrumentId, Price>,
}

impl BetaExecutionResultV1 {
    pub fn disposition(&self) -> &BetaExecutionDisposition {
        &self.disposition
    }
    pub fn plan(&self) -> Option<&SessionOpenPlan> {
        self.plan.as_ref()
    }
    pub fn state(&self) -> &LedgerState {
        &self.state
    }
    pub fn raw_open_prices(&self) -> &BTreeMap<InstrumentId, Price> {
        &self.raw_open_prices
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetaValuationResultV1 {
    pub cash: Money,
    pub positions_value: Money,
    pub equity: Money,
    pub state: LedgerState,
    pub raw_close_prices: BTreeMap<InstrumentId, Price>,
}

/// Builds preview lineage and uses only raw close(T) with the shared sizer.
pub fn calculate_beta_preview(
    prices: &PaperPriceSnapshotV1,
    input: &OwnerBetaDailyRecommendationInputV1,
    target: &OwnerBetaDailyTargetSnapshotV1,
    account_identity: BetaAccountStateIdentity,
    previous_applied_target: Option<&[TargetAllocation]>,
    state: &LedgerState,
) -> CalcResult<BetaPreviewResultV1> {
    let targets = validate_target(input, target, prices)?;
    validate_ledger_state(state)?;
    if account_identity.account_id.is_nil() || account_identity.state_version < 0 {
        return Err(PaperCalculationReasonCode::AccountStateInvalid.into());
    }
    let closes = prices.raw_closes(input.as_of())?;
    let lineage = BetaPreviewLineageV1 {
        run_id: input.run_id(),
        strategy_config_id: input.strategy_config_id(),
        strategy_id: target.strategy_id().to_owned(),
        strategy_version: target.strategy_version().to_owned(),
        strategy_config_sha256: target.strategy_config_sha256().clone(),
        factor_snapshot_sha256: target.factor_snapshot_sha256().clone(),
        target_snapshot_sha256: target.target_snapshot_sha256().clone(),
        price_pins: input.pins().clone(),
        account: account_identity,
    };
    if previous_target_matches(previous_applied_target, &targets) {
        return Ok(BetaPreviewResultV1 {
            status: BetaPlanStatus::NoTrade,
            no_trade_reasons: vec![BetaNoTradeReason::SignalUnchanged],
            report: None,
            lineage,
            raw_close_prices: closes,
        });
    }
    let report = plan_rebalance(&SizingInput {
        cash: state.cash,
        positions: state.positions.clone(),
        open_prices: closes.clone(),
        targets: targets.clone(),
        lot_sizes: lot_sizes_for(&targets, &state.positions),
        profile: state.cost_profile.clone(),
    })
    .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    let no_trade_reasons = collect_no_trade_reasons(&report, &targets, false);
    let status = if report.orders.is_empty() {
        BetaPlanStatus::NoTrade
    } else {
        BetaPlanStatus::Rebalance
    };
    Ok(BetaPreviewResultV1 {
        status,
        no_trade_reasons,
        report: Some(report),
        lineage,
        raw_close_prices: closes,
    })
}

/// Plans and applies a changed target at raw open(E). The previous target is
/// supplied by the integration's fenced account readback; SQL state fencing
/// is intentionally outside this pure function.
#[allow(
    clippy::too_many_arguments,
    reason = "Keep independently verified T/E prices, recommendation, calendar, and ledger inputs explicit"
)]
pub fn execute_beta_session_open(
    t_prices: &PaperPriceSnapshotV1,
    e_prices: &PaperPriceSnapshotV1,
    input: &OwnerBetaDailyRecommendationInputV1,
    target: &OwnerBetaDailyTargetSnapshotV1,
    account_id: Uuid,
    effective_session: TradingDate,
    calendar_proof: &CalendarRangeProof,
    expected_calendar_owner: Uuid,
    expected_calendar_policy: &ContentHash,
    previous_applied_target: Option<&[TargetAllocation]>,
    state: &LedgerState,
) -> CalcResult<BetaExecutionResultV1> {
    let targets = validate_target(input, target, t_prices)?;
    validate_ledger_state(state)?;
    if effective_session != e_prices.as_of || effective_session <= input.as_of() {
        return Err(PaperCalculationReasonCode::CalendarInvalid.into());
    }
    validate_next_trading_session(
        t_prices,
        e_prices,
        calendar_proof,
        expected_calendar_owner,
        expected_calendar_policy,
    )?;
    verify_t_e_prefix(t_prices, e_prices)?;
    let opens = e_prices.raw_opens(effective_session)?;
    if previous_target_matches(previous_applied_target, &targets) {
        return Ok(BetaExecutionResultV1 {
            disposition: BetaExecutionDisposition::NoTrade(vec![
                BetaNoTradeReason::SignalUnchanged,
            ]),
            plan: None,
            state: state.clone(),
            raw_open_prices: opens,
        });
    }
    let pending = PendingTarget {
        account_id,
        effective_date: effective_session,
        targets: targets.clone(),
    };
    let plan = plan_session_open(
        state,
        &pending,
        &effective_session,
        &opens,
        &lot_sizes_for(&targets, &state.positions),
    )
    .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    if plan.events.is_empty() {
        let reasons = collect_no_trade_reasons(&plan.report, &targets, false);
        return Ok(BetaExecutionResultV1 {
            disposition: BetaExecutionDisposition::NoTrade(reasons),
            plan: Some(plan),
            state: state.clone(),
            raw_open_prices: opens,
        });
    }
    let mut applied = state.clone();
    for event in &plan.events {
        applied
            .apply(event.clone())
            .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    }
    validate_ledger_state(&applied)?;
    Ok(BetaExecutionResultV1 {
        disposition: BetaExecutionDisposition::Executed,
        plan: Some(plan),
        state: applied,
        raw_open_prices: opens,
    })
}

/// Marks every held position at raw close(E) with the shared ledger event and
/// verifies cash + marked holdings equals the committed equity curve value.
pub fn value_beta_close(
    prices: &PaperPriceSnapshotV1,
    effective_session: TradingDate,
    state: &LedgerState,
) -> CalcResult<BetaValuationResultV1> {
    validate_ledger_state(state)?;
    if effective_session != prices.as_of {
        return Err(PaperCalculationReasonCode::PinMismatch.into());
    }
    let closes = prices.raw_closes(effective_session)?;
    let event = close_valuation_event(state, effective_session, &closes)
        .map_err(|_| PaperCalculationReasonCode::PriceMissing)?;
    let mut applied = state.clone();
    applied
        .apply(event)
        .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    let mut positions_value = Money::zero(KRW);
    for (instrument, quantity) in &applied.positions {
        let close = closes
            .get(instrument)
            .ok_or(PaperCalculationReasonCode::PriceMissing)?;
        positions_value = positions_value
            .checked_add(
                &quantity
                    .checked_mul_price(close, KRW)
                    .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?,
            )
            .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    }
    let expected_equity = applied
        .cash
        .checked_add(&positions_value)
        .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    let equity = applied
        .equity_curve
        .get(&effective_session)
        .copied()
        .ok_or(PaperCalculationReasonCode::ArithmeticInvalid)?;
    if expected_equity != equity {
        return Err(PaperCalculationReasonCode::ArithmeticInvalid.into());
    }
    Ok(BetaValuationResultV1 {
        cash: applied.cash,
        positions_value,
        equity,
        state: applied,
        raw_close_prices: closes,
    })
}

fn validate_target(
    input: &OwnerBetaDailyRecommendationInputV1,
    target: &OwnerBetaDailyTargetSnapshotV1,
    prices: &PaperPriceSnapshotV1,
) -> CalcResult<Vec<TargetAllocation>> {
    input
        .validate()
        .map_err(|_| PaperCalculationReasonCode::InputInvalid)?;
    target
        .validate_hash()
        .map_err(|_| PaperCalculationReasonCode::InputInvalid)?;
    if prices.as_of != input.as_of()
        || target.as_of() != input.as_of()
        || target.pins() != input.pins()
        || target.strategy_id() != ADOPTED_STRATEGY
        || target.strategy_version() != ADOPTED_STRATEGY_VERSION
        || target.strategy_config_sha256() != input.strategy_snapshot().config_sha256()
    {
        return Err(PaperCalculationReasonCode::PinMismatch.into());
    }
    let config = input.strategy_snapshot().config_json();
    if config
        .get("benchmark_instrument")
        .and_then(serde_json::Value::as_str)
        != Some(ADOPTED_INSTRUMENT)
        || config.get("fast_ma").and_then(serde_json::Value::as_u64) != Some(50)
        || config.get("slow_ma").and_then(serde_json::Value::as_u64) != Some(200)
    {
        return Err(PaperCalculationReasonCode::StrategyUnsupported.into());
    }
    if &prices.pins.content_sha256 != input.pins().content_sha256()
        || &prices.pins.manifest_sha256 != input.pins().manifest_sha256()
        || &prices.pins.admission_sha256 != input.pins().admission_sha256()
        || &prices.pins.policy_sha256 != input.pins().policy_sha256()
    {
        return Err(PaperCalculationReasonCode::PinMismatch.into());
    }
    let mut seen = BTreeSet::new();
    let mut target_ppm = None;
    for item in target.items() {
        if !seen.insert(item.instrument_id().to_owned()) {
            return Err(PaperCalculationReasonCode::AssetUnsupported.into());
        }
        if item.instrument_id() == ADOPTED_INSTRUMENT {
            target_ppm = item.target_weight_ppm();
        } else if item.target_weight_ppm().is_some_and(|weight| weight > 0) {
            return Err(PaperCalculationReasonCode::AssetUnsupported.into());
        }
    }
    let weight_ppm = target_ppm.ok_or(PaperCalculationReasonCode::StrategyUnsupported)?;
    if !matches!(weight_ppm, 0 | 1_000_000)
        || target.cash_weight_ppm().checked_add(weight_ppm) != Some(1_000_000)
    {
        return Err(PaperCalculationReasonCode::StrategyUnsupported.into());
    }
    if weight_ppm == 0 {
        return Ok(Vec::new());
    }
    let fixed = FixedPoint::from_i128(i128::from(weight_ppm), 6)
        .map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    let weight =
        Weight::from_fixed(fixed).map_err(|_| PaperCalculationReasonCode::ArithmeticInvalid)?;
    let instrument_id = InstrumentId::parse(ADOPTED_INSTRUMENT)
        .map_err(|_| PaperCalculationReasonCode::AssetUnsupported)?;
    Ok(vec![TargetAllocation {
        instrument_id,
        weight,
    }])
}

fn validate_ledger_state(state: &LedgerState) -> CalcResult<()> {
    let profile = CostProfile::resolve(ADOPTED_COST_PROFILE)
        .map_err(|_| PaperCalculationReasonCode::CostProfileMismatch)?;
    if state.cost_profile != profile {
        return Err(PaperCalculationReasonCode::CostProfileMismatch.into());
    }
    if state.base_currency != KRW
        || state.cash.currency() != KRW
        || state.cash.amount().is_negative()
    {
        return Err(PaperCalculationReasonCode::AccountStateInvalid.into());
    }
    let allowed = InstrumentId::parse(ADOPTED_INSTRUMENT)
        .map_err(|_| PaperCalculationReasonCode::AssetUnsupported)?;
    if state
        .positions
        .keys()
        .any(|instrument| instrument != &allowed)
    {
        return Err(PaperCalculationReasonCode::AssetUnsupported.into());
    }
    Ok(())
}

fn lot_sizes_for(
    targets: &[TargetAllocation],
    positions: &BTreeMap<InstrumentId, Quantity>,
) -> BTreeMap<InstrumentId, u64> {
    targets
        .iter()
        .map(|target| target.instrument_id.clone())
        .chain(positions.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|instrument| (instrument, 1))
        .collect()
}

fn previous_target_matches(
    previous: Option<&[TargetAllocation]>,
    current: &[TargetAllocation],
) -> bool {
    previous.is_some_and(|previous| previous == current)
}

fn collect_no_trade_reasons(
    report: &SizingReport,
    targets: &[TargetAllocation],
    unchanged: bool,
) -> Vec<BetaNoTradeReason> {
    if unchanged {
        return vec![BetaNoTradeReason::SignalUnchanged];
    }
    let mut reasons = report
        .decisions
        .iter()
        .filter_map(|decision| match decision.action {
            portfolio_model::SizingAction::Skip(reason) => Some(BetaNoTradeReason::Sizer(reason)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if targets.is_empty() && reasons.is_empty() {
        reasons.push(BetaNoTradeReason::CashTarget);
    }
    reasons.sort_by_key(|reason| match reason {
        BetaNoTradeReason::SignalUnchanged => (0, ""),
        BetaNoTradeReason::CashTarget => (1, ""),
        BetaNoTradeReason::Sizer(_) => (2, ""),
    });
    reasons.dedup();
    reasons
}

// Explicitly synthetic row helpers below are private to unit tests. They do
// not construct public opaque capabilities or stand in for admitted inputs.
#[cfg(test)]
fn synthetic_bar(
    date: TradingDate,
    instrument: InstrumentId,
    open: &str,
    close: &str,
) -> HistoricalPriceOnlyBar {
    let open = FixedPoint::parse(open).expect("synthetic open");
    let close = FixedPoint::parse(close).expect("synthetic close");
    HistoricalPriceOnlyBar {
        instrument_id: instrument,
        session_date: date,
        raw_open: open,
        raw_high: open.max(close),
        raw_low: open.min(close),
        raw_close: close,
        raw_volume: 1,
        raw_trading_value: None,
        adjusted_open: open,
        adjusted_high: open.max(close),
        adjusted_low: open.min(close),
        adjusted_close: close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn instrument() -> InstrumentId {
        InstrumentId::parse(ADOPTED_INSTRUMENT).unwrap()
    }
    fn state(cash: &str) -> LedgerState {
        LedgerState::new(
            Money::parse(cash, KRW).unwrap(),
            CostProfile::resolve(ADOPTED_COST_PROFILE).unwrap(),
        )
    }
    fn targets(weight: &str) -> Vec<TargetAllocation> {
        if weight == "0" {
            return Vec::new();
        }
        vec![TargetAllocation {
            instrument_id: instrument(),
            weight: Weight::from_fixed(FixedPoint::parse(weight).unwrap()).unwrap(),
        }]
    }
    fn price_map(value: &str) -> BTreeMap<InstrumentId, Price> {
        BTreeMap::from([(instrument(), Price::parse(value).unwrap())])
    }
    fn complete_cal_days(
        t: TradingDate,
        middle: DailySessionTypeV1,
        e: TradingDate,
    ) -> Vec<(TradingDate, DailySessionTypeV1)> {
        vec![
            (t, DailySessionTypeV1::Trading),
            (t.checked_add_days(1).unwrap(), middle),
            (e, DailySessionTypeV1::Trading),
        ]
    }
    fn synthetic_base() -> DailyBaseV1Wire {
        DailyBaseV1Wire {
            kind: "historical_price_only_v3".to_owned(),
            pins: market_data::daily_price_only::DailyBasePinsV1Wire {
                candidate_content_sha256: ContentHash::from_bytes(b"1"),
                artifact_manifest_sha256: ContentHash::from_bytes(b"2"),
                stage5_manifest_sha256: ContentHash::from_bytes(b"3"),
                action_manifest_sha256: ContentHash::from_bytes(b"4"),
                approval_registry_sha256: ContentHash::from_bytes(b"5"),
            },
        }
    }
    fn synthetic_pins(suffix: &[u8]) -> DailyPricePinsV1Wire {
        DailyPricePinsV1Wire {
            content_sha256: ContentHash::from_bytes([suffix, b"content"].concat().as_slice()),
            manifest_sha256: ContentHash::from_bytes([suffix, b"manifest"].concat().as_slice()),
            admission_sha256: ContentHash::from_bytes([suffix, b"admission"].concat().as_slice()),
            policy_sha256: ContentHash::from_bytes(b"policy"),
        }
    }
    fn synthetic_snapshot(
        as_of: TradingDate,
        pins: DailyPricePinsV1Wire,
        bars: Vec<HistoricalPriceOnlyBar>,
    ) -> PaperPriceSnapshotV1 {
        let mut grouped =
            BTreeMap::<TradingDate, BTreeMap<InstrumentId, HistoricalPriceOnlyBar>>::new();
        for bar in bars {
            grouped
                .entry(bar.session_date)
                .or_default()
                .insert(bar.instrument_id.clone(), bar);
        }
        PaperPriceSnapshotV1 {
            as_of,
            pins,
            base: synthetic_base(),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            bars: grouped,
        }
    }
    fn synthetic_day_bars(
        date: TradingDate,
        open: &str,
        close: &str,
    ) -> Vec<HistoricalPriceOnlyBar> {
        KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| {
                let instrument = InstrumentId::parse(&format!("{symbol}.KRX")).unwrap();
                synthetic_bar(date, instrument, open, close)
            })
            .collect()
    }

    #[test]
    fn preview_close_and_execution_open_produce_different_integer_quantities() {
        let mut initial = state("10000000");
        let allocations = targets("1");
        let close_report = plan_rebalance(&SizingInput {
            cash: initial.cash,
            positions: initial.positions.clone(),
            open_prices: price_map("100000"),
            targets: allocations.clone(),
            lot_sizes: BTreeMap::new(),
            profile: initial.cost_profile.clone(),
        })
        .unwrap();
        let target = PendingTarget {
            account_id: Uuid::from_u128(7),
            effective_date: date("2026-11-03"),
            targets: allocations,
        };
        let open_plan = plan_session_open(
            &initial,
            &target,
            &date("2026-11-03"),
            &price_map("200000"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            close_report.orders[0].quantity,
            Quantity::parse("99").unwrap()
        );
        assert_eq!(
            open_plan.report.orders[0].quantity,
            Quantity::parse("49").unwrap()
        );
        for event in open_plan.events {
            initial.apply(event).unwrap();
        }
        assert!(initial.cash.amount().is_positive());
    }

    #[test]
    fn shared_ledger_counts_minimum_commission_once_and_slippage_only_in_fill_price() {
        let mut ledger = state("1000000");
        let target = PendingTarget {
            account_id: Uuid::from_u128(9),
            effective_date: date("2026-11-03"),
            targets: targets("1"),
        };
        let plan = plan_session_open(
            &ledger,
            &target,
            &date("2026-11-03"),
            &price_map("100000"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(plan.report.orders.len(), 1);
        assert_eq!(
            plan.report.orders[0].estimated_fees,
            Money::parse("1000", KRW).unwrap()
        );
        let fill_price = match &plan.events[1] {
            portfolio_model::LedgerEvent::Fill { price, .. } => *price,
            _ => panic!("expected fill"),
        };
        assert_eq!(fill_price, FixedPoint::parse("100100").unwrap());
        for event in plan.events {
            ledger.apply(event).unwrap();
        }
        assert!(ledger.cash.amount().is_positive());
        let fill = ledger.fills.last().unwrap();
        assert_eq!(fill.commission, Money::parse("1000", KRW).unwrap());
        let fill_notional = fill.quantity.checked_mul_price(&fill.price, KRW).unwrap();
        let expected_cash = Money::parse("1000000", KRW)
            .unwrap()
            .checked_sub(&fill_notional)
            .unwrap()
            .checked_sub(&fill.commission)
            .unwrap()
            .checked_sub(&fill.tax)
            .unwrap();
        assert_eq!(
            ledger.cash, expected_cash,
            "slippage is embedded once in fill price"
        );
    }

    #[test]
    fn unchanged_target_after_zero_lot_attempt_is_typed_no_trade() {
        let current = targets("1");
        assert!(previous_target_matches(Some(&current), &current));
        let small = LedgerState::new(
            Money::parse("50000", KRW).unwrap(),
            CostProfile::resolve(ADOPTED_COST_PROFILE).unwrap(),
        );
        let report = plan_rebalance(&SizingInput {
            cash: small.cash,
            positions: small.positions.clone(),
            open_prices: price_map("100000"),
            targets: current.clone(),
            lot_sizes: BTreeMap::from([(instrument(), 1)]),
            profile: small.cost_profile,
        })
        .unwrap();
        assert!(report.orders.is_empty());
        assert!(report.decisions.iter().any(|decision| matches!(
            decision.action,
            portfolio_model::SizingAction::Skip(SkipReason::NoAffordableLot { .. })
                | portfolio_model::SizingAction::Skip(SkipReason::NoAvailableCash)
                | portfolio_model::SizingAction::Skip(SkipReason::BelowMinTrade { .. })
        )));
        assert_eq!(
            collect_no_trade_reasons(&report, &current, true),
            vec![BetaNoTradeReason::SignalUnchanged]
        );
        let cash_report = SizingReport {
            equity: report.equity,
            available_cash: report.available_cash,
            leftover_cash: report.leftover_cash,
            decisions: Vec::new(),
            orders: Vec::new(),
        };
        assert_eq!(
            collect_no_trade_reasons(&cash_report, &[], false),
            vec![BetaNoTradeReason::CashTarget]
        );
    }

    #[test]
    fn exact_prefix_comparison_detects_bar_mutation() {
        // Synthetic local rows exercise only the private exact-row comparator.
        let t = date("2026-11-02");
        let e = date("2026-11-04");
        let t_bars = synthetic_day_bars(t, "100", "101");
        let mut e_bars = t_bars.clone();
        e_bars.extend(synthetic_day_bars(e, "102", "103"));
        let t_snapshot = synthetic_snapshot(t, synthetic_pins(b"T"), t_bars.clone());
        let e_snapshot = synthetic_snapshot(e, synthetic_pins(b"E"), e_bars.clone());
        verify_t_e_prefix(&t_snapshot, &e_snapshot).unwrap();
        e_bars[0].raw_close = FixedPoint::parse("104").unwrap();
        e_bars[0].adjusted_close = FixedPoint::parse("104").unwrap();
        let tampered = synthetic_snapshot(e, synthetic_pins(b"F"), e_bars);
        assert_eq!(
            verify_t_e_prefix(&t_snapshot, &tampered),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PrefixMismatch
            ))
        );
    }

    #[test]
    fn missing_and_future_prices_are_rejected_by_the_date_bounded_snapshot() {
        let t = date("2026-11-02");
        let rows = BTreeMap::from([(
            t,
            KR_ETF_CORE_SYMBOLS
                .iter()
                .map(|symbol| {
                    let instrument = InstrumentId::parse(&format!("{symbol}.KRX")).unwrap();
                    (
                        instrument.clone(),
                        synthetic_bar(t, instrument, "100", "101"),
                    )
                })
                .collect(),
        )]);
        let snapshot = PaperPriceSnapshotV1 {
            as_of: t,
            pins: synthetic_pins(b"T"),
            base: synthetic_base(),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            bars: rows,
        };
        assert_eq!(snapshot.raw_prices_at(t).unwrap().len(), 11);
        assert_eq!(
            snapshot.raw_prices_at(date("2026-11-03")),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::FuturePrice
            ))
        );
        assert_eq!(
            snapshot.raw_prices_at(date("2026-11-01")),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PriceMissing
            ))
        );
        let missing_rows = KR_ETF_CORE_SYMBOLS
            .iter()
            .take(10)
            .map(|symbol| {
                let instrument = InstrumentId::parse(&format!("{symbol}.KRX")).unwrap();
                (
                    instrument.clone(),
                    synthetic_bar(t, instrument, "100", "101"),
                )
            })
            .collect();
        let missing = PaperPriceSnapshotV1 {
            as_of: t,
            pins: synthetic_pins(b"M"),
            base: synthetic_base(),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            bars: BTreeMap::from([(t, missing_rows)]),
        };
        assert_eq!(
            missing.raw_prices_at(t),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PriceMissing
            ))
        );
    }

    #[test]
    fn selected_price_requires_positive_four_digit_exact_raw_adjusted_values() {
        assert_eq!(
            checked_price(FixedPoint::parse("0").unwrap()),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PriceInvalid
            ))
        );
        assert_eq!(
            checked_price(FixedPoint::parse("1.00001").unwrap()),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PriceNotFourDigitExact
            ))
        );
        let t = date("2026-11-02");
        let mut rows = synthetic_day_bars(t, "100", "101")
            .into_iter()
            .map(|bar| (bar.instrument_id.clone(), bar))
            .collect::<BTreeMap<_, _>>();
        rows.get_mut(&instrument()).unwrap().adjusted_close = FixedPoint::parse("102").unwrap();
        let snapshot = PaperPriceSnapshotV1 {
            as_of: t,
            pins: synthetic_pins(b"A"),
            base: synthetic_base(),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            bars: BTreeMap::from([(t, rows)]),
        };
        assert_eq!(
            snapshot.raw_prices_at(t),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::RawAdjustedMismatch
            ))
        );
        let duplicate = synthetic_bar(t, instrument(), "100", "101");
        assert_eq!(
            group_bars(&[duplicate.clone(), duplicate], t),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::DuplicatePrice
            ))
        );
    }

    #[test]
    fn calendar_range_requires_gap_free_closed_days_between_trading_endpoints() {
        let t = date("2026-11-02");
        let e = date("2026-11-04");
        assert!(
            validate_calendar_days(t, e, complete_cal_days(t, DailySessionTypeV1::Closed, e))
                .is_ok()
        );
        assert_eq!(
            validate_calendar_days(
                t,
                e,
                vec![
                    (t, DailySessionTypeV1::Trading),
                    (e, DailySessionTypeV1::Trading)
                ]
            ),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::CalendarInvalid
            ))
        );
        assert_eq!(
            validate_calendar_days(t, e, complete_cal_days(t, DailySessionTypeV1::Trading, e)),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::CalendarInvalid
            ))
        );
    }

    #[test]
    fn profile_asset_cash_quantity_and_nav_invariants_are_checked() {
        let valid = state("1000000");
        validate_ledger_state(&valid).unwrap();
        let mut wrong_profile = valid.clone();
        wrong_profile.cost_profile = CostProfile::custom("0.001", "0", "0", 0, "0", "0").unwrap();
        assert_eq!(
            validate_ledger_state(&wrong_profile),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::CostProfileMismatch
            ))
        );

        let mut unrelated_asset = valid.clone();
        unrelated_asset.positions.insert(
            InstrumentId::parse("005930.KRX").unwrap(),
            Quantity::parse("1").unwrap(),
        );
        assert_eq!(
            validate_ledger_state(&unrelated_asset),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::AssetUnsupported
            ))
        );

        let target = PendingTarget {
            account_id: Uuid::from_u128(3),
            effective_date: date("2026-11-03"),
            targets: targets("1"),
        };
        let plan = plan_session_open(
            &valid,
            &target,
            &date("2026-11-03"),
            &price_map("100000"),
            &BTreeMap::new(),
        )
        .unwrap();
        let mut applied = valid;
        for event in plan.events {
            applied.apply(event).unwrap();
        }
        let close_event =
            close_valuation_event(&applied, date("2026-11-03"), &price_map("100000")).unwrap();
        applied.apply(close_event).unwrap();
        let shares = applied
            .positions
            .get(&instrument())
            .copied()
            .unwrap_or(Quantity::zero().unwrap());
        assert!(shares.amount().is_integer());
        let value = shares
            .checked_mul_price(&Price::parse("100000").unwrap(), KRW)
            .unwrap();
        let expected_nav = applied.cash.checked_add(&value).unwrap();
        assert_eq!(applied.equity_curve[&date("2026-11-03")], expected_nav);
        assert!(!applied.cash.amount().is_negative());
    }

    #[test]
    fn availability_requires_observed_inputs_and_routes_late_effective_session_to_recovery() {
        let t = date("2026-11-03");
        let e = date("2026-11-04");
        let kst = FixedOffset::east_opt(9 * 3600).unwrap();
        let now = NaiveDate::from_ymd_opt(2026, 11, 3)
            .unwrap()
            .and_hms_opt(18, 0, 0)
            .unwrap()
            .and_local_timezone(kst)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let observation = PriceAvailabilityObservation {
            source_original_retrieved_at: now - chrono::Duration::minutes(2),
            input_admitted_at: now - chrono::Duration::minutes(1),
            usable_at: now - chrono::Duration::seconds(30),
        };
        assert_eq!(
            validate_price_availability(t, e, now, observation, PriceUse::ApplyT),
            Ok(AvailabilityDecision::Ready)
        );
        let late = NaiveDate::from_ymd_opt(2026, 11, 5)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
            .and_local_timezone(kst)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            validate_price_availability(e, e, late, observation, PriceUse::SettleE),
            Ok(AvailabilityDecision::RecoveryRequired)
        );
        let future = PriceAvailabilityObservation {
            usable_at: now + chrono::Duration::seconds(1),
            ..observation
        };
        assert_eq!(
            validate_price_availability(t, e, now, future, PriceUse::SignalT),
            Err(PaperCalculationError(
                PaperCalculationReasonCode::PriceUnavailable
            ))
        );
    }
}
