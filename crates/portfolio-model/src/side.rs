//! The order side: buy or sell. Shorting is unsupported across backtest,
//! Paper, and Live, so a sell always reduces an existing position.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The direction of an order or fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Buy: debits cash by `notional + fees`.
    Buy,
    /// Sell: credits cash by `notional - fees` and requires a position.
    Sell,
}

impl Side {
    /// Whether this side buys.
    pub fn is_buy(self) -> bool {
        matches!(self, Side::Buy)
    }

    /// Whether this side sells.
    pub fn is_sell(self) -> bool {
        matches!(self, Side::Sell)
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        })
    }
}
