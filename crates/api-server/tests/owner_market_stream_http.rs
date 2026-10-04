#![cfg(feature = "market-stream-db-tests")]

pub mod owner_equity_v2 {
    pub mod market_stream {
        pub use job_queue::owner_equity_v2::{
            StreamIdentity, StreamLeaseIdentity, StreamSessionProof,
        };
    }
}

#[path = "../../job-queue/tests/owner_market_stream_boundary_support/mod.rs"]
mod boundary_support;
mod owner_market_stream_http_support;

#[tokio::test]
async fn binding_helper_is_app_only_and_reversible() {
    owner_market_stream_http_support::run(owner_market_stream_http_support::Case::BindingHelper)
        .await;
}

#[tokio::test]
async fn leases_authenticate_and_fence_consumer_session_and_sequence() {
    owner_market_stream_http_support::run(
        owner_market_stream_http_support::Case::LeaseAuthentication,
    )
    .await;
}

#[tokio::test]
async fn sse_initial_snapshot_is_private_and_read_only() {
    owner_market_stream_http_support::run(owner_market_stream_http_support::Case::InitialSnapshot)
        .await;
}

#[tokio::test]
async fn sse_replacement_and_revocation_clear_old_memberships() {
    owner_market_stream_http_support::run(
        owner_market_stream_http_support::Case::ReplacementAndRevocation,
    )
    .await;
}

#[tokio::test]
async fn twenty_consumers_are_bounded_and_drops_release_capacity() {
    owner_market_stream_http_support::run(owner_market_stream_http_support::Case::ConsumerCapacity)
        .await;
}

#[tokio::test]
async fn disabled_or_revoked_binding_denies_demand_and_stream() {
    owner_market_stream_http_support::run(
        owner_market_stream_http_support::Case::DisabledAndRevoked,
    )
    .await;
}
