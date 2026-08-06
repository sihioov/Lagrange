//! Train/validation/test holdout (FR-ROB-001, plan Todo 21).
//!
//! The final test period exists so the selected parameters can be evaluated
//! honestly ONCE at the end. Everything in the selection pipeline — the
//! [`HoldoutBarrier`] and [`select_equity_series`] — refuses to touch dates
//! past `validation_end`: a buggy selector that feeds the full series gets a
//! typed [`RobustnessError::HoldoutViolation`] naming the first test date
//! instead of a silent filter. The test segment is only reachable through
//! the explicit [`SplitResult::test`] escape hatch.

use crate::robustness::RobustnessError;

/// Train/validation/test boundaries (`YYYY-MM-DD`, inclusive on both ends of
/// train and validation).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeriodSplit {
    /// Last date of the training segment (inclusive).
    pub train_end: String,
    /// Last date of the validation segment (inclusive); everything strictly
    /// after is the final test period.
    pub validation_end: String,
}

impl PeriodSplit {
    /// `train_end` must not fall after `validation_end` (ISO dates compare
    /// lexicographically).
    pub fn validate(&self) -> Result<(), RobustnessError> {
        if self.train_end > self.validation_end {
            return Err(RobustnessError::InvalidSplit {
                detail: format!(
                    "train_end {} is after validation_end {}",
                    self.train_end, self.validation_end
                ),
            });
        }
        Ok(())
    }
}

/// The selection-time barrier: any date past `validation_end` is the final
/// test period and must never be read during parameter selection.
#[derive(Debug, Clone)]
pub struct HoldoutBarrier {
    validation_end: String,
}

impl HoldoutBarrier {
    /// A barrier over the split's validation end.
    pub fn new(split: &PeriodSplit) -> Self {
        Self {
            validation_end: split.validation_end.clone(),
        }
    }

    /// `Ok(())` for train/validation dates; [`RobustnessError::HoldoutViolation`]
    /// for any date strictly past the validation end.
    pub fn guard(&self, date: &str) -> Result<(), RobustnessError> {
        if date > self.validation_end.as_str() {
            return Err(RobustnessError::HoldoutViolation {
                date: date.to_owned(),
            });
        }
        Ok(())
    }
}

/// Selects the train+validation portion of a `(date, value)` series for
/// parameter selection.
///
/// Every input point is guarded: if ANY point lies in the final test period
/// the whole selection fails with [`RobustnessError::HoldoutViolation`]
/// naming the first offending date (FR-ROB-001; a buggy selector is caught,
/// never silently filtered).
pub fn select_equity_series(
    series: &[(String, i64)],
    split: &PeriodSplit,
) -> Result<Vec<(String, i64)>, RobustnessError> {
    split.validate()?;
    let barrier = HoldoutBarrier::new(split);
    let mut selected = Vec::new();
    for (date, value) in series {
        barrier.guard(date)?;
        if date.as_str() <= split.validation_end.as_str() {
            selected.push((date.clone(), *value));
        }
    }
    Ok(selected)
}

/// The three immutable segments of a series under a [`PeriodSplit`].
#[derive(Debug, Clone)]
pub struct SplitResult {
    train: Vec<(String, i64)>,
    validation: Vec<(String, i64)>,
    test: Vec<(String, i64)>,
}

impl SplitResult {
    /// Partitions the series into train/validation/test.
    pub fn new(series: &[(String, i64)], split: &PeriodSplit) -> Result<Self, RobustnessError> {
        split.validate()?;
        let mut train = Vec::new();
        let mut validation = Vec::new();
        let mut test = Vec::new();
        for (date, value) in series {
            if date.as_str() <= split.train_end.as_str() {
                train.push((date.clone(), *value));
            } else if date.as_str() <= split.validation_end.as_str() {
                validation.push((date.clone(), *value));
            } else {
                test.push((date.clone(), *value));
            }
        }
        Ok(Self {
            train,
            validation,
            test,
        })
    }

    /// Training-segment points (used by selection).
    pub fn train(&self) -> &[(String, i64)] {
        &self.train
    }

    /// Validation-segment points (used by selection).
    pub fn validation(&self) -> &[(String, i64)] {
        &self.validation
    }

    /// The final test segment. This is the explicit escape hatch: reachable
    /// only when the final, one-shot evaluation happens — never during
    /// parameter selection (FR-ROB-001).
    pub fn test(&self) -> &[(String, i64)] {
        &self.test
    }
}
