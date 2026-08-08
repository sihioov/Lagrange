//! Execution reports: partial, duplicate, and out-of-order (plan Todo 36).
//!
//! KIS delivers fills over a WebSocket that offers no ordering or
//! exactly-once guarantee. After a reconnect the adapter re-queries and
//! replays, so the same report arrives twice; under load a later sequence can
//! arrive before an earlier one. Design §17's injection list names both
//! ("중복 이벤트", "순서가 뒤바뀐 체결 통보") and §6.12 requires a full
//! re-query after reconnect.
//!
//! [`ExecutionTracker`] therefore accumulates fills as a SET keyed by the
//! broker's execution id rather than as a running total. That single choice is
//! what makes duplicates idempotent and ordering irrelevant: adding the same
//! execution twice cannot change the total, and adding executions in any order
//! yields the same state. A `filled += qty` accumulator would be wrong in both
//! cases, and wrong in a way that only shows up in production.

use std::collections::BTreeMap;

/// One execution (a fill or partial fill) as reported by the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    /// The broker's unique id for THIS execution. Deduplication depends on it.
    pub execution_id: String,
    /// The broker's order number this execution belongs to.
    pub broker_order_no: String,
    /// Quantity filled by this execution, as an exact decimal string.
    pub quantity: String,
    /// Execution price, as an exact decimal string.
    pub price: String,
    /// Broker sequence number when supplied. Recorded for audit; correctness
    /// does NOT depend on it, because it cannot be trusted to arrive in order.
    pub sequence: Option<u64>,
}

/// What changed when a report was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// A new execution was recorded.
    Recorded,
    /// Already known; state is unchanged. Not an error — it is the expected
    /// outcome of a reconnect replay.
    Duplicate,
}

/// Accumulated executions for one order.
#[derive(Debug, Clone, Default)]
pub struct ExecutionTracker {
    /// Keyed by execution id, so applying the same report twice is a no-op and
    /// order of arrival cannot matter. BTreeMap keeps iteration deterministic
    /// for audit rendering.
    executions: BTreeMap<String, ExecutionReport>,
}

impl ExecutionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a report. Idempotent by execution id.
    pub fn apply(&mut self, report: ExecutionReport) -> Applied {
        if self.executions.contains_key(&report.execution_id) {
            return Applied::Duplicate;
        }
        self.executions.insert(report.execution_id.clone(), report);
        Applied::Recorded
    }

    /// Total filled quantity in minor units (scale-4 fixed point, matching the
    /// ledger's decimal convention). Integer arithmetic on purpose: summing
    /// f64 fills would drift, and a drifting fill total silently corrupts the
    /// position it feeds.
    pub fn filled_quantity_scaled(&self) -> i128 {
        self.executions
            .values()
            .filter_map(|e| parse_scaled(&e.quantity))
            .sum()
    }

    /// Executions in a deterministic order, for audit and reconciliation.
    pub fn executions(&self) -> impl Iterator<Item = &ExecutionReport> {
        self.executions.values()
    }

    pub fn len(&self) -> usize {
        self.executions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.executions.is_empty()
    }

    /// Whether the order is fully filled against an ordered quantity.
    ///
    /// Compared with `>=` rather than `==`: an over-fill is a real broker
    /// condition and must read as complete-and-anomalous, never as "still
    /// working", which would leave the adapter waiting forever.
    pub fn is_complete(&self, ordered_quantity: &str) -> bool {
        match parse_scaled(ordered_quantity) {
            Some(target) => self.filled_quantity_scaled() >= target,
            None => false,
        }
    }
}

/// Scale used for quantities and prices, matching the ledger.
const SCALE: u32 = 4;

/// Parse a decimal string into scale-4 minor units without floating point.
fn parse_scaled(s: &str) -> Option<i128> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1i128, rest),
        None => (1i128, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int_part, frac_part) = match digits.split_once('.') {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // More precision than the ledger carries is a contract mismatch, not
    // something to round away silently.
    if frac_part.len() > SCALE as usize {
        return None;
    }
    let mut value: i128 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().ok()?
    };
    for _ in 0..SCALE {
        value = value.checked_mul(10)?;
    }
    let mut frac: i128 = 0;
    for (i, c) in frac_part.chars().enumerate() {
        let d = (c as u8 - b'0') as i128;
        let place = 10i128.checked_pow(SCALE - 1 - i as u32)?;
        frac += d * place;
    }
    Some(sign * (value + frac))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str, qty: &str, seq: Option<u64>) -> ExecutionReport {
        ExecutionReport {
            execution_id: id.to_string(),
            broker_order_no: "0000117057".to_string(),
            quantity: qty.to_string(),
            price: "40200".to_string(),
            sequence: seq,
        }
    }

    #[test]
    fn partial_fills_accumulate() {
        let mut t = ExecutionTracker::new();
        assert_eq!(t.apply(report("e1", "30", Some(1))), Applied::Recorded);
        assert_eq!(t.apply(report("e2", "70", Some(2))), Applied::Recorded);
        assert_eq!(t.filled_quantity_scaled(), 100 * 10_000);
        assert!(t.is_complete("100"));
    }

    #[test]
    fn a_duplicate_report_changes_nothing() {
        // The reconnect-replay case: the same execution is delivered again.
        let mut t = ExecutionTracker::new();
        t.apply(report("e1", "30", Some(1)));
        assert_eq!(t.apply(report("e1", "30", Some(1))), Applied::Duplicate);
        assert_eq!(t.len(), 1);
        assert_eq!(
            t.filled_quantity_scaled(),
            30 * 10_000,
            "a replayed fill must not double the position"
        );
    }

    #[test]
    fn out_of_order_reports_reach_the_same_state_as_in_order_ones() {
        // Ordering must be irrelevant, because the transport does not
        // guarantee it and a reconnect replays from an arbitrary point.
        let mut forward = ExecutionTracker::new();
        forward.apply(report("e1", "30", Some(1)));
        forward.apply(report("e2", "20", Some(2)));
        forward.apply(report("e3", "50", Some(3)));

        let mut reversed = ExecutionTracker::new();
        reversed.apply(report("e3", "50", Some(3)));
        reversed.apply(report("e1", "30", Some(1)));
        reversed.apply(report("e2", "20", Some(2)));

        assert_eq!(
            forward.filled_quantity_scaled(),
            reversed.filled_quantity_scaled()
        );
        assert_eq!(forward.len(), reversed.len());
        let a: Vec<_> = forward.executions().map(|e| &e.execution_id).collect();
        let b: Vec<_> = reversed.executions().map(|e| &e.execution_id).collect();
        assert_eq!(a, b, "iteration order must be deterministic for audit");
    }

    #[test]
    fn a_replay_that_interleaves_new_and_seen_reports_stays_correct() {
        // What a real reconnect looks like: re-query returns everything, some
        // of which is already known and some of which is new.
        let mut t = ExecutionTracker::new();
        t.apply(report("e1", "30", Some(1)));
        t.apply(report("e2", "20", Some(2)));

        for r in [
            report("e1", "30", Some(1)),
            report("e2", "20", Some(2)),
            report("e3", "50", Some(3)),
        ] {
            t.apply(r);
        }
        assert_eq!(t.len(), 3);
        assert_eq!(t.filled_quantity_scaled(), 100 * 10_000);
    }

    #[test]
    fn an_overfill_reads_as_complete_not_as_still_working() {
        let mut t = ExecutionTracker::new();
        t.apply(report("e1", "60", Some(1)));
        t.apply(report("e2", "60", Some(2)));
        assert!(
            t.is_complete("100"),
            "an over-fill must not leave the adapter waiting forever"
        );
    }

    #[test]
    fn fractional_quantities_do_not_drift() {
        // Integer minor units, not f64: 0.1 + 0.2 must be exactly 0.3.
        let mut t = ExecutionTracker::new();
        t.apply(report("e1", "0.1", None));
        t.apply(report("e2", "0.2", None));
        assert_eq!(t.filled_quantity_scaled(), 3_000);
        assert!(t.is_complete("0.3"));
    }

    #[test]
    fn a_malformed_quantity_is_ignored_rather_than_guessed() {
        assert_eq!(parse_scaled("abc"), None);
        assert_eq!(parse_scaled(""), None);
        // More precision than the ledger carries is a contract mismatch.
        assert_eq!(parse_scaled("1.234567"), None);
        assert_eq!(parse_scaled("100"), Some(1_000_000));
        assert_eq!(parse_scaled("-2.5"), Some(-25_000));
    }

    #[test]
    fn completeness_is_false_when_the_ordered_quantity_is_unparseable() {
        // Fail closed: an unreadable target must never read as filled.
        let mut t = ExecutionTracker::new();
        t.apply(report("e1", "100", None));
        assert!(!t.is_complete("not-a-number"));
    }
}
