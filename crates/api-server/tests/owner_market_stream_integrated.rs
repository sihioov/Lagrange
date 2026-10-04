#![cfg(feature = "market-stream-db-tests")]

pub mod owner_equity_v2 {
    pub mod market_stream {
        pub use job_queue::owner_equity_v2::{
            StreamIdentity, StreamLeaseIdentity, StreamSessionProof,
        };
    }
}

#[path = "owner_market_stream_integrated_support/heap_census.rs"]
mod heap_census;
#[path = "owner_market_stream_integrated_support/heap_census_tests.rs"]
mod heap_census_tests;
#[path = "owner_market_stream_integrated_support/resource_measurements.rs"]
mod resource_measurements;

#[global_allocator]
static TEST_HEAP: heap_census::Census = heap_census::Census::new(std::alloc::System);

#[path = "../../job-queue/tests/owner_market_stream_boundary_support/mod.rs"]
mod boundary_support;
mod owner_market_stream_http_support;
mod owner_market_stream_integrated_support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a root-reviewed isolated runtime/browser runner"]
async fn integrated_runtime_api_fixture() {
    let outcome = owner_market_stream_integrated_support::run_fixture().await;
    assert!(
        outcome.is_ok(),
        "integrated runtime/API fixture lifecycle failed: {}",
        outcome
            .err()
            .map_or("unknown fixture error", |error| error.code())
    );
}
