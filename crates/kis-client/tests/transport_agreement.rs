//! The real transport and the simulator must classify failures IDENTICALLY.
//!
//! Every other test in this crate is written against `BrokerSimulator`. That
//! is only worth anything if the simulator answers the way the real transport
//! answers. If they disagreed about what a timeout means, the suite would
//! still be green and the single most important property it appears to prove
//! — that a timed-out order is never resubmitted — would be the one that
//! broke in production.
//!
//! So this file asserts the agreement directly rather than trusting that two
//! separately-written `match` arms happen to line up. It is deliberately not
//! a test of either component: it is a test of the RELATIONSHIP between them,
//! which nothing else would notice going wrong.

use kis_client::error::KisError;
use kis_client::live_transport::{Failure, classify};
use kis_client::simulator::{BrokerSimulator, Scenario};
use kis_client::transport::{CLIENT_ORDER_ID_HEADER, HttpRequest, Transport};

const ORDER_PATH: &str = "/uapi/domestic-stock/v1/trading/order-cash";
const QUOTE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";

fn order_request(coid: &str) -> HttpRequest {
    HttpRequest::post(ORDER_PATH, "TTTC0802U", "{}").with_header(CLIENT_ORDER_ID_HEADER, coid)
}

fn quote_request() -> HttpRequest {
    HttpRequest::get(QUOTE_PATH, "FHKST01010100")
}

/// The shape of an error, ignoring message text.
///
/// Compared by SHAPE rather than by string so the two implementations may word
/// themselves differently -- a real transport naming a host, a simulator
/// naming a script -- while still being required to agree on the only thing a
/// caller branches on.
fn shape(err: &KisError) -> &'static str {
    match err {
        KisError::Connect { .. } => "Connect",
        KisError::Ambiguous { .. } => "Ambiguous",
        KisError::Broker { .. } => "Broker",
        KisError::RateLimited { .. } => "RateLimited",
        KisError::SchemaDrift { .. } => "SchemaDrift",
        KisError::Auth { .. } => "Auth",
        KisError::ClockSkew { .. } => "ClockSkew",
        KisError::UnknownInstrument { .. } => "UnknownInstrument",
        KisError::Credential(_) => "Credential",
    }
}

#[tokio::test]
async fn a_timed_out_order_is_ambiguous_in_both_implementations() {
    // THE agreement. A mutation that timed out may be sitting at the broker,
    // so neither implementation may report anything a caller could read as
    // "safe to send again".
    let sim = BrokerSimulator::new().script("POST", ORDER_PATH, vec![Scenario::Timeout]);
    let sim_err = sim
        .send(order_request("coid-1"))
        .await
        .expect_err("a timeout is not a success");

    let live_err = classify(Failure::TimedOut, &order_request("coid-1"));

    assert_eq!(shape(&sim_err), "Ambiguous");
    assert_eq!(shape(&live_err), shape(&sim_err));
    assert!(sim_err.is_ambiguous() && live_err.is_ambiguous());
}

#[tokio::test]
async fn both_carry_the_same_correlation_id_out_of_an_ambiguous_order() {
    // The id is how the order is found at the broker afterwards. If one
    // implementation dropped it, the recovery procedure would work in tests
    // and fail in the incident.
    let sim = BrokerSimulator::new().script("POST", ORDER_PATH, vec![Scenario::Timeout]);
    let sim_err = sim.send(order_request("coid-77")).await.unwrap_err();
    let live_err = classify(Failure::TimedOut, &order_request("coid-77"));

    for err in [&sim_err, &live_err] {
        match err {
            KisError::Ambiguous {
                client_order_id, ..
            } => assert_eq!(client_order_id, "coid-77"),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_timed_out_read_is_not_ambiguous_in_either_implementation() {
    // A read that never answered changed nothing. Marking it ambiguous would
    // block a retry that is entirely safe, and would train an operator to
    // ignore the word.
    let sim = BrokerSimulator::new().script("GET", QUOTE_PATH, vec![Scenario::Timeout]);
    let sim_err = sim.send(quote_request()).await.unwrap_err();
    let live_err = classify(Failure::TimedOut, &quote_request());

    assert_eq!(shape(&sim_err), "Broker");
    assert_eq!(shape(&live_err), shape(&sim_err));
    assert!(!sim_err.is_ambiguous() && !live_err.is_ambiguous());
}

#[tokio::test]
async fn a_request_that_never_left_is_connect_in_both_implementations() {
    // The complement of the ambiguous case, and the one that makes it
    // meaningful: this IS safe to retry, including for a mutation.
    let sim = BrokerSimulator::new().script(
        "POST",
        ORDER_PATH,
        vec![Scenario::Unreachable {
            reason: "connection refused".into(),
        }],
    );
    let sim_err = sim.send(order_request("coid-2")).await.unwrap_err();
    let live_err = classify(Failure::NeverSent, &order_request("coid-2"));

    assert_eq!(shape(&sim_err), "Connect");
    assert_eq!(shape(&live_err), shape(&sim_err));
    assert!(
        !sim_err.is_ambiguous() && !live_err.is_ambiguous(),
        "a request that never left leaves nothing unresolved"
    );
}

#[test]
fn the_live_transport_resolves_an_undecidable_timeout_toward_safety() {
    // reqwest cannot distinguish a connect timeout from a response timeout --
    // both are is_timeout(). The live transport therefore treats a POST
    // timeout as AMBIGUOUS even when the connection may never have opened.
    //
    // Being wrong in that direction costs a manual lookup. Being wrong the
    // other way costs a duplicate live order. This test exists so the choice
    // is visible rather than buried in an if.
    let ambiguous = classify(Failure::TimedOut, &order_request("coid-3"));
    let safe = classify(Failure::NeverSent, &order_request("coid-3"));

    assert!(ambiguous.is_ambiguous());
    assert!(!safe.is_ambiguous());
    assert_ne!(
        shape(&ambiguous),
        shape(&safe),
        "'might have arrived' and 'certainly did not' must never collapse into one answer"
    );
}
