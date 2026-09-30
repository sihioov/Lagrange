//! Provider-neutral value types for the KIS market WebSocket publication path.
//!
//! This module is intentionally separate from [`crate::intraday_quotes`].  A
//! market-stream observation does not contain the REST previous-day base
//! price, and its halt flag is the value verified on the wire rather than a
//! status inferred from a REST response.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use kis_client::market_stream_wire::{BasePriceReason, Direction, MarketReceipt, WIRE_VERSION};
use thiserror::Error;

pub const STREAM_SOURCE: &str = "KIS_MARKET_WS";
pub const STREAM_WIRE_VERSION: &str = WIRE_VERSION;
pub const STREAM_VENUE: &str = "KRX";
pub const STREAM_CURRENCY: &str = "KRW";
pub const STREAM_TIMEZONE: &str = "Asia/Seoul";

/// Direction copied into the stream DTO without coupling it to the REST
/// `IntradayQuoteDirection` type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamQuoteDirection {
    Up,
    Down,
    Flat,
    LimitUp,
    LimitDown,
}

impl StreamQuoteDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Flat => "FLAT",
            Self::LimitUp => "LIMIT_UP",
            Self::LimitDown => "LIMIT_DOWN",
        }
    }
}

impl From<Direction> for StreamQuoteDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
            Direction::Flat => Self::Flat,
            Direction::LimitUp => Self::LimitUp,
            Direction::LimitDown => Self::LimitDown,
        }
    }
}

/// The reason a stream quote has no REST-style base price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamBasePriceReason {
    NotProvidedByChannel,
}

impl StreamBasePriceReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotProvidedByChannel => "NOT_PROVIDED_BY_CHANNEL",
        }
    }
}

/// Errors raised while converting a transport receipt into the storage DTO.
/// None of these variants carries a provider payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StreamQuoteError {
    #[error("market stream receipt is invalid")]
    InvalidReceipt,
    #[error("market stream observation date is invalid")]
    InvalidBusinessDate,
    #[error("market stream observation time is invalid")]
    InvalidTradeTime,
    #[error("market stream observation is not a regular KRX trade")]
    NonRegularObservation,
    #[error("market stream channel supplied a REST base price")]
    BasePricePresent,
}

/// A validated KRX market-stream observation suitable for latest-value
/// publication.  This type cannot be constructed from a REST quote: the
/// production conversion accepts only the private-provenance `MarketReceipt`
/// created by the WS-2 transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamQuote {
    pub symbol: String,
    pub business_date: NaiveDate,
    pub trade_time: NaiveTime,
    pub provider_trade_at: DateTime<FixedOffset>,
    pub price: String,
    pub change_from_previous_day: String,
    pub change_percent_from_previous_day: String,
    pub direction: StreamQuoteDirection,
    pub trade_volume: u64,
    pub cumulative_volume: u64,
    /// Always `None`: H0STCNT0 does not provide `STCK_SDPR`.
    pub base_price: Option<String>,
    pub base_price_reason: StreamBasePriceReason,
    /// The verified `TRHT_YN` value from the market channel.
    pub halted: bool,
    pub opening_class: String,
    pub hour_class: String,
    pub market_class: String,
    pub transaction_class: String,
}

impl StreamQuote {
    /// Convert an actual WS-2 receipt.  A caller-created observation or clock
    /// timestamp is not accepted as a publication proof.
    pub fn from_receipt(receipt: &MarketReceipt) -> Result<Self, StreamQuoteError> {
        if receipt.epoch().is_nil()
            || receipt.receive_ordinal() == 0
            || receipt.received_at_ms() < 0
            || receipt.socket_open_ms() < 0
            || receipt.socket_open_ms() > receipt.received_at_ms()
        {
            return Err(StreamQuoteError::InvalidReceipt);
        }

        let observation = receipt.observation();
        if !observation.is_regular() || observation.business_date != receipt.session_date() {
            return Err(StreamQuoteError::NonRegularObservation);
        }
        if observation.base_price.is_some()
            || observation.base_price_reason != BasePriceReason::NotProvidedByChannel
        {
            return Err(StreamQuoteError::BasePricePresent);
        }

        let business_date = date_from_wire(observation.business_date)
            .ok_or(StreamQuoteError::InvalidBusinessDate)?;
        let trade_time =
            time_from_wire(observation.trade_time).ok_or(StreamQuoteError::InvalidTradeTime)?;
        let timezone =
            FixedOffset::east_opt(9 * 60 * 60).ok_or(StreamQuoteError::InvalidReceipt)?;
        let provider_trade_at = business_date
            .and_time(trade_time)
            .and_local_timezone(timezone)
            .single()
            .ok_or(StreamQuoteError::InvalidTradeTime)?;

        Ok(Self {
            symbol: observation.symbol.clone(),
            business_date,
            trade_time,
            provider_trade_at,
            price: observation.price.as_str().to_owned(),
            change_from_previous_day: observation.change_amount.as_str().to_owned(),
            change_percent_from_previous_day: observation.change_percent.as_str().to_owned(),
            direction: observation.direction.into(),
            trade_volume: observation.trade_volume,
            cumulative_volume: observation.cumulative_volume,
            base_price: None,
            base_price_reason: StreamBasePriceReason::NotProvidedByChannel,
            halted: observation.halted,
            opening_class: observation.opening_class.clone(),
            hour_class: observation.hour_class.clone(),
            market_class: observation.market_class.clone(),
            transaction_class: observation.transaction_class.clone(),
        })
    }

    pub const fn source(&self) -> &'static str {
        STREAM_SOURCE
    }

    pub const fn wire_version(&self) -> &'static str {
        STREAM_WIRE_VERSION
    }

    pub const fn venue(&self) -> &'static str {
        STREAM_VENUE
    }

    pub const fn currency(&self) -> &'static str {
        STREAM_CURRENCY
    }
}

fn date_from_wire(value: u32) -> Option<NaiveDate> {
    let year = value / 10_000;
    let month = (value / 100) % 100;
    let day = value % 100;
    NaiveDate::from_ymd_opt(year as i32, month, day)
}

fn time_from_wire(value: u32) -> Option<NaiveTime> {
    let hour = value / 10_000;
    let minute = (value / 100) % 100;
    let second = value % 100;
    NaiveTime::from_hms_opt(hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_contract_constants_are_not_the_rest_source() {
        assert_eq!(STREAM_SOURCE, "KIS_MARKET_WS");
        assert_eq!(STREAM_WIRE_VERSION, "kis-h0stcnt0-20260914-v1");
        assert_eq!(STREAM_VENUE, "KRX");
        assert_eq!(STREAM_CURRENCY, "KRW");
        assert_eq!(
            StreamBasePriceReason::NotProvidedByChannel.as_str(),
            "NOT_PROVIDED_BY_CHANNEL"
        );
    }
}
