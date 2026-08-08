//! Live order intents and their append-only event log (plan Todo 39).
//!
//! # Claim before gate
//!
//! [`OrderIntentRepo::claim`] is deliberately the FIRST thing a submission
//! route does, before the Risk Gateway is consulted. Tracing a duplicate POST
//! through the alternative shows why: the gate would run again, `RiskRepo`
//! would hit 0018's unique index, the store error would become
//! `DenyReason::NotPersisted` — which is graded CRITICAL — and a client's
//! benign retry would page an operator. Worse, the caller would be told the
//! order was refused when in fact it was already accepted.
//!
//! So a claim that finds an existing intent returns it and the route reports
//! that intent's current state. The gate runs exactly once per intent, which
//! is also what makes 0018's one-decision-per-intent index the right shape
//! rather than a trap.
//!
//! # The log is the truth
//!
//! `order_intents.state` is a derived cache of `order_intent_events`.
//! [`OrderIntentRepo::append`] writes the event and updates the cache in ONE
//! transaction, so they cannot diverge across a crash; [`replay_state`] exists
//! to prove they have not.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use kis_client::order_state::{Applied, Event, OrderIntentState, TransitionError};
use risk_gateway::RiskApproval;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// A stored intent.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OrderIntentRow {
    pub intent_ref: String,
    pub account_id: Uuid,
    pub instrument_id: String,
    pub side: String,
    /// Exact decimal string; numerics cross the wire as text so nothing is
    /// rounded on the way in or out.
    pub quantity: String,
    pub price: Option<String>,
    pub correlation_id: String,
    /// The client `Idempotency-Key` this intent was created for.
    pub client_key: Option<String>,
    pub state: String,
    pub broker_order_no: Option<String>,
    pub cumulative_filled: String,
    pub state_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const INTENT_COLUMNS: &str = "intent_ref, account_id, instrument_id, side, \
     quantity::text AS quantity, price::text AS price, correlation_id, client_key, state, \
     broker_order_no, cumulative_filled::text AS cumulative_filled, state_reason, \
     created_at, updated_at";

/// What a claim found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The intent did not exist and is now recorded as `INTENT_CREATED`. The
    /// caller owns it and must proceed to the gate.
    Created(OrderIntentRow),
    /// The intent already existed. The caller must NOT re-gate or resubmit;
    /// the row says what actually happened to it.
    Existing(OrderIntentRow),
}

impl Claim {
    pub fn row(&self) -> &OrderIntentRow {
        match self {
            Claim::Created(r) | Claim::Existing(r) => r,
        }
    }

    /// Whether this caller may proceed to the Risk Gateway.
    pub const fn is_new(&self) -> bool {
        matches!(self, Claim::Created(_))
    }
}

/// A new intent to claim.
#[derive(Debug, Clone)]
pub struct NewOrderIntent {
    /// Server-generated and globally unique — see migration 0019.
    pub intent_ref: String,
    pub account_id: Uuid,
    pub instrument_id: String,
    pub side: String,
    pub quantity: String,
    pub price: Option<String>,
    pub correlation_id: String,
    /// The client's `Idempotency-Key`. THIS is what a retransmission repeats,
    /// and therefore what deduplication has to key on — see migration 0020.
    pub client_key: String,
}

impl NewOrderIntent {
    /// Mints a fresh, globally unique intent reference.
    ///
    /// Server-generated on purpose. A client-composed ref (account plus a
    /// client sequence, say) could collide across accounts, and 0018's gate
    /// index is global — the second account would be unable to record a
    /// decision at all.
    pub fn mint_ref() -> String {
        format!("oi_{}", Uuid::new_v4().simple())
    }
}

/// Live order intents for one actor.
pub struct OrderIntentRepo {
    pool: PgPool,
    actor: Actor,
    owner_user_id: Uuid,
}

impl OrderIntentRepo {
    pub fn new(pool: PgPool, actor: Actor, owner_user_id: Uuid) -> Self {
        Self {
            pool,
            actor,
            owner_user_id,
        }
    }

    /// Claims an intent, or returns the one this client key already made.
    ///
    /// Deduplication keys on the CLIENT's `Idempotency-Key`, never on the
    /// server-minted `intent_ref`. The ref is different on every request, so
    /// it can never match a retransmission — and keying on it would leave
    /// FR-LIVE-003 unmet in the worst possible way. Trace AT-09: the client
    /// POSTs, the response times out, the client retransmits, the route mints
    /// a NEW ref, the claim finds nothing to deduplicate against, the gate
    /// runs again and legitimately approves, and a SECOND REAL ORDER reaches
    /// the broker. Every individual step is correct, and the state machine
    /// cannot help, because the two submissions are two distinct intents.
    ///
    /// `ON CONFLICT DO NOTHING` plus a read makes the outcome the database's
    /// to decide, so two concurrent retransmissions cannot both be told they
    /// created it.
    pub async fn claim(&self, input: NewOrderIntent) -> TenancyResult<Claim> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;

        let inserted = sqlx::query_scalar::<_, String>(
            "INSERT INTO order_intents \
             (intent_ref, owner_user_id, account_id, instrument_id, side, quantity, price, \
              correlation_id, client_key) \
             VALUES ($1, $2, $3, $4, $5, $6::numeric, $7::numeric, $8, $9) \
             ON CONFLICT (owner_user_id, client_key) WHERE client_key IS NOT NULL \
             DO NOTHING \
             RETURNING intent_ref",
        )
        .bind(&input.intent_ref)
        .bind(self.owner_user_id)
        .bind(input.account_id)
        .bind(&input.instrument_id)
        .bind(&input.side)
        .bind(&input.quantity)
        .bind(input.price.as_deref())
        .bind(&input.correlation_id)
        .bind(&input.client_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| TenancyError::Forbidden)?;

        // Read by the CLIENT key, not by the minted ref: on a retransmission
        // the ref in `input` was never stored, so reading by it would miss the
        // very row this method exists to find.
        let row = sqlx::query_as::<_, OrderIntentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {INTENT_COLUMNS} FROM order_intents \
             WHERE owner_user_id = $1 AND client_key = $2"
        )))
        .bind(self.owner_user_id)
        .bind(&input.client_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| TenancyError::NotFound)?;

        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;

        Ok(if inserted.is_some() {
            Claim::Created(row)
        } else {
            Claim::Existing(row)
        })
    }

    /// Records the gate's approval, CONSUMING the token that proves it.
    ///
    /// This is where Todo 38's compile-time guarantee is switched back on at
    /// the layer that will actually be called. `Event::RiskApproved` has to
    /// stay freely constructible — [`Self::events`] deserializes it to replay
    /// the log — so the enum cannot be the enforcement point. This method is:
    /// it takes [`RiskApproval`] by value, so a caller must have obtained one
    /// from a gate run that both approved AND durably recorded, and cannot
    /// use it twice.
    ///
    /// The approval must name THIS intent. One laundered from another intent
    /// would authorise an order the gate never assessed.
    pub async fn record_approval(
        &self,
        intent_ref: &str,
        approval: RiskApproval,
    ) -> TenancyResult<AppendOutcome> {
        if approval.intent_ref() != intent_ref {
            return Err(TenancyError::InvalidState(format!(
                "approval authorises intent {}, not {intent_ref}",
                approval.intent_ref()
            )));
        }
        self.append(intent_ref, &Event::RiskApproved).await
    }

    /// Loads an intent's current state as the machine understands it.
    pub async fn state(&self, intent_ref: &str) -> TenancyResult<OrderIntentState> {
        let row = self.get(intent_ref).await?;
        state_from_row(&row).ok_or(TenancyError::NotFound)
    }

    pub async fn get(&self, intent_ref: &str) -> TenancyResult<OrderIntentRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, OrderIntentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {INTENT_COLUMNS} FROM order_intents WHERE intent_ref = $1"
        )))
        .bind(intent_ref)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| TenancyError::NotFound)?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;
        Ok(row)
    }

    /// Applies an event: validates the transition, appends it, and moves the
    /// cached state — all in one transaction.
    ///
    /// The transition is decided by `kis-client`'s pure machine rather than by
    /// SQL, so there is exactly one definition of what is legal. A rejected
    /// transition writes NOTHING: an illegal event is not history, it is a
    /// bug or a hostile message.
    ///
    /// An event the machine reports as `NoChange` — a broker re-send — is
    /// still APPENDED, because it genuinely happened and a reconciliation
    /// reading the log should see that the broker said it twice. It simply
    /// does not move the state, which is what keeps the ledger from being
    /// touched twice.
    pub async fn append(&self, intent_ref: &str, event: &Event) -> TenancyResult<AppendOutcome> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;

        // Lock the intent for the duration: two fill reports racing would
        // otherwise both read the same state and both claim seq n+1, and one
        // of them would lose to the unique constraint after doing its work.
        let row = sqlx::query_as::<_, OrderIntentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {INTENT_COLUMNS} FROM order_intents WHERE intent_ref = $1 FOR UPDATE"
        )))
        .bind(intent_ref)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| TenancyError::NotFound)?;

        let current = state_from_row(&row).ok_or(TenancyError::NotFound)?;
        let applied = current
            .apply(event)
            .map_err(|e| TenancyError::InvalidState(e.to_string()))?;

        let next = match &applied {
            Applied::Moved(next) => next.clone(),
            Applied::NoChange => current.clone(),
        };

        let seq = sqlx::query_scalar::<_, i32>(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM order_intent_events WHERE intent_ref = $1",
        )
        .bind(intent_ref)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| TenancyError::Forbidden)?;

        sqlx::query(
            "INSERT INTO order_intent_events \
             (intent_ref, owner_user_id, seq, event_type, payload_json, resulting_state) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(intent_ref)
        .bind(self.owner_user_id)
        .bind(seq)
        .bind(event.name())
        .bind(serde_json::to_value(event).unwrap_or(serde_json::Value::Null))
        .bind(next.name())
        .execute(&mut *tx)
        .await
        .map_err(|_| TenancyError::Forbidden)?;

        if matches!(applied, Applied::Moved(_)) {
            // A FILLED order is filled to its full quantity; carrying the
            // last partial's total forward would leave the row saying 6 of 10
            // filled on a completed order.
            let cumulative = match &next {
                OrderIntentState::Filled { .. } => row.quantity.clone(),
                other => cumulative_of(other).unwrap_or_else(|| row.cumulative_filled.clone()),
            };
            sqlx::query(
                "UPDATE order_intents SET state = $2, broker_order_no = $3, \
                 cumulative_filled = $4::numeric, state_reason = $5, updated_at = now() \
                 WHERE intent_ref = $1",
            )
            .bind(intent_ref)
            .bind(next.name())
            .bind(next.broker_order_no())
            .bind(&cumulative)
            .bind(reason_of(&next))
            .execute(&mut *tx)
            .await
            .map_err(|_| TenancyError::Forbidden)?;
        }

        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;

        Ok(AppendOutcome {
            seq,
            state: next,
            moved: matches!(applied, Applied::Moved(_)),
        })
    }

    /// Every event for an intent, in sequence order.
    pub async fn events(&self, intent_ref: &str) -> TenancyResult<Vec<(i32, Event)>> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let rows = sqlx::query_as::<_, (i32, serde_json::Value)>(
            "SELECT seq, payload_json FROM order_intent_events \
             WHERE intent_ref = $1 ORDER BY seq",
        )
        .bind(intent_ref)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| TenancyError::NotFound)?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;

        Ok(rows
            .into_iter()
            .filter_map(|(seq, payload)| {
                serde_json::from_value::<Event>(payload)
                    .ok()
                    .map(|e| (seq, e))
            })
            .collect())
    }

    /// Intents that cannot be left alone: in flight, or `UNKNOWN` awaiting a
    /// broker lookup. Todo 40's reconciliation reads this.
    pub async fn unresolved(&self) -> TenancyResult<Vec<OrderIntentRow>> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let rows = sqlx::query_as::<_, OrderIntentRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {INTENT_COLUMNS} FROM order_intents \
             WHERE state IN ('SUBMITTING', 'SUBMITTED', 'UNKNOWN') ORDER BY created_at"
        )))
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| TenancyError::Forbidden)?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;
        Ok(rows)
    }
}

/// What `append` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOutcome {
    pub seq: i32,
    pub state: OrderIntentState,
    /// False for a broker re-send: recorded, but the state (and the ledger)
    /// must not move.
    pub moved: bool,
}

/// Replays an event log to a state, for checking the cached column.
///
/// A mismatch between this and `order_intents.state` means the cache and the
/// history disagree, which is corruption — the log wins, because it is what
/// actually happened.
pub fn replay_state(events: &[Event]) -> Result<OrderIntentState, TransitionError> {
    kis_client::order_state::replay(OrderIntentState::IntentCreated, events)
}

/// Rebuilds the machine's state from a row.
fn state_from_row(row: &OrderIntentRow) -> Option<OrderIntentState> {
    let broker = row.broker_order_no.clone();
    let reason = row.state_reason.clone().unwrap_or_default();
    Some(match row.state.as_str() {
        "INTENT_CREATED" => OrderIntentState::IntentCreated,
        "RISK_APPROVED" => OrderIntentState::RiskApproved,
        "SUBMITTING" => OrderIntentState::Submitting,
        "SUBMITTED" => OrderIntentState::Submitted,
        "UNKNOWN" => OrderIntentState::Unknown,
        "REJECTED" => OrderIntentState::Rejected { reason },
        "DENIED" => OrderIntentState::Denied { reason },
        "ACCEPTED" => OrderIntentState::Accepted {
            broker_order_no: broker?,
        },
        "PARTIALLY_FILLED" => OrderIntentState::PartiallyFilled {
            broker_order_no: broker?,
            cumulative_filled: whole_units(&row.cumulative_filled)?,
        },
        "FILLED" => OrderIntentState::Filled {
            broker_order_no: broker?,
        },
        "CANCELED" => OrderIntentState::Canceled {
            broker_order_no: broker?,
        },
        "EXPIRED" => OrderIntentState::Expired {
            broker_order_no: broker?,
        },
        _ => return None,
    })
}

/// Parses a `numeric(18,4)` string as whole units.
///
/// Quantities are whole units (`domain::QUANTITY_SCALE` is 0), so the
/// fractional part is always zero; it is dropped by taking the integer part
/// rather than by going through `f64`, which would start losing precision
/// above 2^53 and is the wrong tool for an exact value.
fn whole_units(decimal: &str) -> Option<u64> {
    decimal.split('.').next()?.parse::<u64>().ok()
}

/// The cumulative filled quantity a state implies, as a decimal string.
fn cumulative_of(state: &OrderIntentState) -> Option<String> {
    match state {
        OrderIntentState::PartiallyFilled {
            cumulative_filled, ..
        } => Some(cumulative_filled.to_string()),
        _ => None,
    }
}

fn reason_of(state: &OrderIntentState) -> Option<String> {
    match state {
        OrderIntentState::Rejected { reason } | OrderIntentState::Denied { reason } => {
            Some(reason.clone())
        }
        _ => None,
    }
}
