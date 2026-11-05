//! Untrusted SQL readback and private-construction sealed Daily input.

use chrono::NaiveDate;
use market_data::daily_price_only::{
    DailyPricePinsV1Wire,
    replay::{DailyReplayError, ReopenedDailyChain},
};
use serde_json::Value;

use super::{contract::PAPER_SESSION_LIMIT, source_evidence as wire};

/// A closed-schema DB response decoded from bytes. It is evidence to compare,
/// never an admission or success capability by itself.
#[derive(Clone, Debug, PartialEq)]
pub struct UntrustedPaperSessionReadbackV1 {
    value: Value,
}

impl UntrustedPaperSessionReadbackV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, PaperInputError> {
        if bytes.len() > PAPER_SESSION_LIMIT {
            return Err(PaperInputError::LimitExceeded);
        }
        let value = wire::decode_canonical(bytes, PAPER_SESSION_LIMIT)
            .map_err(|_| PaperInputError::InvalidWire)?;
        Self::from_value(value)
    }

    pub(crate) fn from_value(value: Value) -> Result<Self, PaperInputError> {
        let bytes = wire::canonical(&value).map_err(|_| PaperInputError::InvalidWire)?;
        if bytes.len() > PAPER_SESSION_LIMIT {
            return Err(PaperInputError::LimitExceeded);
        }
        wire::decode_canonical(&bytes, PAPER_SESSION_LIMIT)
            .map_err(|_| PaperInputError::InvalidWire)?;
        wire::validate_wire(&value).map_err(|_| PaperInputError::InvalidWire)?;
        if value["schema"] != "owner-beta-paper-db-v1" {
            return Err(PaperInputError::WrongSchema);
        }
        Ok(Self { value })
    }

    pub fn as_untrusted_value(&self) -> &Value {
        &self.value
    }
}

/// A retained admission opened through the Daily replay API. The only
/// constructor is crate-private and requires a reopened sealed chain.
pub struct VerifiedPaperInputV1<'base> {
    session: NaiveDate,
    pins: DailyPricePinsV1Wire,
    chain: ReopenedDailyChain<'base>,
}

impl<'base> VerifiedPaperInputV1<'base> {
    #[expect(
        dead_code,
        reason = "The existing Legacy constructor is preserved; checked callers retain their actual chain themselves."
    )]
    pub(crate) fn from_sealed_chain(
        session: NaiveDate,
        pins: DailyPricePinsV1Wire,
        chain: ReopenedDailyChain<'base>,
    ) -> Result<Self, PaperInputError> {
        let admitted = chain.admitted().map_err(PaperInputError::DailyReplay)?;
        validate_initial_input(
            admitted.as_of().as_naive_date(),
            admitted.pins(),
            session,
            &pins,
            PaperInputError::PinMismatch,
            || {
                admitted
                    .revalidate()
                    .map_err(|_| PaperInputError::DailyReplay(DailyReplayError::InvalidChain))
            },
        )?;
        Ok(Self {
            session,
            pins,
            chain,
        })
    }

    pub fn session(&self) -> NaiveDate {
        self.session
    }

    pub fn pins(&self) -> &DailyPricePinsV1Wire {
        &self.pins
    }

    /// Revalidate retained descriptors before using this input in another
    /// read-only check. It carries no Paper action or financial-value access.
    pub fn revalidate(&self) -> Result<(), PaperInputError> {
        let admitted = self
            .chain
            .admitted()
            .map_err(PaperInputError::DailyReplay)?;
        admitted
            .revalidate()
            .map_err(|_| PaperInputError::DailyReplay(DailyReplayError::InvalidChain))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperInputError {
    #[error("PAPER_INPUT_LIMIT_EXCEEDED")]
    LimitExceeded,
    #[error("PAPER_INPUT_INVALID_WIRE")]
    InvalidWire,
    #[error("PAPER_INPUT_WRONG_SCHEMA")]
    WrongSchema,
    #[error("PAPER_INPUT_PIN_MISMATCH")]
    PinMismatch,
    #[error("PAPER_INPUT_DAILY_REPLAY_REJECTED")]
    DailyReplay(DailyReplayError),
}

// One original initial policy; the caller already owns the genuine chain.
fn validate_initial_input<P: PartialEq, E>(
    as_of: NaiveDate,
    actual: &P,
    session: NaiveDate,
    pins: &P,
    mismatch: E,
    revalidate: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    if as_of != session || actual != pins {
        return Err(mismatch);
    }
    revalidate()
}

#[cfg(target_os = "linux")]
pub(crate) enum CheckedPaperInputFailure {
    Policy(PaperInputError),
    Source(market_data::daily_price_only::admission::DailyPolicySource),
}
#[cfg(target_os = "linux")]
pub(crate) fn validate_checked_input_in(
    chain: &market_data::daily_price_only::replay::CheckedReopenedDailyChain<'_, '_, '_, '_>,
    session: NaiveDate,
    pins: &DailyPricePinsV1Wire,
    expected: &market_data::checked_close::Attempt,
    captured: &market_data::checked_close::Attempt,
) -> Result<(), CheckedPaperInputFailure> {
    let admitted = chain
        .admitted(expected, captured)
        .map_err(CheckedPaperInputFailure::Source)?;
    let view = admitted
        .view_in(expected, captured)
        .map_err(CheckedPaperInputFailure::Source)?;
    validate_initial_input(
        view.manifest().range_end.as_naive_date(),
        view.pins(),
        session,
        pins,
        CheckedPaperInputFailure::Policy(PaperInputError::PinMismatch),
        || {
            admitted
                .revalidate_in(expected, captured)
                .map_err(CheckedPaperInputFailure::Source)
        },
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn revalidate_checked_input_in(
    chain: &market_data::daily_price_only::replay::CheckedReopenedDailyChain<'_, '_, '_, '_>,
    expected: &market_data::checked_close::Attempt,
    captured: &market_data::checked_close::Attempt,
) -> Result<(), CheckedPaperInputFailure> {
    let admitted = chain
        .admitted(expected, captured)
        .map_err(CheckedPaperInputFailure::Source)?;
    admitted
        .revalidate_in(expected, captured)
        .map_err(CheckedPaperInputFailure::Source)
}

#[cfg(target_os = "linux")]
impl CheckedPaperInputFailure {
    pub(crate) fn source_report(&self) -> Option<&market_data::checked_close::Report> {
        match self {
            Self::Policy(_) => None,
            Self::Source(source) => source.report(),
        }
    }
}
#[cfg(target_os = "linux")]
impl std::fmt::Debug for CheckedPaperInputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Policy(policy) => f
                .debug_tuple("CheckedPaperInputPolicy")
                .field(policy)
                .finish(),
            Self::Source(source) => {
                let _ = source;
                f.write_str("CheckedPaperInputSource(..)")
            }
        }
    }
}

#[cfg(test)]
mod checked_input_tests {
    use super::*;
    #[test]
    fn checked_paper_input_retains_actual_chain_on_pin_or_revalidation_failure() {
        // Ordinary nonClone holder; positive opaque native chains are never manufactured.
        struct Holder {
            pins: String,
        }
        let holder = Holder {
            pins: "actual".into(),
        };
        let date = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = validate_initial_input(date, &holder.pins, date, &"other".into(), 1, || {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert_eq!(result, Err(1));
        assert_eq!(calls.get(), 0);
        let result = validate_initial_input(date, &holder.pins, date, &holder.pins, 1, || {
            calls.set(calls.get() + 1);
            Err(2)
        });
        assert_eq!(result, Err(2));
        assert_eq!(calls.get(), 1);
        assert_eq!(holder.pins, "actual");
    }
}
