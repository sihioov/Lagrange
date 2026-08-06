//! Execution-delay scenarios (design §9.5 `ExecutionDelay`, plan Todo 21).
//!
//! [`delay_execution`] shifts every fill `delay_sessions` sessions later on
//! the deterministic session calendar (the sorted unique fill dates) and
//! rebuilds the whole result through the shared [`replay`]. Delay moves WHEN
//! the execution is experienced — fills, prices, and fees are untouched, so
//! the terminal value is preserved while the equity curve is re-dated
//! (execution-timing risk). A fill whose shifted session would fall beyond
//! the window settles at the last session; a delay at or beyond the calendar
//! horizon is a typed [`RobustnessError::DelayOutOfRange`] (the scenario is
//! nonsensical, never silently truncated).

use std::collections::HashMap;

use domain::UtcTimestamp;

use crate::backtest::BacktestResult;
use crate::robustness::RobustnessError;
use crate::robustness::replay::replay_with;

/// Replays `result` with every fill delayed by `delay_sessions` sessions.
pub fn delay_execution(
    result: &BacktestResult,
    delay_sessions: u32,
) -> Result<BacktestResult, RobustnessError> {
    let mut calendar: Vec<String> = Vec::new();
    for fill in &result.fills {
        let date = fill.ts.to_rfc3339()[..10].to_owned();
        if calendar.last() != Some(&date) {
            calendar.push(date);
        }
    }
    if calendar.is_empty() {
        return Ok(result.clone());
    }
    if delay_sessions as usize >= calendar.len() {
        return Err(RobustnessError::DelayOutOfRange {
            detail: format!(
                "delay of {delay_sessions} sessions on a {}-session calendar",
                calendar.len()
            ),
        });
    }
    let index_of: HashMap<&str, usize> = calendar
        .iter()
        .enumerate()
        .map(|(i, d)| (d.as_str(), i))
        .collect();
    let horizon = calendar.len() - 1;

    replay_with(
        result,
        |fill| {
            let mut shifted = fill.clone();
            let date = fill.ts.to_rfc3339()[..10].to_owned();
            let index = index_of[date.as_str()];
            let new_index = (index + delay_sessions as usize).min(horizon);
            let new_date = &calendar[new_index];
            shifted.ts = UtcTimestamp::parse_rfc3339(&format!("{new_date}T00:00:00Z"))
                .expect("calendar dates parse as RFC 3339");
            shifted
        },
        |fill| (fill.commission, fill.tax),
    )
}
