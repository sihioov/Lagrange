//! Pure D5 supplier proposal. No request, timer, transport, budget reservation or
//! operational approval is created here. The collectors runtime separately
//! binds protected configuration, source evidence and durable reservations.

use chrono::{FixedOffset, Timelike};
use domain::{ContentHash, TradingDate, UtcTimestamp};
use uuid::Uuid;

use super::calendar::ExpectedCalendarSession;

const MAX_GAP_DAYS: i64 = 14;
const LOGICAL_GROUPS: u16 = 7;
const MAX_PAGES_PER_GROUP: u16 = 10;
const MAX_ATTEMPTS_PER_PAGE: u16 = 4;
const MAX_WIRE_GETS: u16 = LOGICAL_GROUPS * MAX_PAGES_PER_GROUP * MAX_ATTEMPTS_PER_PAGE;

/// A source proposal, deliberately not an installed/approved runtime capability.
#[derive(Default)]
pub enum SupplyProposalConfig {
    #[default]
    Disabled,
    Proposed {
        owner: Uuid,
        policy_sha256: ContentHash,
        activation_date: TradingDate,
    },
}

/// Caller claims for pure planning only. The runtime supplier must recover durable
/// reservation/consumption receipts; calling this function cannot reset that state.
pub struct SupplyDayStateClaim {
    pub kst_date: TradingDate,
    pub range_attempt_reserved: bool,
    pub wire_gets_consumed: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplyPlanError {
    InvalidContext,
    CalendarMismatch,
    CalendarCoverageGap,
    FrontierAhead,
    GapLimit,
    UnreconciledBudget,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplyNoop {
    Disabled,
    BeforeActivation,
    BeforeCutoff,
    AlreadyCovered,
    AlreadyAttempted,
}

pub enum ActionSupplyProposal {
    Noop(SupplyNoop),
    Range(ProposedActionRange),
}

/// A plan to review/reserve, never permission to invoke a provider. It includes
/// closed civil days and supports only the WholeMarket7 source proposal.
pub struct ProposedActionRange {
    start: TradingDate,
    end: TradingDate,
    kst_day: TradingDate,
}
impl ProposedActionRange {
    pub fn start(&self) -> TradingDate {
        self.start
    }
    pub fn end(&self) -> TradingDate {
        self.end
    }
    pub fn kst_day(&self) -> TradingDate {
        self.kst_day
    }
    pub fn scope(&self) -> &'static str {
        "whole-market"
    }
    pub fn logical_groups(&self) -> u16 {
        LOGICAL_GROUPS
    }
    pub fn max_pages_per_group(&self) -> u16 {
        MAX_PAGES_PER_GROUP
    }
    pub fn max_attempts_per_page(&self) -> u16 {
        MAX_ATTEMPTS_PER_PAGE
    }
    pub fn maximum_wire_gets(&self) -> u16 {
        MAX_WIRE_GETS
    }
}

/// The calendar comes from independent Raw projection, not the latest admitted
/// artifact. covered_through_claim is only a claim in this pure source planner.
/// Runtime must derive it from complete committed range evidence, not job success.
pub fn plan_action_supply_proposal(
    config: &SupplyProposalConfig,
    expected: Option<&ExpectedCalendarSession>,
    covered_through_claim: TradingDate,
    day_state: &SupplyDayStateClaim,
    now: UtcTimestamp,
) -> Result<ActionSupplyProposal, SupplyPlanError> {
    let SupplyProposalConfig::Proposed {
        owner,
        policy_sha256,
        activation_date,
    } = config
    else {
        return Ok(ActionSupplyProposal::Noop(SupplyNoop::Disabled));
    };
    if owner.is_nil() {
        return Err(SupplyPlanError::InvalidContext);
    }
    let (today, after_cutoff) = clock_context(now)?;
    if today < *activation_date {
        return Ok(ActionSupplyProposal::Noop(SupplyNoop::BeforeActivation));
    }
    if !after_cutoff {
        return Ok(ActionSupplyProposal::Noop(SupplyNoop::BeforeCutoff));
    }
    let expected = expected.ok_or(SupplyPlanError::CalendarCoverageGap)?;
    if expected.proof().owner() != *owner || expected.proof().policy_sha256() != policy_sha256 {
        return Err(SupplyPlanError::InvalidContext);
    }
    if expected.eligible_date() != today {
        return Err(SupplyPlanError::CalendarMismatch);
    }
    let first = expected
        .proof()
        .days()
        .first()
        .ok_or(SupplyPlanError::CalendarCoverageGap)?
        .date();
    plan_range(
        covered_through_claim,
        expected.session(),
        first,
        today,
        day_state,
    )
}

fn clock_context(now: UtcTimestamp) -> Result<(TradingDate, bool), SupplyPlanError> {
    let offset = FixedOffset::east_opt(9 * 3600).ok_or(SupplyPlanError::InvalidContext)?;
    let kst = now.as_datetime().with_timezone(&offset);
    let today = TradingDate::parse(&kst.date_naive().to_string())
        .map_err(|_| SupplyPlanError::InvalidContext)?;
    Ok((today, (kst.hour(), kst.minute()) >= (16, 30)))
}

fn plan_range(
    frontier: TradingDate,
    target: TradingDate,
    calendar_start: TradingDate,
    today: TradingDate,
    state: &SupplyDayStateClaim,
) -> Result<ActionSupplyProposal, SupplyPlanError> {
    if frontier < TradingDate::parse("2026-08-28").map_err(|_| SupplyPlanError::InvalidContext)?
        || target > today
    {
        return Err(SupplyPlanError::InvalidContext);
    }
    if frontier > target {
        return Err(SupplyPlanError::FrontierAhead);
    }
    let start = frontier
        .checked_add_days(1)
        .map_err(|_| SupplyPlanError::InvalidContext)?;
    if calendar_start > start {
        return Err(SupplyPlanError::CalendarCoverageGap);
    }
    if state.kst_date != today
        || state.wire_gets_consumed > MAX_WIRE_GETS
        || (!state.range_attempt_reserved && state.wire_gets_consumed != 0)
    {
        return Err(SupplyPlanError::UnreconciledBudget);
    }
    if frontier == target {
        return Ok(ActionSupplyProposal::Noop(SupplyNoop::AlreadyCovered));
    }
    if state.range_attempt_reserved {
        return Ok(ActionSupplyProposal::Noop(SupplyNoop::AlreadyAttempted));
    }
    let span = target
        .as_naive_date()
        .signed_duration_since(start.as_naive_date())
        .num_days()
        + 1;
    if !(1..=MAX_GAP_DAYS).contains(&span) {
        return Err(SupplyPlanError::GapLimit);
    }
    Ok(ActionSupplyProposal::Range(ProposedActionRange {
        start,
        end: target,
        kst_day: today,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn state(day: &str) -> SupplyDayStateClaim {
        SupplyDayStateClaim {
            kst_date: date(day),
            range_attempt_reserved: false,
            wire_gets_consumed: 0,
        }
    }
    fn outcome(
        result: Result<ActionSupplyProposal, SupplyPlanError>,
    ) -> Result<Option<SupplyNoop>, SupplyPlanError> {
        result.map(|value| match value {
            ActionSupplyProposal::Noop(reason) => Some(reason),
            ActionSupplyProposal::Range(_) => None,
        })
    }
    #[test]
    fn s5f_supplier_proposal_defaults_disabled_without_transport() {
        assert!(matches!(
            SupplyProposalConfig::default(),
            SupplyProposalConfig::Disabled
        ));
        assert_eq!(MAX_WIRE_GETS, 280);
    }
    #[test]
    fn s5f_clock_uses_exact_kst_cutoff_and_day_boundary() {
        for (instant, day, ready) in [
            ("2026-10-15T07:29:59Z", "2026-10-15", false),
            ("2026-10-15T07:30:00Z", "2026-10-15", true),
            ("2026-10-15T14:59:59Z", "2026-10-15", true),
            ("2026-10-15T15:00:00Z", "2026-10-16", false),
        ] {
            assert_eq!(
                clock_context(UtcTimestamp::parse_rfc3339(instant).unwrap()),
                Ok((date(day), ready))
            );
        }
    }
    #[test]
    fn s5f_friday_to_monday_plan_includes_weekend_and_whole_market_budget() {
        let plan = plan_range(
            date("2026-10-09"),
            date("2026-10-12"),
            date("2026-10-09"),
            date("2026-10-12"),
            &state("2026-10-12"),
        )
        .unwrap();
        let ActionSupplyProposal::Range(plan) = plan else {
            panic!("range required")
        };
        assert_eq!(plan.start(), date("2026-10-10"));
        assert_eq!(plan.end(), date("2026-10-12"));
        assert_eq!(plan.kst_day(), date("2026-10-12"));
        assert_eq!(plan.scope(), "whole-market");
        assert_eq!(plan.logical_groups(), 7);
        assert_eq!(plan.max_pages_per_group(), 10);
        assert_eq!(plan.max_attempts_per_page(), 4);
        assert_eq!(plan.maximum_wire_gets(), 280);
    }
    #[test]
    fn s5f_closed_day_with_same_latest_session_needs_no_range() {
        assert_eq!(
            outcome(plan_range(
                date("2026-10-09"),
                date("2026-10-09"),
                date("2026-10-09"),
                date("2026-10-12"),
                &state("2026-10-12")
            )),
            Ok(Some(SupplyNoop::AlreadyCovered))
        );
    }
    #[test]
    fn s5f_fourteen_day_limit_is_inclusive_and_never_chunks_backlog() {
        assert_eq!(
            outcome(plan_range(
                date("2026-09-30"),
                date("2026-10-14"),
                date("2026-10-01"),
                date("2026-10-15"),
                &state("2026-10-15")
            )),
            Ok(None)
        );
        assert_eq!(
            outcome(plan_range(
                date("2026-09-30"),
                date("2026-10-15"),
                date("2026-10-01"),
                date("2026-10-15"),
                &state("2026-10-15")
            )),
            Err(SupplyPlanError::GapLimit)
        );
    }
    #[test]
    fn s5f_reserved_attempt_never_gets_a_second_budget_after_restart() {
        for consumed in [0, 1, 279, 280] {
            let state = SupplyDayStateClaim {
                kst_date: date("2026-10-12"),
                range_attempt_reserved: true,
                wire_gets_consumed: consumed,
            };
            assert_eq!(
                outcome(plan_range(
                    date("2026-10-09"),
                    date("2026-10-12"),
                    date("2026-10-09"),
                    date("2026-10-12"),
                    &state
                )),
                Ok(Some(SupplyNoop::AlreadyAttempted))
            );
        }
    }
    #[test]
    fn s5f_stale_or_inconsistent_budget_is_not_implicitly_reset() {
        for claim in [
            state("2026-10-11"),
            SupplyDayStateClaim {
                kst_date: date("2026-10-12"),
                range_attempt_reserved: false,
                wire_gets_consumed: 1,
            },
            SupplyDayStateClaim {
                kst_date: date("2026-10-12"),
                range_attempt_reserved: true,
                wire_gets_consumed: 281,
            },
        ] {
            assert_eq!(
                outcome(plan_range(
                    date("2026-10-09"),
                    date("2026-10-12"),
                    date("2026-10-09"),
                    date("2026-10-12"),
                    &claim
                )),
                Err(SupplyPlanError::UnreconciledBudget)
            );
        }
    }
    #[test]
    fn s5f_frontier_or_calendar_gaps_cannot_create_a_range() {
        let state = state("2026-10-12");
        assert_eq!(
            outcome(plan_range(
                date("2026-10-13"),
                date("2026-10-12"),
                date("2026-10-09"),
                date("2026-10-12"),
                &state
            )),
            Err(SupplyPlanError::FrontierAhead)
        );
        assert_eq!(
            outcome(plan_range(
                date("2026-10-09"),
                date("2026-10-13"),
                date("2026-10-09"),
                date("2026-10-12"),
                &state
            )),
            Err(SupplyPlanError::InvalidContext)
        );
        assert_eq!(
            outcome(plan_range(
                date("2026-10-09"),
                date("2026-10-12"),
                date("2026-10-11"),
                date("2026-10-12"),
                &state
            )),
            Err(SupplyPlanError::CalendarCoverageGap)
        );
    }
    #[test]
    fn s5f_disabled_or_pre_activation_needs_no_raw_calendar_or_budget_reset() {
        let now = UtcTimestamp::parse_rfc3339("2026-10-15T07:30:00Z").unwrap();
        assert_eq!(
            outcome(plan_action_supply_proposal(
                &SupplyProposalConfig::default(),
                None,
                date("2026-08-28"),
                &state("2026-10-14"),
                now
            )),
            Ok(Some(SupplyNoop::Disabled))
        );
        let config = SupplyProposalConfig::Proposed {
            owner: Uuid::from_u128(99),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            activation_date: date("2026-10-16"),
        };
        assert_eq!(
            outcome(plan_action_supply_proposal(
                &config,
                None,
                date("2026-08-28"),
                &state("2026-10-14"),
                now
            )),
            Ok(Some(SupplyNoop::BeforeActivation))
        );
    }
    #[test]
    fn s5f_before_cutoff_is_inert_but_eligible_run_requires_calendar_proof() {
        let config = SupplyProposalConfig::Proposed {
            owner: Uuid::from_u128(99),
            policy_sha256: ContentHash::from_bytes(b"policy"),
            activation_date: date("2026-10-15"),
        };
        let before = UtcTimestamp::parse_rfc3339("2026-10-15T07:29:59Z").unwrap();
        let at = UtcTimestamp::parse_rfc3339("2026-10-15T07:30:00Z").unwrap();
        assert_eq!(
            outcome(plan_action_supply_proposal(
                &config,
                None,
                date("2026-10-14"),
                &state("2026-10-15"),
                before
            )),
            Ok(Some(SupplyNoop::BeforeCutoff))
        );
        assert_eq!(
            outcome(plan_action_supply_proposal(
                &config,
                None,
                date("2026-10-14"),
                &state("2026-10-15"),
                at
            )),
            Err(SupplyPlanError::CalendarCoverageGap)
        );
    }
}
