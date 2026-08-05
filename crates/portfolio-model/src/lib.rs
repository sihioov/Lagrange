//! `portfolio-model` - Lagrange Station shared sizing, costs, and replayable
//! ledger invariants.
//!
//! One implementation serves backtest, Paper, and Live (design §8.3, §9.3-9.4,
//! §10): there is NO mode-specific arithmetic anywhere in this crate.
//!
//! - [`sizing`]: target-to-order sizing (integer lots, KRW fixed point,
//!   sell-before-buy, available-cash + cost reservation, minimum trade,
//!   rebalance threshold).
//! - [`cost`]: versioned `KRX_ETF_DEFAULT | CUSTOM` commission/tax/slippage
//!   profiles with the documented `CostModel` breakdown interface.
//! - [`ledger`]: the canonical order/fill/cash/position/daily-equity
//!   transitions and deterministic replay (implemented in its own todo).
//! - [`persistence`]: the DB-manifest seam (Todo 3); the ledger core is
//!   deliberately DB-free.

pub mod cost;
pub mod error;
pub mod side;
pub mod sizing;

pub use cost::{CostBreakdown, CostProfile, CostProfileId};
pub use error::PortfolioError;
pub use side::Side;
pub use sizing::{
    OrderRequest, SizingAction, SizingDecision, SizingInput, SizingReport, SkipReason,
    TargetAllocation, allocation_from_target_portfolio, plan_rebalance, weight_from_ratio,
};
