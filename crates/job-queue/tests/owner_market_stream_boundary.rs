//! WS-3A storage boundary tests.
//!
//! The actual-role database case lives in the private library test module;
//! this external target checks only the safe public DTO boundary.

use job_queue::owner_equity_v2::{
    MarketStreamStorageError, StreamConnectionState, StreamFreshness, StreamMarketState,
    StreamStatus, StreamStatusCode,
};
use market_data::market_stream::{
    STREAM_CURRENCY, STREAM_SOURCE, STREAM_VENUE, STREAM_WIRE_VERSION, StreamBasePriceReason,
    StreamQuote, StreamQuoteDirection,
};

#[test]
fn stream_storage_exports_remain_separate_from_rest_intraday_contract() {
    assert_eq!(STREAM_SOURCE, "KIS_MARKET_WS");
    assert_eq!(STREAM_WIRE_VERSION, "kis-h0stcnt0-20260914-v1");
    assert_eq!(STREAM_VENUE, "KRX");
    assert_eq!(STREAM_CURRENCY, "KRW");
    assert_eq!(
        StreamBasePriceReason::NotProvidedByChannel.as_str(),
        "NOT_PROVIDED_BY_CHANNEL"
    );
    assert!(std::mem::size_of::<StreamQuote>() > 0);
    assert_ne!(
        std::any::type_name::<StreamQuoteDirection>(),
        std::any::type_name::<market_data::intraday_quotes::IntradayQuoteDirection>()
    );
}
