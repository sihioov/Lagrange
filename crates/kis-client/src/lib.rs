//! `kis-client` — Lagrange Station KIS broker adapter (Phase 3, Owner-only).
//!
//! Module layout follows design §6.12. Two rules shape the whole crate and are
//! enforced by types rather than by discipline:
//!
//! * **Nothing sensitive can be rendered.** [`secret::Secret`] and
//!   [`secret::AccountNo`] have no non-redacting `Debug`/`Display`, so a
//!   derived `Debug`, a `format!`, or a log line cannot leak them. Broker
//!   payloads reach an audit record only through [`error::redact_payload`].
//! * **A mutation is never blindly retried, and a submit timeout is not a
//!   failure.** [`error::KisError::is_retryable`] requires an
//!   [`error::RequestKind`], and a timed-out submit becomes
//!   [`error::KisError::Ambiguous`] — resolvable only by querying order state.
//!
//! Live credentials are never inlined in configuration: config carries a
//! [`secret::CredentialRef`] naming where the value lives.

pub mod auth;
pub mod clock;
pub mod error;
pub mod execution;
pub mod idempotency;
pub mod mapping;
pub mod order_state;
pub mod rate_limit;
pub mod rest;
pub mod retry;
pub mod secret;
pub mod simulator;
pub mod transport;
pub mod websocket;

pub use auth::{AccessToken, TokenIssuer, TokenManager};
pub use clock::{Clock, SystemClock, check_skew};
pub use error::{KisError, RequestKind};
pub use execution::{Applied, ExecutionReport, ExecutionTracker};
pub use idempotency::{Claim, InMemoryIntentStore, IntentState, IntentStore, guard_submission};
pub use mapping::{InstrumentMapper, OrderAck, OrderRequest, OrderSide, OrderType};
pub use rate_limit::{BucketKey, Permit, Quota, RateLimiter};
pub use rest::{Profile, RestClient, SubmitError};
pub use retry::{RetryPolicy, Sleeper, TokioSleeper};
pub use secret::{AccountNo, CredentialError, CredentialRef, CredentialSource, Secret};
pub use simulator::{BrokerSimulator, SIMULATOR_CONTRACT_VERSION, Scenario};
pub use transport::{HttpRequest, HttpResponse, Transport};
pub use websocket::{ReconnectPolicy, ResyncRequired, SessionState, WebSocketSession};
