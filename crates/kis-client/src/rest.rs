//! The REST client (plan Todo 36, design §6.12).
//!
//! Composes the pieces the rest of the crate proves in isolation: a serialized
//! token, per-endpoint/TR rate limiting, kind-aware bounded retry, explicit
//! mapping, and idempotent order intent.
//!
//! The order this does things in is the safety property, not a style choice:
//!
//! 1. claim the idempotency key — BEFORE anything is sent, so a crash leaves a
//!    record rather than a silent gap;
//! 2. rate-limit — so a throttle is refused locally rather than turning into a
//!    broker ban;
//! 3. attach a token — refreshed on a margin so it cannot die in flight;
//! 4. send under the MUTATION retry policy — one attempt unless the request
//!    provably never left;
//! 5. resolve the intent — and here an ambiguous outcome becomes
//!    [`IntentState::Unknown`], never `Rejected`.
//!
//! Step 5 is where a plausible-looking simplification would be catastrophic.
//! Treating "no usable reply" as a rejection would let the next submission
//! through, and the broker may already hold the first order.

use std::sync::Arc;

use crate::auth::TokenManager;
use crate::error::{KisError, RequestKind};
use crate::idempotency::{IntentState, IntentStore, SubmitGuardError, guard_submission};
use crate::mapping::{
    InstrumentMapper, OrderAck, OrderRequest, order_to_broker_body, parse_order_ack,
};
use crate::rate_limit::{BucketKey, Permit, RateLimiter};
use crate::retry::{RetryPolicy, Sleeper};
use crate::secret::AccountNo;
use crate::transport::{HttpRequest, Transport};

/// Which broker environment the client talks to.
///
/// Mock and live differ ONLY in this value and in which [`Transport`] is
/// installed, so a mock-proven code path is the same path that runs live —
/// and a live host is impossible to reach by forgetting a flag, because there
/// is no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Mock,
    Live,
}

impl Profile {
    pub fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

/// Endpoint paths, kept in one place so a rate-limit key and a request can
/// never disagree about which endpoint they mean.
pub mod paths {
    pub const ORDER_CASH: &str = "/uapi/domestic-stock/v1/trading/order-cash";
    pub const INQUIRE_ORDER: &str = "/uapi/domestic-stock/v1/trading/inquire-psbl-rvsecncl";
    pub const INQUIRE_BALANCE: &str = "/uapi/domestic-stock/v1/trading/inquire-balance";
}

/// The Owner-only KIS REST client.
pub struct RestClient<T: Transport, S: Sleeper> {
    profile: Profile,
    transport: T,
    sleeper: S,
    tokens: Arc<TokenManager>,
    limiter: Arc<RateLimiter>,
    intents: Arc<dyn IntentStore>,
    mapper: InstrumentMapper,
    account: AccountNo,
    product_code: String,
}

impl<T: Transport, S: Sleeper> RestClient<T, S> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile: Profile,
        transport: T,
        sleeper: S,
        tokens: Arc<TokenManager>,
        limiter: Arc<RateLimiter>,
        intents: Arc<dyn IntentStore>,
        account: AccountNo,
        product_code: impl Into<String>,
    ) -> Self {
        Self {
            profile,
            transport,
            sleeper,
            tokens,
            limiter,
            intents,
            mapper: InstrumentMapper,
            account,
            product_code: product_code.into(),
        }
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Attach auth headers. The token is a `Secret`, so it travels in
    /// `secret_headers` and cannot be rendered by a request dump.
    async fn authorize(&self, req: HttpRequest) -> Result<HttpRequest, KisError> {
        let token = self.tokens.token().await?;
        Ok(req.with_secret_header("authorization", token.value))
    }

    fn check_limit(&self, path: &str, tr_id: &str) -> Result<(), KisError> {
        match self.limiter.acquire(&BucketKey::new(path, tr_id)) {
            Permit::Granted => Ok(()),
            // Refusing locally is the point: sending anyway is what turns a
            // throttle into a ban.
            Permit::Throttled { retry_after_ms } => Err(KisError::RateLimited {
                endpoint: path.to_string(),
                retry_after_ms,
            }),
        }
    }

    /// Submit an order, at most once per idempotency key.
    pub async fn submit_order(&self, order: &OrderRequest) -> Result<OrderAck, SubmitError> {
        // 1. Claim BEFORE building or sending anything.
        guard_submission(self.intents.as_ref(), &order.client_order_id)
            .map_err(SubmitError::Guard)?;

        let tr_id = order.side.tr_id();
        let body = order_to_broker_body(&self.mapper, order, &self.account, &self.product_code)
            .map_err(|e| {
                // A mapping failure never reached the broker, so the intent is
                // safely reusable after the caller fixes the order.
                self.intents.set(
                    &order.client_order_id,
                    IntentState::Rejected {
                        reason: e.to_string(),
                    },
                );
                SubmitError::Broker(e)
            })?;

        let result = crate::retry::execute(
            RetryPolicy::mutations(),
            RequestKind::Mutation,
            &self.sleeper,
            |_attempt| {
                let body = body.clone();
                let tr_id = tr_id.to_string();
                let coid = order.client_order_id.clone();
                async move {
                    self.check_limit(paths::ORDER_CASH, &tr_id)?;
                    let req = HttpRequest::post(paths::ORDER_CASH, &tr_id, body)
                        .with_header("tr_id", &tr_id)
                        // Lets the transport name the order in an ambiguous
                        // error, so an unknown outcome is correlatable.
                        .with_header(crate::transport::CLIENT_ORDER_ID_HEADER, coid);
                    let req = self.authorize(req).await?;
                    self.transport.send(req).await
                }
            },
        )
        .await;

        match result {
            Ok(resp) => match parse_order_ack(&resp.body) {
                Ok(ack) => {
                    self.intents.set(
                        &order.client_order_id,
                        IntentState::Acknowledged {
                            broker_order_no: ack.broker_order_no.clone(),
                        },
                    );
                    Ok(ack)
                }
                // The broker answered and refused: no order exists, so the
                // intent is reusable.
                Err(e @ KisError::Broker { .. }) => {
                    self.intents.set(
                        &order.client_order_id,
                        IntentState::Rejected {
                            reason: e.to_string(),
                        },
                    );
                    Err(SubmitError::Broker(e))
                }
                // We could not understand the reply. The order may well exist,
                // so this is UNKNOWN, not rejected.
                Err(e) => {
                    self.intents
                        .set(&order.client_order_id, IntentState::Unknown);
                    Err(SubmitError::Broker(e))
                }
            },
            Err(e) if e.is_ambiguous() => {
                // The whole reason IntentState::Unknown exists.
                self.intents
                    .set(&order.client_order_id, IntentState::Unknown);
                Err(SubmitError::Broker(e))
            }
            Err(e @ KisError::Connect { .. }) => {
                // Provably never sent, so the intent may be reused.
                self.intents.set(
                    &order.client_order_id,
                    IntentState::Rejected {
                        reason: e.to_string(),
                    },
                );
                Err(SubmitError::Broker(e))
            }
            Err(e) => {
                // Anything else was sent and did not clearly resolve. Fail
                // closed to UNKNOWN rather than assuming it did not happen.
                self.intents
                    .set(&order.client_order_id, IntentState::Unknown);
                Err(SubmitError::Broker(e))
            }
        }
    }

    /// Read the account balance. A read, so it retries transient faults.
    pub async fn account_balance(&self) -> Result<String, KisError> {
        let tr_id = "TTTC8434R";
        crate::retry::execute(
            RetryPolicy::reads(),
            RequestKind::Read,
            &self.sleeper,
            |_attempt| async move {
                self.check_limit(paths::INQUIRE_BALANCE, tr_id)?;
                let req = HttpRequest::get(paths::INQUIRE_BALANCE, tr_id)
                    .with_header("tr_id", tr_id)
                    .with_header("CANO", self.account.masked());
                let req = self.authorize(req).await?;
                self.transport.send(req).await.map(|r| r.body)
            },
        )
        .await
    }
}

/// Why a submission did not produce an acknowledgement.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    /// Refused before anything was sent, by the idempotency guard.
    #[error(transparent)]
    Guard(#[from] SubmitGuardError),
    #[error(transparent)]
    Broker(#[from] KisError),
}

impl SubmitError {
    /// Whether the order's fate is unknown. Callers must branch on this before
    /// treating a failure as "no order exists".
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Broker(e) if e.is_ambiguous())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AccessToken, TokenIssuer};
    use crate::clock::{Clock, TestClock};
    use crate::idempotency::InMemoryIntentStore;
    use crate::mapping::{OrderSide, OrderType};
    use crate::rate_limit::Quota;
    use crate::secret::Secret;
    use crate::simulator::{BrokerSimulator, Scenario};

    struct StubIssuer(TestClock);

    #[async_trait::async_trait]
    impl TokenIssuer for StubIssuer {
        async fn issue(&self) -> Result<AccessToken, KisError> {
            Ok(AccessToken {
                value: Secret::new("test-token".to_string()),
                expires_at_ms: self.0.now_ms() + 3_600_000,
            })
        }
    }

    struct NoSleep;
    impl Sleeper for NoSleep {
        fn sleep_ms(&self, _ms: u64) -> impl std::future::Future<Output = ()> + Send {
            std::future::ready(())
        }
    }

    fn client(
        sim: BrokerSimulator,
        intents: Arc<dyn IntentStore>,
    ) -> RestClient<BrokerSimulator, NoSleep> {
        let clock = TestClock::at(0);
        let tokens = Arc::new(TokenManager::new(
            Arc::new(clock.clone()),
            Arc::new(StubIssuer(clock.clone())),
        ));
        let limiter = Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(100, 100)));
        RestClient::new(
            Profile::Mock,
            sim,
            NoSleep,
            tokens,
            limiter,
            intents,
            AccountNo::new("50123456"),
            "01",
        )
    }

    fn order(coid: &str) -> OrderRequest {
        OrderRequest {
            client_order_id: coid.to_string(),
            instrument_id: "069500.KRX".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 10,
            price: Some("40200".to_string()),
        }
    }

    #[tokio::test]
    async fn a_successful_submission_is_acknowledged_and_recorded() {
        let sim = BrokerSimulator::new().script(
            "POST",
            paths::ORDER_CASH,
            vec![Scenario::Ok {
                body: BrokerSimulator::order_ack("0000117057"),
            }],
        );
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, Arc::clone(&intents));

        let ack = c
            .submit_order(&order("coid-1"))
            .await
            .expect("acknowledged");
        assert_eq!(ack.broker_order_no, "0000117057");
        assert_eq!(
            intents.get("coid-1"),
            Some(IntentState::Acknowledged {
                broker_order_no: "0000117057".to_string()
            })
        );
        // ...and the mapping back from broker order number works, which
        // reconciliation needs after a reconnect.
        assert_eq!(
            intents.key_for_broker_order("0000117057").as_deref(),
            Some("coid-1")
        );
    }

    #[tokio::test]
    async fn a_timed_out_submission_is_unknown_attempted_once_and_blocks_resubmission() {
        // The crate's most important end-to-end property.
        let sim = BrokerSimulator::new().script("POST", paths::ORDER_CASH, vec![Scenario::Timeout]);
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, Arc::clone(&intents));

        let err = c
            .submit_order(&order("coid-1"))
            .await
            .expect_err("ambiguous");
        assert!(err.is_ambiguous(), "a submit timeout must be ambiguous");
        assert_eq!(intents.get("coid-1"), Some(IntentState::Unknown));

        // Exactly one attempt reached the broker.
        assert_eq!(c.transport.call_count("POST", paths::ORDER_CASH), 1);

        // And a second submission of the same intent is refused outright.
        let again = c.submit_order(&order("coid-1")).await.expect_err("blocked");
        assert!(matches!(
            again,
            SubmitError::Guard(SubmitGuardError::UnresolvedUnknown { .. })
        ));
        assert_eq!(
            c.transport.call_count("POST", paths::ORDER_CASH),
            1,
            "the blocked resubmission must not reach the broker"
        );
    }

    #[tokio::test]
    async fn an_explicit_rejection_leaves_the_intent_reusable() {
        // A rejection proves no order exists, so the caller may fix and retry.
        let sim = BrokerSimulator::new().script(
            "POST",
            paths::ORDER_CASH,
            vec![Scenario::Ok {
                body: r#"{"rt_cd":"1","msg1":"주문가능금액이 부족합니다","output":{}}"#.to_string(),
            }],
        );
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, Arc::clone(&intents));

        let err = c
            .submit_order(&order("coid-1"))
            .await
            .expect_err("rejected");
        assert!(!err.is_ambiguous(), "an explicit refusal is not ambiguous");
        assert!(matches!(
            intents.get("coid-1"),
            Some(IntentState::Rejected { .. })
        ));
    }

    #[tokio::test]
    async fn an_unreadable_reply_is_unknown_not_rejected() {
        // We could not understand the answer, so the order may well exist.
        let sim = BrokerSimulator::new().script(
            "POST",
            paths::ORDER_CASH,
            vec![Scenario::DriftedSchema {
                body: r#"{"rt_cd":"0","output":{"ORDER_NO":"0000117057"}}"#.to_string(),
            }],
        );
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, Arc::clone(&intents));

        c.submit_order(&order("coid-1")).await.expect_err("drift");
        assert_eq!(
            intents.get("coid-1"),
            Some(IntentState::Unknown),
            "an unparseable ack must not read as 'no order exists'"
        );
    }

    #[tokio::test]
    async fn a_request_that_never_left_is_retried_once_and_stays_reusable() {
        let sim = BrokerSimulator::new().script(
            "POST",
            paths::ORDER_CASH,
            vec![
                Scenario::Unreachable {
                    reason: "connection refused".to_string(),
                },
                Scenario::Ok {
                    body: BrokerSimulator::order_ack("0000117057"),
                },
            ],
        );
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, Arc::clone(&intents));

        let ack = c.submit_order(&order("coid-1")).await.expect("second wins");
        assert_eq!(ack.broker_order_no, "0000117057");
        assert_eq!(c.transport.call_count("POST", paths::ORDER_CASH), 2);
    }

    #[tokio::test]
    async fn a_balance_read_retries_a_transient_500() {
        let sim = BrokerSimulator::new().script(
            "GET",
            paths::INQUIRE_BALANCE,
            vec![
                Scenario::ServerError {
                    status: 500,
                    body: "{}".to_string(),
                },
                Scenario::Ok {
                    body: r#"{"rt_cd":"0","output1":[]}"#.to_string(),
                },
            ],
        );
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(sim, intents);

        let body = c.account_balance().await.expect("recovers");
        assert!(body.contains("output1"));
        assert_eq!(c.transport.call_count("GET", paths::INQUIRE_BALANCE), 2);
    }

    #[tokio::test]
    async fn a_local_throttle_refuses_before_reaching_the_broker() {
        let clock = TestClock::at(0);
        let tokens = Arc::new(TokenManager::new(
            Arc::new(clock.clone()),
            Arc::new(StubIssuer(clock.clone())),
        ));
        // One call allowed, no refill.
        let limiter = Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(1, 0)));
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = RestClient::new(
            Profile::Mock,
            BrokerSimulator::new(),
            NoSleep,
            tokens,
            limiter,
            intents,
            AccountNo::new("50123456"),
            "01",
        );

        c.account_balance().await.expect("first call fits");
        let err = c.account_balance().await.expect_err("throttled locally");
        assert!(matches!(err, KisError::RateLimited { .. }));
        assert_eq!(
            c.transport.call_count("GET", paths::INQUIRE_BALANCE),
            1,
            "a locally throttled call must not reach the broker"
        );
    }

    #[tokio::test]
    async fn a_mapping_failure_never_reaches_the_broker() {
        let intents: Arc<dyn IntentStore> = Arc::new(InMemoryIntentStore::new());
        let c = client(BrokerSimulator::new(), Arc::clone(&intents));
        let mut bad = order("coid-1");
        bad.instrument_id = "AAPL.NASDAQ".to_string();

        c.submit_order(&bad).await.expect_err("unmappable");
        assert_eq!(c.transport.call_count("POST", paths::ORDER_CASH), 0);
        // Nothing was sent, so the intent is safely reusable.
        assert!(matches!(
            intents.get("coid-1"),
            Some(IntentState::Rejected { .. })
        ));
    }

    #[test]
    fn the_profile_is_explicit_with_no_default() {
        // There is no Default impl: a live host cannot be reached by
        // forgetting a flag.
        assert!(Profile::Live.is_live());
        assert!(!Profile::Mock.is_live());
    }
}
