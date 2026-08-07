//! Paper account opening (plan Todo 30, design §10.1 `PaperAccount`).
//!
//! A Paper account is fully described by its initial cash and cost profile
//! — opening one is exactly constructing the canonical [`LedgerState`] from
//! those two inputs, the SAME ledger backtest/Paper/Live all share (design:
//! no mode-specific arithmetic). This module adds nothing to the ledger
//! itself; it only validates the inputs a Paper account creation route must
//! check before it ever reaches the database.

use domain::Money;

use crate::cost::CostProfile;
use crate::error::PortfolioError;
use crate::ledger::LedgerState;

/// A validated Paper account opening spec.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPaperAccount {
    pub initial_cash: Money,
    pub cost_profile: CostProfile,
}

impl NewPaperAccount {
    /// Validates a Paper account opening: initial cash must be positive,
    /// and the cost profile's own money fields must share its currency (a
    /// KRW account cannot run a cost profile denominated in another
    /// currency).
    pub fn new(initial_cash: Money, cost_profile: CostProfile) -> Result<Self, PortfolioError> {
        if !initial_cash.amount().is_positive() {
            return Err(PortfolioError::NonPositiveInitialCash {
                amount: initial_cash,
            });
        }
        let base_currency = initial_cash.currency();
        if cost_profile.min_commission.currency() != base_currency {
            return Err(PortfolioError::Domain(
                domain::DomainError::CurrencyMismatch {
                    left: base_currency,
                    right: cost_profile.min_commission.currency(),
                },
            ));
        }
        if cost_profile.min_trade.currency() != base_currency {
            return Err(PortfolioError::Domain(
                domain::DomainError::CurrencyMismatch {
                    left: base_currency,
                    right: cost_profile.min_trade.currency(),
                },
            ));
        }
        Ok(Self {
            initial_cash,
            cost_profile,
        })
    }

    /// The account's opening ledger state: cash equals the initial deposit,
    /// no positions, no orders.
    pub fn opening_state(&self) -> LedgerState {
        LedgerState::new(self.initial_cash, self.cost_profile.clone())
    }
}
