//! Pure HTTP-origin and bounded delivery tests. No pools, sockets or fixtures.
#![allow(dead_code)]
#[path = "../src/http/owner_market_stream_config.rs"]
mod owner_market_stream_config;
#[path = "../src/http/owner_market_stream_contract.rs"]
mod owner_market_stream_contract;
#[path = "../src/http/owner_market_stream_delivery.rs"]
mod owner_market_stream_delivery;
#[path = "../src/http/owner_market_stream_projection.rs"]
mod owner_market_stream_projection;
