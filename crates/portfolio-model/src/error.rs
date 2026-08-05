//! Typed errors for the shared portfolio model.
//!
//! Every failure mode is a typed [`PortfolioError`]: the ledger and sizer
//! never panic and never fail silently. Monetary/quantity payloads are the
//! domain branded types (their `Display` renders exact decimal strings).

use domain::{DomainError, FillId, FixedPoint, InstrumentId, Money, OrderId, Quantity};

use crate::side::Side;

/// The single typed error for sizing, costs, and ledger transitions.
#[derive(Debug, thiserror::Error)]
pub enum PortfolioError {
    /// Any domain contract violation surfaced by the fixed-point types.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// A buy (or any debit) would drive cash negative.
    #[error("insufficient cash: need {needed}, available {available}")]
    InsufficientCash { needed: Money, available: Money },

    /// The same fill id applied twice (idempotency of a fill event).
    #[error("duplicate fill {fill_id}")]
    DuplicateFill { fill_id: FillId },

    /// A fill referencing an order that was never placed.
    #[error("fill references unknown order {order_id}")]
    UnknownOrder { order_id: OrderId },

    /// The same order id placed twice.
    #[error("duplicate order {order_id}")]
    DuplicateOrder { order_id: OrderId },

    /// A fill exceeds the remaining quantity of its order.
    #[error("fill of {fill_quantity} exceeds remaining {remaining} on order {order_id}")]
    OverFill {
        order_id: OrderId,
        remaining: Quantity,
        fill_quantity: Quantity,
    },

    /// A fill whose side contradicts its order.
    #[error("fill side {fill_side} does not match order side {order_side} on order {order_id}")]
    SideMismatch {
        order_id: OrderId,
        order_side: Side,
        fill_side: Side,
    },

    /// A fill whose instrument contradicts its order.
    #[error(
        "fill instrument {fill_instrument} does not match order instrument {order_instrument} on order {order_id}"
    )]
    InstrumentMismatch {
        order_id: OrderId,
        order_instrument: InstrumentId,
        fill_instrument: InstrumentId,
    },

    /// A zero-quantity order or fill (shorting/empty orders are unsupported).
    #[error("zero {kind} for {id}")]
    ZeroQuantity { kind: &'static str, id: String },

    /// A sell that would drive a position negative (no shorting).
    #[error("sell of {fill_quantity} exceeds position {position} for {instrument_id}")]
    SellWithoutPosition {
        instrument_id: InstrumentId,
        fill_quantity: Quantity,
        position: Quantity,
    },

    /// A sell whose explicit fees exceed its proceeds (loss-making sale).
    #[error("sell fees {fees} exceed proceeds {notional}")]
    FeesExceedProceeds { notional: Money, fees: Money },

    /// Events must arrive with strictly increasing sequence numbers.
    #[error("event out of order: seq {seq} <= last seq {last_seq}")]
    OutOfOrderEvent { seq: u64, last_seq: u64 },

    /// Daily marking attempted without a price for a held position.
    #[error("missing mark for position {instrument_id}")]
    MissingMark { instrument_id: InstrumentId },

    /// Sizing attempted without an open price for an instrument.
    #[error("missing open price for {instrument_id}")]
    MissingPrice { instrument_id: InstrumentId },

    /// Sizing with zero equity while targets exist (division by zero guard).
    #[error("zero equity with non-zero targets")]
    ZeroEquity,

    /// A value whose precision cannot be represented at the canonical scale.
    #[error("price precision {value} exceeds supported scale {max_scale}")]
    PrecisionExceeded { value: FixedPoint, max_scale: u8 },

    /// Slippage would drive the execution price to zero or negative.
    #[error("execution price non-positive for raw {raw} with slippage {slippage_bps} bps")]
    NonPositiveExecutionPrice { raw: FixedPoint, slippage_bps: u64 },

    /// An f64 weight conversion received a non-finite value.
    #[error("non-finite weight {value}")]
    NonFiniteWeight { value: f64 },

    /// An f64 weight conversion outside [0, 1].
    #[error("weight {value} out of range [{min}, {max}]")]
    WeightOutOfRange { value: f64, min: f64, max: f64 },

    /// A rate (commission/tax/threshold) that is negative.
    #[error("negative rate for {field}: {value}")]
    NegativeRate { field: &'static str, value: String },

    /// A rate above the supported maximum.
    #[error("rate for {field}: {value} exceeds supported maximum 1")]
    RateOutOfRange { field: &'static str, value: String },

    /// Slippage beyond 100% (10,000 bps).
    #[error("slippage {bps} bps out of range [0, 10000]")]
    SlippageOutOfRange { bps: u64 },

    /// A zero lot size.
    #[error("invalid lot size {lot_size}")]
    InvalidLotSize { lot_size: u64 },

    /// An internal sizing invariant was violated (never user-reachable).
    #[error("internal sizing invariant violated: {detail}")]
    SizingInternal { detail: String },

    /// Canonical serialization failed (never user-reachable).
    #[error("serialization failed: {detail}")]
    Serialization { detail: String },
}
