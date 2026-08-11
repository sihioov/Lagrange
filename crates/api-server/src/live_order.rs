//! Submitting a Live order: the orchestration that strings the pieces together
//! (plan Todos 38-42).
//!
//! Every piece already existed and none of them called each other. This is the
//! function that does, and it is generic over [`Transport`] so the whole path
//! is testable against the simulator without credentials or a network.
//!
//! # The order of operations IS the safety
//!
//!   1. CLAIM the intent, keyed on the client's `Idempotency-Key`. An intent
//!      that already exists returns its current state and goes no further --
//!      re-entering the gate would hit 0018's one-decision-per-intent index,
//!      report `NotPersisted` (CRITICAL), and page an operator for a benign
//!      retry while telling the caller the order was refused when it had
//!      actually been accepted.
//!   2. GATE it. The approval is an unforgeable, single-use token that only a
//!      gate run which BOTH approved and durably recorded can mint.
//!   3. RECORD the approval, consuming the token. From here the intent is
//!      submittable and the token cannot be used again.
//!   4. SUBMIT, recording `SUBMITTING` BEFORE the request leaves. A crash
//!      between the two leaves an intent that a startup sweep will find,
//!      rather than one that looks untouched.
//!   5. RECORD what came back.
//!
//! # Dry run
//!
//! Todo 42 asks for a staged `dry-run -> shadow -> low-value` rollout. A dry
//! run here evaluates the twelve checks and reports what WOULD happen, and
//! deliberately writes NOTHING -- no intent, no risk decision, no events.
//!
//! That is not laziness, it is the only correct shape. Recording a decision
//! would consume the intent's one permitted gate decision (0018), so a real
//! submission of the same order could never be authorised afterwards; and
//! creating an intent row would leave reconciliation looking for an order at
//! the broker that was never going to be sent. A dry run is a PREDICTION, and
//! a prediction that mutates the world is not one.

use crate::error::{TenancyError, TenancyResult};
use crate::repos::order_intents::{Claim, NewOrderIntent, OrderIntentRepo};
use crate::repos::risk::RiskRepo;
use crate::risk_snapshot::{parse_side, side_str};
use domain::{Price, Quantity};
use kis_client::mapping::{OrderRequest, OrderSide, OrderType};
use kis_client::order_state::Event;
use kis_client::rest::RestClient;
use kis_client::retry::Sleeper;
use kis_client::transport::Transport;
use risk_gateway::snapshot::Side;
use risk_gateway::{Decision, RiskLimits, RiskSnapshot};

/// Wall-clock seconds, carried into the snapshot so a replay re-evaluates
/// against the instant the decision was actually made.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the order is rehearsed or actually sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Evaluate and report; write nothing, send nothing.
    DryRun,
    /// The real thing.
    Live,
}

/// What a submission attempt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// A dry run. Nothing was written and nothing was sent.
    Rehearsed {
        would_submit: bool,
        decision: Box<Decision>,
    },
    /// The intent already existed; this request changed nothing. The state is
    /// whatever the FIRST request achieved.
    AlreadySubmitted { intent_ref: String, state: String },
    /// The gate refused. No order exists.
    Denied {
        intent_ref: String,
        reason: String,
        severity: String,
    },
    /// The broker acknowledged.
    Accepted {
        intent_ref: String,
        broker_order_no: String,
    },
    /// The submission timed out. The broker MAY hold this order; it must be
    /// resolved by a lookup and must never be resubmitted.
    Unresolved {
        intent_ref: String,
        client_order_id: String,
    },
}

/// Everything a submission needs that is not already in a repo.
pub struct SubmitRequest {
    pub intent: NewOrderIntent,
    pub snapshot: RiskSnapshot,
    pub limits: RiskLimits,
    pub mode: Mode,
}

/// Runs the whole path.
///
/// Takes the repos and the broker client by reference so a caller can supply a
/// simulator; nothing here knows or cares which it has.
pub async fn submit<T: Transport, S: Sleeper>(
    intents: &OrderIntentRepo,
    risk: &RiskRepo,
    broker: &RestClient<T, S>,
    request: SubmitRequest,
) -> TenancyResult<Submission> {
    // --- dry run: decide, report, touch nothing ------------------------------
    if request.mode == Mode::DryRun {
        let decision = risk_gateway::evaluate(&request.snapshot, &request.limits);
        return Ok(Submission::Rehearsed {
            would_submit: decision.is_approved(),
            decision: Box::new(decision),
        });
    }

    // --- 1. claim ------------------------------------------------------------
    let claim = intents.claim(request.intent.clone()).await?;
    let intent_ref = claim.row().intent_ref.clone();
    if let Claim::Existing(row) = claim {
        // The retry path. Answer from what already happened rather than doing
        // it again.
        return Ok(Submission::AlreadySubmitted {
            intent_ref: row.intent_ref,
            state: row.state,
        });
    }

    // --- 2. gate -------------------------------------------------------------
    let mut snapshot = request.snapshot;
    // The gate must assess THIS intent, not whatever the caller's snapshot
    // happened to name.
    snapshot.intent.intent_ref = intent_ref.clone();
    let outcome = risk_gateway::evaluate_and_record(&snapshot, &request.limits, risk).await;
    let decision = outcome.decision().clone();

    let Some(approval) = outcome.into_approval() else {
        // Record the denial on the intent so its history explains itself, then
        // stop. No token was minted, so nothing below could run anyway.
        let reason = decision
            .reason
            .map_or("UNKNOWN", |r| r.as_str())
            .to_string();
        let _ = intents
            .append(
                &intent_ref,
                &Event::RiskDenied {
                    reason: reason.clone(),
                },
            )
            .await;
        return Ok(Submission::Denied {
            intent_ref,
            reason,
            severity: decision.severity().to_string(),
        });
    };

    // --- 3. record the approval, consuming the token --------------------------
    intents.record_approval(&intent_ref, approval).await?;

    // --- 4. submit -----------------------------------------------------------
    // SUBMITTING before the request leaves, so a crash mid-flight leaves an
    // intent the startup sweep will find.
    intents
        .append(&intent_ref, &Event::SubmissionStarted)
        .await?;

    let order = OrderRequest {
        client_order_id: intent_ref.clone(),
        instrument_id: request.intent.instrument_id.clone(),
        // Parsed once at the route and carried typed ever since, so there is
        // no arm here that can turn an unrecognised value into a BUY.
        side: match parse_side(&request.intent.side) {
            Some(Side::Sell) => OrderSide::Sell,
            Some(Side::Buy) => OrderSide::Buy,
            None => {
                return Err(TenancyError::InvalidState(format!(
                    "intent {intent_ref} carries an unrecognised side {:?}",
                    request.intent.side
                )));
            }
        },
        order_type: if request.intent.price.is_some() {
            OrderType::Limit
        } else {
            OrderType::Market
        },
        quantity: Quantity::parse(&request.intent.quantity)
            .map_err(|e| {
                TenancyError::InvalidState(format!("intent {intent_ref} quantity is unusable: {e}"))
            })?
            .to_u64()
            .map_err(|e| {
                TenancyError::InvalidState(format!("intent {intent_ref} quantity overflows: {e}"))
            })?,
        price: request.intent.price.clone(),
    };

    match broker.submit_order(&order).await {
        Ok(ack) => {
            intents.append(&intent_ref, &Event::SubmissionSent).await?;
            intents
                .append(
                    &intent_ref,
                    &Event::BrokerAccepted {
                        broker_order_no: ack.broker_order_no.clone(),
                    },
                )
                .await?;
            Ok(Submission::Accepted {
                intent_ref,
                broker_order_no: ack.broker_order_no,
            })
        }
        Err(err) if is_ambiguous(&err) => {
            // The one outcome that must never be reported as a failure: the
            // broker may hold this order. UNKNOWN is recorded so nothing can
            // resubmit it, and only a lookup settles it.
            intents
                .append(&intent_ref, &Event::SubmissionTimedOut)
                .await?;
            Ok(Submission::Unresolved {
                intent_ref: intent_ref.clone(),
                client_order_id: intent_ref,
            })
        }
        Err(err) => {
            // The request never left, or the broker refused outright. Either
            // way no order exists, so the intent terminates as rejected and a
            // retry is a NEW intent with a new gate run.
            let reason = format!("{err}");
            intents
                .append(
                    &intent_ref,
                    &Event::BrokerRejected {
                        reason: reason.clone(),
                    },
                )
                .await?;
            Ok(Submission::Denied {
                intent_ref,
                reason,
                severity: "WARNING".to_string(),
            })
        }
    }
}

/// Whether an error leaves the order's existence unresolved.
///
/// A free function rather than an inline match so the question is asked in one
/// place. Getting this wrong in either direction is the whole hazard: treating
/// ambiguity as failure invites a resubmission that doubles a live position,
/// and treating a clean failure as ambiguity strands an order that never
/// existed.
fn is_ambiguous(err: &kis_client::rest::SubmitError) -> bool {
    match err {
        kis_client::rest::SubmitError::Broker(k) => k.is_ambiguous(),
        // The guard refused before anything was sent. Nothing is unresolved.
        kis_client::rest::SubmitError::Guard(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Building a broker client from a stored connection.
// ---------------------------------------------------------------------------

use crate::http::session::Session;
use crate::http::state::ApiState;
use crate::repos::live::BrokerConnectionRow;
use kis_client::auth::TokenManager;
use kis_client::clock::SystemClock;
use kis_client::idempotency::InMemoryIntentStore;
use kis_client::live_transport::LiveTransport;
use kis_client::rate_limit::{Quota, RateLimiter};
use kis_client::rest::Profile;
use kis_client::retry::TokioSleeper;
use kis_client::secret::{AccountNo, CredentialRef, CredentialSource, SystemCredentialSource};
use kis_client::token_issuer::KisTokenIssuer;
use std::sync::Arc;
use std::time::Duration;

/// What a route knows about the order it was asked to place.
pub struct OrderInput {
    pub account_id: uuid::Uuid,
    pub instrument_id: String,
    /// Typed, because the route rejects anything that is not BUY or SELL.
    ///
    /// This was a `String` compared as `eq_ignore_ascii_case("SELL")` with a
    /// bare `else` arm, so `"SEL"`, `"sell "`, `""` and every other value
    /// became a BUY -- a typo silently reversing an order's direction.
    pub side: Side,
    /// Whole units. `Quantity::parse` refuses a fractional value, which is
    /// what `kis_client::mapping::OrderRequest` documents it wants: "a
    /// fractional quantity is a bug to surface, not round". The previous code
    /// did the opposite, truncating "10.7" to 10 on the way to the broker.
    pub quantity: Quantity,
    pub price: Option<Price>,
    pub client_key: String,
    pub correlation_id: String,
    pub dry_run: bool,
}

/// How long a broker call may take before it is treated as unresolved.
///
/// Short on purpose. A longer timeout does not make an order more likely to
/// succeed; it only widens the window in which the operator knows nothing, and
/// every second of that window is a second where a duplicate could be issued
/// by someone who ran out of patience.
const BROKER_TIMEOUT: Duration = Duration::from_secs(10);

/// Parses a stored credential reference back into its typed form.
///
/// The database CHECK and the route both refuse anything that is not a
/// reference, so a malformed one here means the row was written by something
/// that bypassed both — worth failing loudly rather than defaulting.
fn parse_ref(raw: &str) -> Result<CredentialRef, TenancyError> {
    if let Some(var) = raw.strip_prefix("env:") {
        return Ok(CredentialRef::env(var));
    }
    if let Some(path) = raw.strip_prefix("file:") {
        return Ok(CredentialRef::file(path));
    }
    Err(TenancyError::InvalidState(format!(
        "stored credential reference is not a reference: {raw}"
    )))
}

/// Submits (or rehearses) an order through a stored connection.
///
/// The profile decides which KIS host is reached, and nothing else does. There
/// is no flag, no environment variable, and no default that can send a `mock`
/// connection's order to the live endpoint: the two are separate constructors
/// chosen by an explicit match on a value an Owner had to write down.
pub async fn submit_through_connection(
    state: &ApiState,
    session: &Session,
    owner: uuid::Uuid,
    connection: &BrokerConnectionRow,
    input: OrderInput,
) -> TenancyResult<Submission> {
    let intents = state.order_intents(&session.actor(), owner);
    let risk = state.risk(&session.actor(), owner, Some(input.account_id));

    // The gate must be asked about THIS order.
    //
    // What stood here was `risk_gateway::testing::snapshot_all_green()` with
    // two fields overwritten, and `testing::limits()` beside it. The gate
    // therefore approved a fixture -- a 10-unit buy at 7,250 against a
    // fabricated account, measured against limits nobody configured -- while
    // the caller's real order went to the broker. See `risk_snapshot` for the
    // full account; the property that matters is that the snapshot below and
    // the `OrderRequest` built later describe the same order.
    let intent_ref = NewOrderIntent::mint_ref();
    let snapshot = crate::risk_snapshot::for_submission(
        &state.app_pool,
        &session.actor(),
        &state.reconciliation(&session.actor(), owner),
        Some(connection.id),
        state
            .live(&session.actor())
            .kill_switch_engaged()
            .await
            .ok(),
        &crate::risk_snapshot::GateOrder {
            intent_ref: intent_ref.clone(),
            account_id: input.account_id,
            instrument_id: input.instrument_id.clone(),
            side: input.side,
            quantity: input.quantity,
            price: input.price,
            correlation_id: input.correlation_id.clone(),
        },
        now_secs(),
    )
    .await?;

    let request = SubmitRequest {
        intent: NewOrderIntent {
            intent_ref,
            account_id: input.account_id,
            instrument_id: input.instrument_id.clone(),
            side: side_str(input.side).to_string(),
            quantity: input.quantity.as_decimal_string(),
            price: input.price.map(|p| p.as_decimal_string()),
            correlation_id: input.correlation_id.clone(),
            client_key: input.client_key.clone(),
        },
        snapshot,
        limits: crate::risk_snapshot::limits_for(&state.app_pool, &session.actor(), owner).await?,
        mode: if input.dry_run {
            Mode::DryRun
        } else {
            Mode::Live
        },
    };

    // A dry run reaches no broker, so it needs no transport and no
    // credentials. Building one anyway would make a rehearsal fail for a
    // reason that has nothing to do with the order.
    if input.dry_run {
        let decision = risk_gateway::evaluate(&request.snapshot, &request.limits);
        return Ok(Submission::Rehearsed {
            would_submit: decision.is_approved(),
            decision: Box::new(decision),
        });
    }

    let profile = if connection.profile == "live" {
        Profile::Live
    } else {
        Profile::Mock
    };
    let transport = match profile {
        Profile::Live => LiveTransport::live(BROKER_TIMEOUT),
        Profile::Mock => LiveTransport::sandbox(BROKER_TIMEOUT),
    }
    .map_err(|e| TenancyError::InvalidState(format!("broker transport unavailable: {e}")))?;

    let token_transport = match profile {
        Profile::Live => LiveTransport::live(BROKER_TIMEOUT),
        Profile::Mock => LiveTransport::sandbox(BROKER_TIMEOUT),
    }
    .map_err(|e| TenancyError::InvalidState(format!("broker transport unavailable: {e}")))?;

    let account_raw = SystemCredentialSource
        .resolve(&parse_ref(&connection.account_ref)?)
        .map_err(|e| TenancyError::InvalidState(format!("account number unavailable: {e}")))?;

    let issuer = KisTokenIssuer::new(
        token_transport,
        SystemCredentialSource,
        parse_ref(&connection.app_key_ref)?,
        parse_ref(&connection.secret_ref)?,
        || chrono::Utc::now().timestamp_millis(),
    );

    let broker = RestClient::new(
        profile,
        transport,
        TokioSleeper,
        Arc::new(TokenManager::new(Arc::new(SystemClock), Arc::new(issuer))),
        // KIS's documented per-second allowance. The limiter refuses locally
        // rather than sending and being throttled: sending anyway is what
        // turns a throttle into a ban.
        Arc::new(RateLimiter::new(
            Arc::new(SystemClock),
            Quota::new(20, 1000),
        )),
        Arc::new(InMemoryIntentStore::new()),
        AccountNo::new(account_raw.expose().clone()),
        connection.account_product_code.clone(),
    );

    submit(&intents, &risk, &broker, request).await
}
