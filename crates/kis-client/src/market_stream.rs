//! Market-only KIS WebSocket transport.
//!
//! This is deliberately separate from [`crate::websocket`], whose recovery
//! contract is account reconciliation.  The transport below speaks only the
//! approved `H0STCNT0` channel and keeps all endpoint injection behind a
//! loopback check used by synthetic tests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::Value;
use sha1_smol::Sha1;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::clock::{Clock, SystemClock};
use crate::market_stream_approval::{ApprovalClient, ApprovalError};
use crate::market_stream_state::{
    BudgetError, CommandBudget, ConnectionEpoch, MarketStreamLock, ReconnectPolicy, StateError,
    WsStateStore,
};
use crate::market_stream_wire::{
    MAX_MESSAGE_BYTES, MarketReceipt, MarketStreamSessionProof, MarketTradeObservation,
    ParseContext, ParsedMarketRecord, TR_ID, WireError, parse_market_message,
};

const PRODUCTION_WS_URL: &str = "ws://ops.koreainvestment.com:21000/tryitout";
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ACTIVE_SUBSCRIPTIONS: usize = 30;
const MAX_QUEUED_EVENTS: usize = 128;
const MAX_INCOMING_CONTROL_MESSAGES_PER_SECOND: usize = 10;
const MAX_INCOMING_CONTROL_BURST: f64 = 20.0;
const MAX_INCOMING_DATA_MESSAGES_PER_SECOND: f64 = 2_000.0;
const MAX_INCOMING_DATA_BURST: f64 = 4_000.0;
const MAX_FRAGMENTED_MESSAGE_FRAMES: usize = 64;
const MAX_AVAILABLE_READS_PER_DRAIN: usize = 64;

// Copied in the adopted canonical order from the owner fixed-stock universe.
// WS-2 owns only the transport admission check; the later producer still
// supplies READY/rights evidence.  Keeping this literal here avoids a
// dependency cycle to the market-data crate.
const APPROVED_MARKET_SYMBOLS: [&str; 30] = [
    "005930", "000660", "373220", "207940", "005380", "000270", "105560", "055550", "068270",
    "035420", "035720", "005490", "051910", "006400", "012330", "028260", "012450", "329180",
    "034020", "015760", "017670", "030200", "066570", "009150", "096770", "036570", "090430",
    "011200", "003490", "000810",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketStreamError {
    #[error(transparent)]
    Approval(#[from] ApprovalError),
    #[error("market stream endpoint is not allowed")]
    EndpointNotAllowed,
    #[error("market stream transport I/O failed")]
    Io,
    #[error("market stream WebSocket handshake failed")]
    Handshake,
    #[error("market stream WebSocket framing failed")]
    FrameInvalid,
    #[error("market stream protocol message is invalid")]
    Protocol,
    #[error("market stream socket is closed")]
    Closed,
    #[error("market stream command is already pending")]
    CommandPending,
    #[error("market stream command capability does not belong to this session")]
    CommandCapabilityMismatch,
    #[error("market stream command reservation timing is invalid")]
    CommandTimingInvalid,
    #[error("market stream command deadline cannot be represented")]
    CommandTimingOverflow,
    #[error("market stream command acknowledgement timed out")]
    AckTimeout,
    #[error("market stream control write timed out")]
    ControlWriteTimeout,
    #[error("market stream command acknowledgement did not match the pending operation")]
    AckMismatch,
    #[error("market stream command acknowledgement was duplicated or unexpected")]
    DuplicateAck,
    #[error("market stream command acknowledgement was rejected")]
    CommandRejected,
    #[error("market stream data arrived before its subscription was acknowledged")]
    DataBeforeAck,
    #[error("market stream symbol is not an admitted six-digit KRX identity")]
    SymbolNotAllowed,
    #[error("market stream symbol cannot be resubscribed in the current socket epoch")]
    ResubscribeRequiresNewEpoch,
    #[error("market stream subscription capacity is exhausted")]
    SubscriptionLimit,
    #[error("market stream command budget is exhausted")]
    CommandBudget(#[from] BudgetError),
    #[error("market stream reconnect budget is exhausted")]
    ReconnectBudget(#[source] BudgetError),
    #[error("market stream reconnect budget is exhausted")]
    ReconnectExhausted,
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("market stream cancellation requested")]
    Cancelled,
    #[error("market stream requires a typed current session proof")]
    SessionProofRequired,
    #[error("market stream session proof is not current")]
    SessionProofInvalid,
}

#[derive(Clone, PartialEq, Eq)]
pub enum MarketStreamEndpoint {
    Production,
    #[cfg(any(feature = "test-support", test))]
    Loopback(LoopbackEndpoint),
}

#[cfg(any(feature = "test-support", test))]
#[derive(Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct LoopbackEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl fmt::Debug for MarketStreamEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Production => f.write_str("MarketStreamEndpoint::Production"),
            #[cfg(any(feature = "test-support", test))]
            Self::Loopback(LoopbackEndpoint { host, port, path }) => f
                .debug_struct("MarketStreamEndpoint::Loopback")
                .field("host", host)
                .field("port", port)
                .field("path", path)
                .finish(),
        }
    }
}

impl MarketStreamEndpoint {
    pub fn production() -> Self {
        Self::Production
    }

    /// Accept only an explicit loopback `ws://host:port/path` endpoint.  This
    /// is the only injectable endpoint constructor in test-support builds.
    #[cfg(any(feature = "test-support", test))]
    pub fn loopback(url: &str) -> Result<Self, MarketStreamError> {
        let parsed = reqwest::Url::parse(url).map_err(|_| MarketStreamError::EndpointNotAllowed)?;
        if parsed.scheme() != "ws"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.username() != ""
            || parsed.password().is_some()
            || !matches!(
                parsed.host_str(),
                Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
            )
            || parsed.port().is_none()
            || parsed.path() != "/tryitout"
        {
            return Err(MarketStreamError::EndpointNotAllowed);
        }
        Ok(Self::Loopback(LoopbackEndpoint {
            host: parsed.host_str().unwrap_or_default().to_owned(),
            port: parsed.port().ok_or(MarketStreamError::EndpointNotAllowed)?,
            path: parsed.path().to_owned(),
        }))
    }

    fn parts(&self) -> (&str, u16, &str) {
        match self {
            Self::Production => ("ops.koreainvestment.com", 21_000, "/tryitout"),
            #[cfg(any(feature = "test-support", test))]
            Self::Loopback(LoopbackEndpoint { host, port, path }) => (host, *port, path),
        }
    }

    pub fn literal(&self) -> &'static str {
        match self {
            Self::Production => PRODUCTION_WS_URL,
            #[cfg(any(feature = "test-support", test))]
            Self::Loopback(_) => "loopback://127.0.0.1",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProductionMarketStreamEndpoint;

impl ProductionMarketStreamEndpoint {
    pub const URL: &'static str = PRODUCTION_WS_URL;
}

#[derive(Clone)]
pub struct MarketStreamConfig {
    pub(crate) approval: ApprovalClient,
    pub(crate) state: WsStateStore,
    pub(crate) endpoint: MarketStreamEndpoint,
    pub(crate) ack_timeout: Duration,
    pub(crate) session_proof: Option<MarketStreamSessionProof>,
    pub(crate) reconnect_policy: ReconnectPolicy,
    pub(crate) clock: Arc<dyn Clock>,
    write_barrier: Option<Arc<Notify>>,
}

impl fmt::Debug for MarketStreamConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketStreamConfig")
            .field("endpoint", &self.endpoint)
            .field("ack_timeout", &self.ack_timeout)
            .field("reconnect_policy", &self.reconnect_policy)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl MarketStreamConfig {
    pub fn new(approval: ApprovalClient) -> Result<Self, MarketStreamError> {
        let state = approval.domain().state().clone();
        Ok(Self {
            approval,
            state,
            endpoint: MarketStreamEndpoint::production(),
            ack_timeout: Duration::from_secs(5),
            session_proof: None,
            reconnect_policy: ReconnectPolicy::default(),
            clock: Arc::new(SystemClock),
            write_barrier: None,
        })
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn loopback(approval: ApprovalClient, endpoint: &str) -> Result<Self, MarketStreamError> {
        let mut config = Self::new(approval)?;
        config.endpoint = MarketStreamEndpoint::loopback(endpoint)?;
        config.session_proof = Some(MarketStreamSessionProof::synthetic_default());
        config.clock = Arc::new(crate::clock::TestClock::at(
            1_789_916_400_000 + 10 * 60 * 60 * 1_000,
        ));
        Ok(config)
    }

    pub fn with_session_proof(mut self, proof: MarketStreamSessionProof) -> Self {
        self.session_proof = Some(proof);
        self
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn with_ack_timeout(mut self, timeout: Duration) -> Self {
        self.ack_timeout = timeout;
        self
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    #[cfg(any(feature = "test-support", test))]
    pub fn with_test_write_barrier(mut self, barrier: Arc<Notify>) -> Self {
        self.write_barrier = Some(barrier);
        self
    }
}

#[derive(Clone)]
pub struct MarketStreamClient {
    config: MarketStreamConfig,
}

impl fmt::Debug for MarketStreamClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketStreamClient")
            .field("config", &self.config)
            .finish()
    }
}

impl MarketStreamClient {
    pub fn new(config: MarketStreamConfig) -> Self {
        Self { config }
    }

    pub fn production(approval: ApprovalClient) -> Result<Self, MarketStreamError> {
        Ok(Self::new(MarketStreamConfig::new(approval)?))
    }

    pub fn config(&self) -> &MarketStreamConfig {
        &self.config
    }

    /// Reserve the existing connection anchor before a caller performs work
    /// that must precede approval and socket creation.
    ///
    /// This validates the configured durable domain and holds the same
    /// lifetime anchor that a connected session will own. It does not reserve
    /// an attempt or contact the approval service.
    pub fn reserve_connection(&self) -> Result<MarketStreamConnectionOwner, MarketStreamError> {
        self.config.state.load()?;
        let lifetime_lock = self.config.state.lock()?;
        Ok(MarketStreamConnectionOwner {
            config: self.config.clone(),
            lifetime_lock: Some(lifetime_lock),
        })
    }

    pub async fn connect(&self) -> Result<MarketStreamSession, MarketStreamError> {
        let proof = current_session_proof(&self.config)?;
        self.reserve_connection()?.connect(proof).await
    }
}

/// Affine ownership of the existing market-stream connection anchor.
///
/// The value can be dropped without an attempt. Its only consuming operation
/// turns that same anchor into a connected session.
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// let _ = MarketStreamConnectionOwner {};
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// let _ = MarketStreamConnectionOwner::new();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MarketStreamConnectionOwner>();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// fn require_deserialize<T: serde::de::DeserializeOwned>() {}
/// require_deserialize::<MarketStreamConnectionOwner>();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<MarketStreamConnectionOwner>();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// fn expose_lock(owner: &MarketStreamConnectionOwner) {
///     let _ = owner.lifetime_lock();
/// }
/// ```
///
/// ```compile_fail
/// use kis_client::MarketStreamConnectionOwner;
/// fn expose_proof(owner: &MarketStreamConnectionOwner) {
///     let _ = owner.session_proof();
/// }
/// ```
#[must_use]
pub struct MarketStreamConnectionOwner {
    config: MarketStreamConfig,
    lifetime_lock: Option<MarketStreamLock>,
}

impl MarketStreamConnectionOwner {
    /// Consume the reservation, validate the supplied proof, and transfer the
    /// held lifetime anchor into the resulting session.
    pub async fn connect(
        mut self,
        proof: MarketStreamSessionProof,
    ) -> Result<MarketStreamSession, MarketStreamError> {
        validate_session_proof(&self.config, proof)?;
        self.config.session_proof = Some(proof);
        let permit = self.config.state.begin_connection_attempt(
            self.config.clock.now_ms(),
            false,
            self.config.reconnect_policy,
        )?;
        let approval_key = approval_key_for_attempt(&self.config, permit.attempt_id).await?;
        let mut socket = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            RawWebSocket::connect(&self.config.endpoint, self.config.write_barrier.clone()),
        )
        .await
        .map_err(|_| MarketStreamError::Handshake)??;
        let epoch = ConnectionEpoch::new();
        if let Err(error) = self
            .config
            .state
            .complete_connection(permit.attempt_id, epoch)
        {
            socket.poisoned = true;
            return Err(error.into());
        }
        let socket_open_ms = self.config.clock.now_ms();
        let lifetime_lock = self.lifetime_lock.take().ok_or(MarketStreamError::Closed)?;
        Ok(MarketStreamSession {
            socket: Some(socket),
            lifetime_lock: Some(lifetime_lock),
            config: self.config.clone(),
            approval_key,
            credential_slot_id: self.config.approval.domain().credential_slot_id(),
            session_nonce: Uuid::new_v4(),
            epoch,
            proof,
            socket_open_ms,
            subscriptions: HashSet::new(),
            unsubscribed_in_epoch: HashSet::new(),
            pending: None,
            queued_events: VecDeque::new(),
            receive_ordinal: 0,
            last_observations: HashMap::new(),
            last_inbound: Instant::now(),
            last_control_liveness: Instant::now(),
            ping_sent: None,
            command_phase: CommandPhase::Idle,
            control_tokens: MAX_INCOMING_CONTROL_BURST,
            control_token_time: Instant::now(),
            data_tokens: MAX_INCOMING_DATA_BURST,
            data_token_time: Instant::now(),
            closed: false,
            peer_closed_cleanly: false,
            #[cfg(test)]
            socketless_command_test: false,
        })
    }
}

fn current_session_proof(
    config: &MarketStreamConfig,
) -> Result<MarketStreamSessionProof, MarketStreamError> {
    let proof = config
        .session_proof
        .ok_or(MarketStreamError::SessionProofRequired)?;
    validate_session_proof(config, proof)?;
    Ok(proof)
}

fn validate_session_proof(
    config: &MarketStreamConfig,
    proof: MarketStreamSessionProof,
) -> Result<(), MarketStreamError> {
    if matches!(config.endpoint, MarketStreamEndpoint::Production)
        && proof.validate_current_day(config.clock.now_ms()).is_err()
    {
        return Err(MarketStreamError::SessionProofInvalid);
    }
    Ok(())
}

async fn approval_key_for_attempt(
    config: &MarketStreamConfig,
    attempt_id: Uuid,
) -> Result<crate::market_stream_approval::ApprovalKey, MarketStreamError> {
    let result = config.approval.key_for_connection().await;
    if let Err(error) = &result
        && matches!(
            error,
            ApprovalError::HttpRejected { .. }
                | ApprovalError::AttemptTooSoon
                | ApprovalError::AttemptBudgetExhausted
                | ApprovalError::CredentialGenerationMismatch
                | ApprovalError::Expired
        )
    {
        config.state.clear_connection_attempt(attempt_id)?;
    }
    result.map_err(MarketStreamError::from)
}

#[derive(Debug, Clone)]
pub enum MarketStreamEvent {
    Receipt(MarketReceipt),
    ApplicationHeartbeat,
    ControlPong,
    Status {
        code: StreamStatusCode,
        epoch: ConnectionEpoch,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStatusCode {
    Connected,
    Closed,
    Gap,
    DataBeforeAck,
    DuplicateObservation,
    StaleObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketSubscriptionOperation {
    Subscribe,
    Unsubscribe,
}

impl MarketSubscriptionOperation {
    fn as_wire_name(self) -> &'static str {
        match self {
            Self::Subscribe => "subscribe",
            Self::Unsubscribe => "unsubscribe",
        }
    }
}

/// A unique reservation for one real transport command.
///
/// Metadata getters are not proof by themselves. Only the session which minted
/// this value can consume it with `send_prepared`.
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// let _ = PreparedMarketSubscriptionCommand {};
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// let _ = PreparedMarketSubscriptionCommand::new();
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// fn mutate(command: &mut PreparedMarketSubscriptionCommand) {
///     command.ordinal = 4;
/// }
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// fn require_clone<T: Clone>() {}
/// require_clone::<PreparedMarketSubscriptionCommand>();
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// let _: PreparedMarketSubscriptionCommand = Default::default();
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<PreparedMarketSubscriptionCommand>();
/// ```
///
/// ```compile_fail
/// use kis_client::PreparedMarketSubscriptionCommand;
/// fn require_deserialize<T: serde::de::DeserializeOwned>() {}
/// require_deserialize::<PreparedMarketSubscriptionCommand>();
/// ```
///
/// ```compile_fail
/// use kis_client::market_stream::MarketStreamEndpoint;
/// let _ = MarketStreamEndpoint::loopback("ws://127.0.0.1:3000/tryitout");
/// ```
///
/// ```compile_fail
/// use kis_client::market_stream::MarketStreamConfig;
/// let _ = MarketStreamConfig::loopback;
/// ```
///
/// ```compile_fail
/// use kis_client::market_stream_state::MarketStreamDomain;
/// let _ = MarketStreamDomain::for_test;
/// ```
#[must_use]
pub struct PreparedMarketSubscriptionCommand {
    credential_slot_id: Uuid,
    session_nonce: Uuid,
    epoch: ConnectionEpoch,
    symbol: String,
    operation: MarketSubscriptionOperation,
    ordinal: u64,
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
}

impl fmt::Debug for PreparedMarketSubscriptionCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedMarketSubscriptionCommand")
            .field("credential_slot_id", &self.credential_slot_id)
            .field("epoch", &self.epoch)
            .field("symbol", &self.symbol)
            .field("operation", &self.operation)
            .field("ordinal", &self.ordinal)
            .field("reserved_at_ms", &self.reserved_at_ms)
            .field("deadline_at_ms", &self.deadline_at_ms)
            .finish()
    }
}

impl PreparedMarketSubscriptionCommand {
    pub fn credential_slot_id(&self) -> Uuid {
        self.credential_slot_id
    }

    pub fn epoch(&self) -> ConnectionEpoch {
        self.epoch
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn operation(&self) -> MarketSubscriptionOperation {
        self.operation
    }

    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }

    pub fn reserved_at_ms(&self) -> i64 {
        self.reserved_at_ms
    }

    pub fn reserved_at_monotonic(&self) -> Instant {
        self.reserved_at_monotonic
    }

    pub fn deadline_at_ms(&self) -> i64 {
        self.deadline_at_ms
    }

    pub fn deadline_monotonic(&self) -> Instant {
        self.deadline_monotonic
    }
}

/// Opaque evidence that one matching command received and committed a timely
/// transport ACK. Its copied metadata cannot be used as a substitute for it.
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// let _ = MarketSubscriptionAck {};
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// let _ = MarketSubscriptionAck::new();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// fn mutate(ack: &mut MarketSubscriptionAck) {
///     ack.ordinal = 4;
/// }
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MarketSubscriptionAck>();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// let _: MarketSubscriptionAck = Default::default();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<MarketSubscriptionAck>();
/// ```
///
/// ```compile_fail
/// use kis_client::MarketSubscriptionAck;
/// fn require_deserialize<T: serde::de::DeserializeOwned>() {}
/// require_deserialize::<MarketSubscriptionAck>();
/// ```
#[must_use]
pub struct MarketSubscriptionAck {
    credential_slot_id: Uuid,
    epoch: ConnectionEpoch,
    symbol: String,
    operation: MarketSubscriptionOperation,
    ordinal: u64,
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
    ack_received_at_ms: i64,
    ack_received_monotonic: Instant,
}

impl fmt::Debug for MarketSubscriptionAck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketSubscriptionAck")
            .field("credential_slot_id", &self.credential_slot_id)
            .field("epoch", &self.epoch)
            .field("symbol", &self.symbol)
            .field("operation", &self.operation)
            .field("ordinal", &self.ordinal)
            .field("reserved_at_ms", &self.reserved_at_ms)
            .field("deadline_at_ms", &self.deadline_at_ms)
            .field("ack_received_at_ms", &self.ack_received_at_ms)
            .finish()
    }
}

impl MarketSubscriptionAck {
    pub fn credential_slot_id(&self) -> Uuid {
        self.credential_slot_id
    }

    pub fn epoch(&self) -> ConnectionEpoch {
        self.epoch
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn operation(&self) -> MarketSubscriptionOperation {
        self.operation
    }

    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }

    pub fn reserved_at_ms(&self) -> i64 {
        self.reserved_at_ms
    }

    pub fn reserved_at_monotonic(&self) -> Instant {
        self.reserved_at_monotonic
    }

    pub fn deadline_at_ms(&self) -> i64 {
        self.deadline_at_ms
    }

    pub fn deadline_monotonic(&self) -> Instant {
        self.deadline_monotonic
    }

    pub fn ack_received_at_ms(&self) -> i64 {
        self.ack_received_at_ms
    }

    pub fn ack_received_monotonic(&self) -> Instant {
        self.ack_received_monotonic
    }
}

struct PendingCommand {
    operation: MarketSubscriptionOperation,
    symbol: String,
    epoch: ConnectionEpoch,
    ordinal: u64,
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
    ack_capture: Option<AckCapture>,
}

#[derive(Clone, Copy)]
struct AckCapture {
    received_at_ms: i64,
    received_monotonic: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandPhase {
    Idle,
    Prepared,
    WriteStarted,
    Sent,
    AckObserved,
    Poisoned,
}

#[derive(Debug, Clone, Copy)]
struct CommandTiming {
    reserved_at_ms: i64,
    reserved_at_monotonic: Instant,
    deadline_at_ms: i64,
    deadline_monotonic: Instant,
}

impl CommandTiming {
    fn reserve(
        reserved_at_ms: i64,
        reserved_at_monotonic: Instant,
        timeout: Duration,
    ) -> Result<Self, MarketStreamError> {
        let deadline_monotonic = reserved_at_monotonic
            .checked_add(timeout)
            .ok_or(MarketStreamError::CommandTimingOverflow)?;
        let timeout_ms = i64::try_from(timeout.as_millis())
            .map_err(|_| MarketStreamError::CommandTimingOverflow)?;
        let deadline_at_ms = reserved_at_ms
            .checked_add(timeout_ms)
            .ok_or(MarketStreamError::CommandTimingOverflow)?;
        Ok(Self {
            reserved_at_ms,
            reserved_at_monotonic,
            deadline_at_ms,
            deadline_monotonic,
        })
    }

    fn check(self, wall_now_ms: i64, monotonic_now: Instant) -> Result<(), MarketStreamError> {
        let wall_valid = wall_now_ms >= self.reserved_at_ms && wall_now_ms < self.deadline_at_ms;
        let monotonic_valid =
            monotonic_now >= self.reserved_at_monotonic && monotonic_now < self.deadline_monotonic;
        if wall_valid && monotonic_valid {
            return Ok(());
        }
        if wall_valid != monotonic_valid
            || wall_now_ms < self.reserved_at_ms
            || monotonic_now < self.reserved_at_monotonic
        {
            return Err(MarketStreamError::CommandTimingInvalid);
        }
        Err(MarketStreamError::AckTimeout)
    }

    fn remaining(self, monotonic_now: Instant) -> Duration {
        self.deadline_monotonic
            .saturating_duration_since(monotonic_now)
    }

    fn from_pending(pending: &PendingCommand) -> Self {
        Self {
            reserved_at_ms: pending.reserved_at_ms,
            reserved_at_monotonic: pending.reserved_at_monotonic,
            deadline_at_ms: pending.deadline_at_ms,
            deadline_monotonic: pending.deadline_monotonic,
        }
    }
}

enum IncomingOutcome {
    Continue,
    Event(MarketStreamEvent),
    Terminal(MarketStreamEvent),
}

pub struct MarketStreamSession {
    socket: Option<RawWebSocket>,
    lifetime_lock: Option<MarketStreamLock>,
    config: MarketStreamConfig,
    approval_key: crate::market_stream_approval::ApprovalKey,
    credential_slot_id: Uuid,
    session_nonce: Uuid,
    epoch: ConnectionEpoch,
    proof: MarketStreamSessionProof,
    socket_open_ms: i64,
    subscriptions: HashSet<String>,
    unsubscribed_in_epoch: HashSet<String>,
    pending: Option<PendingCommand>,
    queued_events: VecDeque<MarketStreamEvent>,
    receive_ordinal: u64,
    last_observations: HashMap<String, MarketTradeObservation>,
    last_inbound: Instant,
    last_control_liveness: Instant,
    ping_sent: Option<(Instant, Vec<u8>)>,
    command_phase: CommandPhase,
    control_tokens: f64,
    control_token_time: Instant,
    data_tokens: f64,
    data_token_time: Instant,
    closed: bool,
    peer_closed_cleanly: bool,
    #[cfg(test)]
    socketless_command_test: bool,
}

impl fmt::Debug for MarketStreamSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketStreamSession")
            .field("epoch", &self.epoch)
            .field("subscriptions", &self.subscriptions)
            .field(
                "pending",
                &self
                    .pending
                    .as_ref()
                    .map(|pending| (&pending.operation, &pending.symbol, pending.reserved_at_ms)),
            )
            .field("command_phase", &self.command_phase)
            .field("closed", &self.closed)
            .finish()
    }
}

// Run the unchanged reservation policy on a disposable copy. No attempt,
// ordinal, pending capability or durable state is created by this hint.
fn command_spacing_not_before_ms(
    attempts_ms: &[i64],
    now_ms: i64,
) -> Result<Option<i64>, MarketStreamError> {
    let mut budget = CommandBudget::from_attempts(attempts_ms.iter().copied());
    match budget.reserve(now_ms) {
        Ok(()) => Ok(None),
        Err(BudgetError::MinimumSpacing) => {
            let deadline = attempts_ms
                .last()
                .and_then(|last| last.checked_add(CommandBudget::MIN_SPACING_MS))
                .ok_or(MarketStreamError::CommandTimingOverflow)?;
            Ok(Some(deadline))
        }
        Err(error) => Err(MarketStreamError::CommandBudget(error)),
    }
}

#[cfg(any(feature = "test-support", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct MarketStreamCommandStateSnapshot {
    pub command_attempts_ms: Vec<i64>,
    pub next_command_ordinal: u64,
    pub pending_command: Option<(String, String, u64, i64, i64)>,
    pub current_epoch: Option<String>,
}

#[cfg(any(feature = "test-support", test))]
fn command_state_snapshot(
    store: &WsStateStore,
) -> Result<MarketStreamCommandStateSnapshot, MarketStreamError> {
    let state = store.load()?;
    Ok(MarketStreamCommandStateSnapshot {
        command_attempts_ms: state.command_attempts_ms,
        next_command_ordinal: state.next_command_ordinal,
        pending_command: state.pending_command.map(|pending| {
            (
                pending.operation,
                pending.symbol,
                pending.ordinal,
                pending.sent_at_ms,
                pending.deadline_ms,
            )
        }),
        current_epoch: state.current_epoch,
    })
}

#[cfg(any(feature = "test-support", test))]
impl crate::market_stream_state::MarketStreamDomain {
    #[doc(hidden)]
    pub fn test_command_state_snapshot(
        &self,
    ) -> Result<MarketStreamCommandStateSnapshot, MarketStreamError> {
        command_state_snapshot(self.state())
    }
}

impl MarketStreamSession {
    #[cfg(test)]
    fn for_command_eligibility_test(
        config: MarketStreamConfig,
        epoch: ConnectionEpoch,
        subscriptions: HashSet<String>,
    ) -> Self {
        let credential_slot_id = config.approval.domain().credential_slot_id();
        let now = Instant::now();
        Self {
            socket: None,
            lifetime_lock: None,
            config,
            approval_key: crate::market_stream_approval::ApprovalKey::new(
                "SYNTHETIC_COMMAND_ELIGIBILITY_KEY".to_owned(),
            )
            .expect("synthetic key is valid"),
            credential_slot_id,
            session_nonce: Uuid::new_v4(),
            epoch,
            proof: MarketStreamSessionProof::synthetic_default(),
            socket_open_ms: 0,
            subscriptions,
            unsubscribed_in_epoch: HashSet::new(),
            pending: None,
            queued_events: VecDeque::new(),
            receive_ordinal: 0,
            last_observations: HashMap::new(),
            last_inbound: now,
            last_control_liveness: now,
            ping_sent: None,
            command_phase: CommandPhase::Idle,
            control_tokens: MAX_INCOMING_CONTROL_BURST,
            control_token_time: now,
            data_tokens: MAX_INCOMING_DATA_BURST,
            data_token_time: now,
            closed: false,
            peer_closed_cleanly: false,
            socketless_command_test: true,
        }
    }

    pub fn epoch(&self) -> ConnectionEpoch {
        self.epoch
    }

    pub fn is_closed(&self) -> bool {
        self.closed || self.socket.as_ref().is_none_or(RawWebSocket::is_poisoned)
    }

    pub fn subscribed_symbols(&self) -> impl Iterator<Item = &str> {
        self.subscriptions.iter().map(String::as_str)
    }

    #[cfg(any(feature = "test-support", test))]
    #[doc(hidden)]
    pub fn test_command_state_snapshot(
        &self,
    ) -> Result<MarketStreamCommandStateSnapshot, MarketStreamError> {
        command_state_snapshot(&self.config.state)
    }

    pub async fn subscribe(&mut self, symbol: &str) -> Result<(), MarketStreamError> {
        if let Some(command) = self.prepare_subscribe(symbol)? {
            let _ack = self.send_prepared(command).await?;
        }
        Ok(())
    }

    pub async fn unsubscribe(&mut self, symbol: &str) -> Result<(), MarketStreamError> {
        if let Some(command) = self.prepare_unsubscribe(symbol)? {
            let _ack = self.send_prepared(command).await?;
        }
        Ok(())
    }

    pub fn prepare_subscribe(
        &mut self,
        symbol: &str,
    ) -> Result<Option<PreparedMarketSubscriptionCommand>, MarketStreamError> {
        self.prepare_command(MarketSubscriptionOperation::Subscribe, symbol)
    }

    pub fn prepare_unsubscribe(
        &mut self,
        symbol: &str,
    ) -> Result<Option<PreparedMarketSubscriptionCommand>, MarketStreamError> {
        self.prepare_command(MarketSubscriptionOperation::Unsubscribe, symbol)
    }

    fn validate_command_request(
        &mut self,
        operation: MarketSubscriptionOperation,
        symbol: &str,
    ) -> Result<bool, MarketStreamError> {
        if self.command_phase != CommandPhase::Idle {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        self.ensure_open()?;
        validate_symbol(symbol)?;
        if self.pending.is_some() {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        if operation == MarketSubscriptionOperation::Subscribe
            && self.unsubscribed_in_epoch.contains(symbol)
        {
            return Err(MarketStreamError::ResubscribeRequiresNewEpoch);
        }
        if operation == MarketSubscriptionOperation::Subscribe
            && self.subscriptions.contains(symbol)
        {
            return Ok(false);
        }
        if operation == MarketSubscriptionOperation::Unsubscribe
            && !self.subscriptions.contains(symbol)
        {
            return Ok(false);
        }
        if operation == MarketSubscriptionOperation::Subscribe
            && self.subscriptions.len() >= MAX_ACTIVE_SUBSCRIPTIONS
        {
            return Err(MarketStreamError::SubscriptionLimit);
        }
        Ok(true)
    }

    /// Non-reserving eligibility check that may terminalize on uncertainty.
    /// Some(t) requires a later explicit reconciliation at/after t; preparation
    /// still revalidates and persists the authoritative reservation.
    pub fn subscription_command_not_before_ms(
        &mut self,
        operation: MarketSubscriptionOperation,
        symbol: &str,
    ) -> Result<Option<i64>, MarketStreamError> {
        if !self.validate_command_request(operation, symbol)? {
            return Ok(None);
        }
        let state = match self.config.state.load() {
            Ok(state) => state,
            Err(error) => {
                self.poison_command_uncertainty();
                return Err(error.into());
            }
        };
        let epoch = self.epoch.uuid().to_string();
        if state.current_epoch.as_deref() != Some(epoch.as_str()) || state.pending_command.is_some()
        {
            self.poison_command_uncertainty();
            return Err(StateError::InvalidState.into());
        }
        command_spacing_not_before_ms(&state.command_attempts_ms, self.config.clock.now_ms())
    }

    fn prepare_command(
        &mut self,
        operation: MarketSubscriptionOperation,
        symbol: &str,
    ) -> Result<Option<PreparedMarketSubscriptionCommand>, MarketStreamError> {
        if !self.validate_command_request(operation, symbol)? {
            return Ok(None);
        }
        let reserved_at_ms = self.config.clock.now_ms();
        let reserved_at_monotonic = Instant::now();
        let timing = CommandTiming::reserve(
            reserved_at_ms,
            reserved_at_monotonic,
            self.config.ack_timeout,
        )?;
        match self.reserve_command_budget(reserved_at_ms) {
            Ok(()) => {}
            Err(error @ MarketStreamError::CommandBudget(_)) => return Err(error),
            Err(error) => {
                self.poison_command_uncertainty();
                return Err(error);
            }
        }
        let ordinal = match self.config.state.reserve_command(
            self.epoch,
            operation.as_wire_name(),
            symbol,
            timing.reserved_at_ms,
            timing.deadline_at_ms,
        ) {
            Ok(ordinal) => ordinal,
            Err(error) => {
                self.poison_command_uncertainty();
                return Err(error.into());
            }
        };
        self.pending = Some(PendingCommand {
            operation,
            symbol: symbol.to_owned(),
            epoch: self.epoch,
            ordinal,
            reserved_at_ms: timing.reserved_at_ms,
            reserved_at_monotonic: timing.reserved_at_monotonic,
            deadline_at_ms: timing.deadline_at_ms,
            deadline_monotonic: timing.deadline_monotonic,
            ack_capture: None,
        });
        self.command_phase = CommandPhase::Prepared;
        Ok(Some(PreparedMarketSubscriptionCommand {
            credential_slot_id: self.credential_slot_id,
            session_nonce: self.session_nonce,
            epoch: self.epoch,
            symbol: symbol.to_owned(),
            operation,
            ordinal,
            reserved_at_ms: timing.reserved_at_ms,
            reserved_at_monotonic: timing.reserved_at_monotonic,
            deadline_at_ms: timing.deadline_at_ms,
            deadline_monotonic: timing.deadline_monotonic,
        }))
    }

    pub async fn send_prepared(
        &mut self,
        command: PreparedMarketSubscriptionCommand,
    ) -> Result<MarketSubscriptionAck, MarketStreamError> {
        if command.session_nonce != self.session_nonce
            || command.credential_slot_id != self.credential_slot_id
            || command.epoch != self.epoch
        {
            return Err(MarketStreamError::CommandCapabilityMismatch);
        }
        if self.command_phase != CommandPhase::Prepared {
            self.poison_epoch();
            return Err(MarketStreamError::CommandCapabilityMismatch);
        }
        if let Err(error) = self.ensure_open() {
            self.poison_epoch();
            return Err(error);
        }
        let matches_pending = self.pending.as_ref().is_some_and(|pending| {
            command.epoch == pending.epoch
                && command.symbol == pending.symbol
                && command.operation == pending.operation
                && command.ordinal == pending.ordinal
                && command.reserved_at_ms == pending.reserved_at_ms
                && command.reserved_at_monotonic == pending.reserved_at_monotonic
                && command.deadline_at_ms == pending.deadline_at_ms
                && command.deadline_monotonic == pending.deadline_monotonic
        });
        if !matches_pending {
            self.poison_epoch();
            return Err(MarketStreamError::CommandCapabilityMismatch);
        }

        let wire_command = Command::new(command.operation, &command.symbol, &self.approval_key);
        let wire_bytes = wire_command.json();
        let pre_send_remaining =
            CommandTiming::from_pending(self.pending.as_ref().unwrap()).remaining(Instant::now());
        if pre_send_remaining.is_zero() {
            self.poison_epoch();
            return Err(MarketStreamError::AckTimeout);
        }
        match tokio::time::timeout(
            pre_send_remaining,
            self.process_buffered_control_before_emit(),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.poison_epoch();
                return Err(error);
            }
            Err(_) => {
                self.poison_epoch();
                return Err(MarketStreamError::AckTimeout);
            }
        }
        if self.command_phase != CommandPhase::Prepared {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        let timing = CommandTiming::from_pending(self.pending.as_ref().unwrap());
        if let Err(error) = timing.check(self.config.clock.now_ms(), Instant::now()) {
            self.poison_epoch();
            return Err(error);
        }

        self.command_phase = CommandPhase::WriteStarted;
        let send_result = match self.socket_mut() {
            Ok(socket) => socket.send_text(wire_bytes.as_bytes()).await,
            Err(error) => Err(error),
        };
        if let Err(error) = send_result {
            self.poison_epoch();
            return Err(error);
        }
        self.command_phase = CommandPhase::Sent;
        if let Err(error) = self.wait_for_ack().await {
            self.poison_epoch();
            return Err(error);
        }
        if self.command_phase != CommandPhase::AckObserved {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        }
        if let Err(error) = self.process_buffered_control_before_emit().await {
            self.poison_epoch();
            return Err(error);
        }
        if self.command_phase != CommandPhase::AckObserved {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        }
        let Some(pending) = self.pending.as_ref() else {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        };
        let Some(capture) = pending.ack_capture else {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        };
        let epoch = pending.epoch;
        let ordinal = pending.ordinal;
        let operation = pending.operation;
        let symbol = pending.symbol.clone();
        let reserved_at_ms = pending.reserved_at_ms;
        let reserved_at_monotonic = pending.reserved_at_monotonic;
        let deadline_at_ms = pending.deadline_at_ms;
        let deadline_monotonic = pending.deadline_monotonic;
        if let Err(error) = self.config.state.clear_pending_command(self.epoch, ordinal) {
            self.poison_epoch();
            return Err(error.into());
        }
        match operation {
            MarketSubscriptionOperation::Subscribe => {
                self.subscriptions.insert(symbol.clone());
            }
            MarketSubscriptionOperation::Unsubscribe => {
                self.subscriptions.remove(&symbol);
                self.unsubscribed_in_epoch.insert(symbol.clone());
            }
        }
        self.pending = None;
        self.command_phase = CommandPhase::Idle;
        Ok(MarketSubscriptionAck {
            credential_slot_id: self.credential_slot_id,
            epoch,
            symbol,
            operation,
            ordinal,
            reserved_at_ms,
            reserved_at_monotonic,
            deadline_at_ms,
            deadline_monotonic,
            ack_received_at_ms: capture.received_at_ms,
            ack_received_monotonic: capture.received_monotonic,
        })
    }

    async fn wait_for_ack(&mut self) -> Result<(), MarketStreamError> {
        loop {
            let Some(pending) = self.pending.as_ref() else {
                return Err(MarketStreamError::AckMismatch);
            };
            if pending.ack_capture.is_some() {
                return Ok(());
            }
            let remaining = CommandTiming::from_pending(pending).remaining(Instant::now());
            if remaining.is_zero() {
                return Err(MarketStreamError::AckTimeout);
            }
            let outcome = match tokio::time::timeout(remaining, async {
                let frame = self.read_next_message().await?;
                self.process_incoming(frame).await
            })
            .await
            {
                Ok(result) => result?,
                Err(_) => return Err(MarketStreamError::AckTimeout),
            };
            match outcome {
                IncomingOutcome::Continue | IncomingOutcome::Event(_) => {}
                IncomingOutcome::Terminal(_) => return Err(MarketStreamError::Closed),
            }
            if self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.ack_capture.is_some())
            {
                return Ok(());
            }
        }
    }

    fn apply_ack(&mut self, bytes: &[u8], capture: AckCapture) -> Result<(), MarketStreamError> {
        if self.command_phase != CommandPhase::Sent {
            self.poison_epoch();
            return Err(MarketStreamError::DuplicateAck);
        }
        let Some(pending) = self.pending.as_ref() else {
            self.poison_epoch();
            return Err(MarketStreamError::DuplicateAck);
        };
        if pending.ack_capture.is_some() {
            self.poison_epoch();
            return Err(MarketStreamError::DuplicateAck);
        }
        if let Err(error) = CommandTiming::from_pending(pending)
            .check(capture.received_at_ms, capture.received_monotonic)
        {
            self.poison_epoch();
            return Err(error);
        }
        let Some(ack) = parse_ack(bytes) else {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        };
        if pending.epoch != self.epoch
            || ack.tr_id != TR_ID
            || ack.tr_key != pending.symbol
            || ack.encrypt != "N"
        {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        }
        if ack.rt_cd != "0" || ack.msg_cd != "OPSP0000" {
            self.poison_epoch();
            return Err(MarketStreamError::CommandRejected);
        }
        let matches_operation = match pending.operation {
            MarketSubscriptionOperation::Subscribe => {
                ack.msg1.as_deref() == Some("SUBSCRIBE SUCCESS")
            }
            MarketSubscriptionOperation::Unsubscribe => ack
                .msg1
                .as_deref()
                .is_some_and(|msg| msg.starts_with("UNSUB")),
        };
        if !matches_operation {
            self.poison_epoch();
            return Err(MarketStreamError::AckMismatch);
        }
        self.pending.as_mut().unwrap().ack_capture = Some(capture);
        self.command_phase = CommandPhase::AckObserved;
        Ok(())
    }

    pub async fn next_event(&mut self) -> Result<MarketStreamEvent, MarketStreamError> {
        self.next_event_inner(None)
            .await?
            .ok_or(MarketStreamError::Closed)
    }

    /// Return the next event, or `None` when the bounded read reaches its
    /// deadline. Partial frame, fragment, and socket read buffers stay owned
    /// by this session and are resumed by a later call.
    pub async fn next_event_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<MarketStreamEvent>, MarketStreamError> {
        self.next_event_inner(Some(deadline)).await
    }

    async fn next_event_inner(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<Option<MarketStreamEvent>, MarketStreamError> {
        if self.command_phase != CommandPhase::Idle {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        self.ensure_open()?;
        // Drain a bounded amount of already-buffered control before releasing
        // an earlier packed receipt.  This detects poison already received on
        // the socket without waiting for future network bytes.
        if !self.queued_events.is_empty() {
            self.process_buffered_control_before_emit().await?;
        }
        if let Some(event) = self.queued_events.pop_front() {
            return Ok(Some(event));
        }
        self.watchdog_check().await?;
        loop {
            let message = match deadline {
                Some(deadline) => self.read_next_message_until(deadline).await?,
                None => Some(self.read_next_message().await?),
            };
            let Some(message) = message else {
                return Ok(None);
            };
            match self.process_incoming(message).await? {
                IncomingOutcome::Continue => {
                    if !self.queued_events.is_empty() {
                        self.process_buffered_control_before_emit().await?;
                    }
                    if let Some(event) = self.queued_events.pop_front() {
                        return Ok(Some(event));
                    }
                }
                IncomingOutcome::Event(event) | IncomingOutcome::Terminal(event) => {
                    return Ok(Some(event));
                }
            }
            self.watchdog_check().await?;
        }
    }

    fn queue_observations(
        &mut self,
        records: Vec<ParsedMarketRecord>,
        received_at_ms: i64,
        received_monotonic: Instant,
    ) -> Result<(), MarketStreamError> {
        let regular_count = records
            .iter()
            .filter(|record| matches!(record, ParsedMarketRecord::Regular(_)))
            .count();
        if self.queued_events.len().saturating_add(regular_count) > MAX_QUEUED_EVENTS {
            self.closed = true;
            return Err(MarketStreamError::Protocol);
        }
        for record in records {
            let ParsedMarketRecord::Regular(observation) = record else {
                continue;
            };
            if !self.symbol_is_admitted_for_current_frame(&observation.symbol) {
                self.enqueue_event(MarketStreamEvent::Status {
                    code: StreamStatusCode::DataBeforeAck,
                    epoch: self.epoch,
                })?;
                continue;
            }
            let key = observation.symbol.clone();
            if let Some(previous) = self.last_observations.get(&key) {
                let previous_time = (previous.business_date, previous.trade_time);
                let current_time = (observation.business_date, observation.trade_time);
                if current_time < previous_time
                    || observation.cumulative_volume < previous.cumulative_volume
                {
                    self.enqueue_event(MarketStreamEvent::Status {
                        code: StreamStatusCode::StaleObservation,
                        epoch: self.epoch,
                    })?;
                    continue;
                }
                if previous == &observation {
                    self.enqueue_event(MarketStreamEvent::Status {
                        code: StreamStatusCode::DuplicateObservation,
                        epoch: self.epoch,
                    })?;
                    continue;
                }
            }
            self.last_observations.insert(key, observation.clone());
            self.receive_ordinal = self.receive_ordinal.saturating_add(1);
            let receipt = MarketReceipt::from_transport(
                self.epoch.uuid(),
                self.receive_ordinal,
                received_at_ms,
                received_monotonic,
                self.socket_open_ms,
                self.proof,
                true,
                observation,
            )?;
            self.enqueue_event(MarketStreamEvent::Receipt(receipt))?;
        }
        Ok(())
    }

    fn symbol_is_admitted_for_current_frame(&self, symbol: &str) -> bool {
        if self.command_phase == CommandPhase::AckObserved
            && let Some(pending) = self.pending.as_ref()
            && pending.symbol == symbol
        {
            return pending.operation == MarketSubscriptionOperation::Subscribe;
        }
        self.subscriptions.contains(symbol)
    }

    fn enqueue_event(&mut self, event: MarketStreamEvent) -> Result<(), MarketStreamError> {
        if self.queued_events.len() >= MAX_QUEUED_EVENTS {
            self.closed = true;
            return Err(MarketStreamError::Protocol);
        }
        self.queued_events.push_back(event);
        Ok(())
    }

    async fn process_buffered_control_before_emit(&mut self) -> Result<(), MarketStreamError> {
        const MAX_BUFFERED_MESSAGE_SCAN: usize = 8;
        for _ in 0..MAX_BUFFERED_MESSAGE_SCAN {
            let message = match self.read_buffered_message()? {
                Some(message) => message,
                None => return Ok(()),
            };
            match self.process_incoming(message).await? {
                IncomingOutcome::Continue => {}
                IncomingOutcome::Event(event) => {
                    if let Err(error) = self.enqueue_event(event) {
                        self.poison_epoch();
                        return Err(error);
                    }
                }
                IncomingOutcome::Terminal(_) => return Err(MarketStreamError::Closed),
            }
        }
        if self.read_buffered_message()?.is_some() {
            self.poison_epoch();
            return Err(MarketStreamError::Protocol);
        }
        Ok(())
    }

    async fn process_incoming(
        &mut self,
        message: IncomingMessage,
    ) -> Result<IncomingOutcome, MarketStreamError> {
        match message {
            IncomingMessage::Text(bytes) => {
                if bytes.first() == Some(&b'{') {
                    let capture = AckCapture {
                        received_at_ms: self.config.clock.now_ms(),
                        received_monotonic: Instant::now(),
                    };
                    if is_application_pingpong(&bytes) {
                        self.send_application_pong(&bytes).await?;
                        return Ok(IncomingOutcome::Event(
                            MarketStreamEvent::ApplicationHeartbeat,
                        ));
                    }
                    if self.command_phase == CommandPhase::Sent {
                        self.apply_ack(&bytes, capture)?;
                        return Ok(IncomingOutcome::Continue);
                    }
                    self.poison_epoch();
                    return Err(MarketStreamError::DuplicateAck);
                }
                let received_at_ms = self.config.clock.now_ms();
                let received_monotonic = Instant::now();
                let parsed =
                    match parse_market_message(&bytes, self.parse_context_at(received_at_ms)) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            self.poison_epoch();
                            return Err(error.into());
                        }
                    };
                if let Err(error) =
                    self.queue_observations(parsed.records, received_at_ms, received_monotonic)
                {
                    self.poison_epoch();
                    return Err(error);
                }
                Ok(IncomingOutcome::Continue)
            }
            IncomingMessage::ControlPing(payload) => {
                self.last_control_liveness = Instant::now();
                self.send_control(0xA, &payload).await?;
                Ok(IncomingOutcome::Continue)
            }
            IncomingMessage::ControlPong(payload) => {
                self.last_control_liveness = Instant::now();
                if self
                    .ping_sent
                    .as_ref()
                    .is_some_and(|(_, expected)| expected.as_slice() == payload.as_slice())
                {
                    self.ping_sent = None;
                }
                Ok(IncomingOutcome::Event(MarketStreamEvent::ControlPong))
            }
            IncomingMessage::Close(payload) => {
                if !valid_close_payload(&payload) {
                    self.poison_epoch();
                    return Err(MarketStreamError::FrameInvalid);
                }
                self.send_control(0x8, &payload).await?;
                if self.pending.is_none() {
                    self.config.state.clear_clean_connection(self.epoch)?;
                    self.peer_closed_cleanly = true;
                }
                let event = MarketStreamEvent::Status {
                    code: StreamStatusCode::Closed,
                    epoch: self.epoch,
                };
                self.poison_epoch();
                Ok(IncomingOutcome::Terminal(event))
            }
        }
    }

    async fn watchdog_check(&mut self) -> Result<(), MarketStreamError> {
        let now = Instant::now();
        if now.duration_since(self.last_inbound) >= Duration::from_secs(90) {
            self.poison_epoch();
            return Err(MarketStreamError::Closed);
        }
        if now.duration_since(self.last_control_liveness) >= Duration::from_secs(20)
            && self.ping_sent.is_none()
        {
            let payload = Vec::new();
            self.send_control(0x9, &payload).await?;
            self.ping_sent = Some((now, payload));
        }
        if self
            .ping_sent
            .as_ref()
            .is_some_and(|(sent, _)| now.duration_since(*sent) >= Duration::from_secs(10))
        {
            self.poison_epoch();
            return Err(MarketStreamError::Closed);
        }
        Ok(())
    }

    async fn read_next_message(&mut self) -> Result<IncomingMessage, MarketStreamError> {
        let message = loop {
            let result = match self.socket_mut()?.read_message().await {
                Ok(result) => result,
                Err(error) => {
                    self.poison_epoch();
                    return Err(error);
                }
            };
            match result {
                ReadMessage::Message(message) => break message,
                ReadMessage::Idle => self.watchdog_check().await?,
            }
        };
        self.note_incoming_message(&message)?;
        Ok(message)
    }

    async fn read_next_message_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<IncomingMessage>, MarketStreamError> {
        let message = loop {
            let result = match self.socket_mut()?.read_message_until(deadline).await {
                Ok(result) => result,
                Err(error) => {
                    self.poison_epoch();
                    return Err(error);
                }
            };
            match result {
                ReadMessage::Message(message) => break message,
                ReadMessage::Idle => {
                    self.watchdog_check().await?;
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                }
            }
        };
        self.note_incoming_message(&message)?;
        Ok(Some(message))
    }

    fn read_buffered_message(&mut self) -> Result<Option<IncomingMessage>, MarketStreamError> {
        let message = match self.socket_mut()?.read_available_message() {
            Ok(message) => message,
            Err(error) => {
                self.poison_epoch();
                return Err(error);
            }
        };
        if let Some(message) = message.as_ref() {
            self.note_incoming_message(message)?;
        }
        Ok(message)
    }

    fn note_incoming_message(
        &mut self,
        message: &IncomingMessage,
    ) -> Result<(), MarketStreamError> {
        self.last_inbound = Instant::now();
        match message {
            IncomingMessage::Text(bytes) if bytes.first() != Some(&b'{') => {
                self.note_data_message()
            }
            _ => self.note_control_message(),
        }
    }

    fn note_control_message(&mut self) -> Result<(), MarketStreamError> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.control_token_time).as_secs_f64();
        self.control_tokens = (self.control_tokens
            + elapsed * MAX_INCOMING_CONTROL_MESSAGES_PER_SECOND as f64)
            .min(MAX_INCOMING_CONTROL_BURST);
        self.control_token_time = now;
        if self.control_tokens < 1.0 {
            self.poison_epoch();
            return Err(MarketStreamError::Protocol);
        }
        self.control_tokens -= 1.0;
        Ok(())
    }

    fn note_data_message(&mut self) -> Result<(), MarketStreamError> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.data_token_time).as_secs_f64();
        self.data_tokens = (self.data_tokens + elapsed * MAX_INCOMING_DATA_MESSAGES_PER_SECOND)
            .min(MAX_INCOMING_DATA_BURST);
        self.data_token_time = now;
        if self.data_tokens < 1.0 {
            self.poison_epoch();
            return Err(MarketStreamError::Protocol);
        }
        self.data_tokens -= 1.0;
        Ok(())
    }

    async fn send_application_pong(&mut self, payload: &[u8]) -> Result<(), MarketStreamError> {
        if payload.len() > 125 {
            self.closed = true;
            return Err(MarketStreamError::FrameInvalid);
        }
        self.send_control(0xA, payload).await?;
        self.last_control_liveness = Instant::now();
        Ok(())
    }

    async fn send_control(&mut self, opcode: u8, payload: &[u8]) -> Result<(), MarketStreamError> {
        if payload.len() > 125 {
            self.closed = true;
            return Err(MarketStreamError::FrameInvalid);
        }
        let write_timeout = self.config.ack_timeout;
        let result = match self.socket_mut() {
            Ok(socket) => {
                match tokio::time::timeout(write_timeout, socket.send_frame(opcode, payload)).await
                {
                    Ok(result) => result,
                    Err(_) => Err(MarketStreamError::ControlWriteTimeout),
                }
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.poison_epoch();
            return Err(error);
        }
        Ok(())
    }

    fn socket_mut(&mut self) -> Result<&mut RawWebSocket, MarketStreamError> {
        let Some(socket) = self.socket.as_mut() else {
            return Err(MarketStreamError::Closed);
        };
        if socket.is_poisoned() {
            self.closed = true;
            return Err(MarketStreamError::Cancelled);
        }
        Ok(socket)
    }

    fn ensure_open(&self) -> Result<(), MarketStreamError> {
        #[cfg(test)]
        if self.socketless_command_test && !self.closed {
            return Ok(());
        }
        if self.closed || self.socket.as_ref().is_none_or(RawWebSocket::is_poisoned) {
            Err(MarketStreamError::Closed)
        } else {
            Ok(())
        }
    }

    fn parse_context_at(&self, received_at_ms: i64) -> ParseContext {
        self.proof.context_at(received_at_ms, self.socket_open_ms)
    }

    fn poison_epoch(&mut self) {
        self.closed = true;
        #[cfg(test)]
        {
            self.socketless_command_test = false;
        }
        if self.command_phase != CommandPhase::Idle || self.pending.is_some() {
            self.command_phase = CommandPhase::Poisoned;
        }
        self.pending = None;
        self.subscriptions.clear();
        self.unsubscribed_in_epoch.clear();
        self.queued_events.clear();
        self.last_observations.clear();
        self.socket.take();
    }

    fn poison_command_uncertainty(&mut self) {
        self.command_phase = CommandPhase::Poisoned;
        self.poison_epoch();
    }

    fn reserve_command_budget(&self, now: i64) -> Result<(), MarketStreamError> {
        self.config
            .state
            .with_locked_state(|state| {
                let mut budget =
                    CommandBudget::from_attempts(state.command_attempts_ms.iter().copied());
                budget.reserve(now).map_err(StateError::Budget)?;
                state.command_attempts_ms = budget.attempts().collect();
                Ok(())
            })
            .map_err(|error| match error {
                StateError::Budget(budget) => MarketStreamError::CommandBudget(budget),
                other => MarketStreamError::State(other),
            })
    }

    pub async fn reconnect(&mut self) -> Result<(), MarketStreamError> {
        if self.command_phase != CommandPhase::Idle {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        if self.lifetime_lock.is_none() {
            return Err(MarketStreamError::Closed);
        }
        let proof = current_session_proof(&self.config)?;
        if !self.peer_closed_cleanly {
            self.close_socket().await?;
        }
        let now = self.config.clock.now_ms();
        let permit =
            self.config
                .state
                .begin_connection_attempt(now, true, self.config.reconnect_policy)?;
        let approval_key = approval_key_for_attempt(&self.config, permit.attempt_id).await?;
        let mut socket = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            RawWebSocket::connect(&self.config.endpoint, self.config.write_barrier.clone()),
        )
        .await
        .map_err(|_| MarketStreamError::Handshake)??;
        let epoch = ConnectionEpoch::new();
        if let Err(error) = self
            .config
            .state
            .complete_connection(permit.attempt_id, epoch)
        {
            socket.poisoned = true;
            return Err(error.into());
        }
        self.socket = Some(socket);
        self.approval_key = approval_key;
        self.epoch = epoch;
        self.proof = proof;
        self.socket_open_ms = self.config.clock.now_ms();
        self.subscriptions.clear();
        self.unsubscribed_in_epoch.clear();
        self.pending = None;
        self.command_phase = CommandPhase::Idle;
        self.queued_events.clear();
        self.last_observations.clear();
        self.receive_ordinal = 0;
        self.last_inbound = Instant::now();
        self.last_control_liveness = Instant::now();
        self.ping_sent = None;
        self.control_tokens = MAX_INCOMING_CONTROL_BURST;
        self.control_token_time = Instant::now();
        self.data_tokens = MAX_INCOMING_DATA_BURST;
        self.data_token_time = Instant::now();
        self.closed = false;
        self.peer_closed_cleanly = false;
        self.enqueue_event(MarketStreamEvent::Status {
            code: StreamStatusCode::Gap,
            epoch: self.epoch,
        })?;
        Ok(())
    }

    async fn close_socket(&mut self) -> Result<(), MarketStreamError> {
        if self.command_phase != CommandPhase::Idle {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        if self.peer_closed_cleanly {
            self.lifetime_lock.take();
            return Ok(());
        }
        if self.closed && self.socket.is_none() {
            return Err(MarketStreamError::Closed);
        }
        if self.pending.is_some() {
            self.poison_epoch();
            return Err(MarketStreamError::CommandPending);
        }
        if let Some(mut socket) = self.socket.take() {
            if let Err(error) = socket.send_frame(0x8, &[]).await {
                self.poison_epoch();
                return Err(error);
            }
        }
        self.config.state.clear_clean_connection(self.epoch)?;
        self.closed = true;
        Ok(())
    }

    pub async fn close(mut self) -> Result<(), MarketStreamError> {
        self.close_socket().await?;
        self.lifetime_lock.take();
        Ok(())
    }
}

impl Drop for MarketStreamSession {
    fn drop(&mut self) {
        // `close` is the async graceful path.  Dropping the socket before the
        // lock makes cancellation release the lifetime inode without keeping
        // a live transport behind the lock.
        self.socket.take();
        self.lifetime_lock.take();
    }
}

#[derive(Clone)]
struct Command {
    operation: MarketSubscriptionOperation,
    symbol: String,
    approval_key: String,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Command")
            .field("operation", &self.operation)
            .field("symbol", &self.symbol)
            .field("approval_key", &"<redacted>")
            .finish()
    }
}

impl Command {
    fn new(
        operation: MarketSubscriptionOperation,
        symbol: &str,
        approval_key: &crate::market_stream_approval::ApprovalKey,
    ) -> Self {
        Self {
            operation,
            symbol: symbol.to_owned(),
            approval_key: approval_key.as_str().to_owned(),
        }
    }

    fn json(&self) -> String {
        let tr_type = match self.operation {
            MarketSubscriptionOperation::Subscribe => "1",
            MarketSubscriptionOperation::Unsubscribe => "2",
        };
        serde_json::json!({
            "header": {
                "approval_key": self.approval_key,
                "custtype": "P",
                "tr_type": tr_type,
                "content-type": "utf-8"
            },
            "body": { "input": { "tr_id": TR_ID, "tr_key": self.symbol } }
        })
        .to_string()
    }
}

#[derive(Debug)]
struct Ack {
    tr_id: String,
    tr_key: String,
    encrypt: String,
    rt_cd: String,
    msg_cd: String,
    msg1: Option<String>,
}

fn parse_ack(bytes: &[u8]) -> Option<Ack> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        header: Header,
        body: Body,
    }
    #[derive(serde::Deserialize)]
    struct Header {
        tr_id: String,
        tr_key: String,
        encrypt: String,
    }
    #[derive(serde::Deserialize)]
    struct Body {
        rt_cd: String,
        msg_cd: String,
        msg1: Option<String>,
    }
    let envelope: Envelope = serde_json::from_slice(bytes).ok()?;
    Some(Ack {
        tr_id: envelope.header.tr_id,
        tr_key: envelope.header.tr_key,
        encrypt: envelope.header.encrypt,
        rt_cd: envelope.body.rt_cd,
        msg_cd: envelope.body.msg_cd,
        msg1: envelope.body.msg1,
    })
}

fn is_application_pingpong(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return false;
    };
    value
        .get("header")
        .and_then(|header| header.get("tr_id"))
        .and_then(Value::as_str)
        .is_some_and(|tr_id| tr_id == "PINGPONG")
        && value.get("body").is_none()
}

#[cfg(test)]
fn ack_deadline_expired(deadline: Instant, received: Instant) -> bool {
    received >= deadline
}

fn available_read_budget_exhausted(successful_reads: usize) -> bool {
    successful_reads >= MAX_AVAILABLE_READS_PER_DRAIN
}

fn valid_close_payload(payload: &[u8]) -> bool {
    if payload.len() == 1 {
        return false;
    }
    if payload.is_empty() {
        return true;
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    // RFC 6455 permits 1000..=1014 except reserved 1004/1005/1006, and
    // application/library codes 3000..=4999.  The reason is UTF-8 only.
    (matches!(code, 1000..=1014 if !matches!(code, 1004 | 1005 | 1006))
        || matches!(code, 3000..=4999))
        && std::str::from_utf8(&payload[2..]).is_ok()
}

fn validate_symbol(symbol: &str) -> Result<(), MarketStreamError> {
    if !APPROVED_MARKET_SYMBOLS.contains(&symbol) {
        Err(MarketStreamError::SymbolNotAllowed)
    } else {
        Ok(())
    }
}

enum IncomingMessage {
    Text(Vec<u8>),
    ControlPing(Vec<u8>),
    ControlPong(Vec<u8>),
    Close(Vec<u8>),
}

struct RawWebSocket {
    stream: TcpStream,
    fragmented_text: Option<FragmentedText>,
    read_buffer: Vec<u8>,
    write_in_progress: bool,
    poisoned: bool,
    write_barrier: Option<Arc<Notify>>,
}

struct FragmentedText {
    payload: Vec<u8>,
    frame_count: usize,
}

enum ReadMessage {
    Idle,
    Message(IncomingMessage),
}

enum ReadFrame {
    Idle,
    Frame(bool, u8, Vec<u8>),
}

struct FrameHeader {
    fin: bool,
    opcode: u8,
    header_len: usize,
    payload_len: usize,
}

impl RawWebSocket {
    async fn connect(
        endpoint: &MarketStreamEndpoint,
        write_barrier: Option<Arc<Notify>>,
    ) -> Result<Self, MarketStreamError> {
        let (host, port, path) = endpoint.parts();
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|_| MarketStreamError::Io)?;
        let nonce = Uuid::new_v4().into_bytes();
        let key = base64::engine::general_purpose::STANDARD.encode(nonce);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        let mut websocket = Self {
            stream,
            fragmented_text: None,
            read_buffer: Vec::new(),
            write_in_progress: false,
            poisoned: false,
            write_barrier,
        };
        websocket
            .stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| MarketStreamError::Io)?;
        websocket.read_handshake(&key).await?;
        Ok(websocket)
    }

    async fn read_handshake(&mut self, key: &str) -> Result<(), MarketStreamError> {
        let mut bytes = Vec::with_capacity(1024);
        let mut one = [0u8; 1];
        while bytes.len() < MAX_HANDSHAKE_BYTES {
            self.stream
                .read_exact(&mut one)
                .await
                .map_err(|_| MarketStreamError::Handshake)?;
            bytes.push(one[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        if !bytes.ends_with(b"\r\n\r\n") {
            return Err(MarketStreamError::Handshake);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| MarketStreamError::Handshake)?;
        let mut lines = text.split("\r\n");
        if lines.next() != Some("HTTP/1.1 101 Switching Protocols") {
            return Err(MarketStreamError::Handshake);
        }
        let mut accept = None;
        let mut upgrade = false;
        let mut connection = false;
        for line in lines {
            if line.is_empty() {
                break;
            }
            let Some((name, value)) = line.split_once(':') else {
                return Err(MarketStreamError::Handshake);
            };
            if name.eq_ignore_ascii_case("sec-websocket-accept") {
                accept = Some(value.trim());
            }
            if name.eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket")
            {
                upgrade = true;
            }
            if name.eq_ignore_ascii_case("connection")
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            {
                connection = true;
            }
        }
        let mut digest = Sha1::new();
        digest.update(key.as_bytes());
        digest.update(WEBSOCKET_GUID.as_bytes());
        let expected = base64::engine::general_purpose::STANDARD.encode(digest.digest().bytes());
        if accept != Some(expected.as_str()) || !upgrade || !connection {
            return Err(MarketStreamError::Handshake);
        }
        Ok(())
    }

    async fn read_message(&mut self) -> Result<ReadMessage, MarketStreamError> {
        loop {
            let ReadFrame::Frame(fin, opcode, payload) = self.read_frame().await? else {
                return Ok(ReadMessage::Idle);
            };
            if let Some(message) = self.assemble_frame(fin, opcode, payload)? {
                return Ok(ReadMessage::Message(message));
            }
        }
    }

    async fn read_message_until(
        &mut self,
        deadline: Instant,
    ) -> Result<ReadMessage, MarketStreamError> {
        loop {
            if Instant::now() >= deadline {
                return Ok(ReadMessage::Idle);
            }
            let ReadFrame::Frame(fin, opcode, payload) = self.read_frame_until(deadline).await?
            else {
                return Ok(ReadMessage::Idle);
            };
            if let Some(message) = self.assemble_frame(fin, opcode, payload)? {
                return Ok(ReadMessage::Message(message));
            }
        }
    }

    fn read_buffered_message(&mut self) -> Result<Option<IncomingMessage>, MarketStreamError> {
        loop {
            let Some(ReadFrame::Frame(fin, opcode, payload)) = self.take_buffered_frame()? else {
                return Ok(None);
            };
            if let Some(message) = self.assemble_frame(fin, opcode, payload)? {
                return Ok(Some(message));
            }
        }
    }

    fn read_available_message(&mut self) -> Result<Option<IncomingMessage>, MarketStreamError> {
        let mut successful_reads = 0usize;
        loop {
            if let Some(message) = self.read_buffered_message()? {
                return Ok(Some(message));
            }
            if available_read_budget_exhausted(successful_reads) {
                return Err(MarketStreamError::FrameInvalid);
            }
            let mut bytes = [0u8; 8192];
            match self.stream.try_read(&mut bytes) {
                Ok(0) => return Err(MarketStreamError::Closed),
                Ok(read) => {
                    successful_reads += 1;
                    if self.read_buffer.len().saturating_add(read) > MAX_MESSAGE_BYTES + 14 {
                        return Err(MarketStreamError::FrameInvalid);
                    }
                    self.read_buffer.extend_from_slice(&bytes[..read]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(_) => return Err(MarketStreamError::Closed),
            }
        }
    }

    fn assemble_frame(
        &mut self,
        fin: bool,
        opcode: u8,
        payload: Vec<u8>,
    ) -> Result<Option<IncomingMessage>, MarketStreamError> {
        if let Some(fragmented) = self.fragmented_text.as_mut() {
            fragmented.frame_count = fragmented.frame_count.saturating_add(1);
            if fragmented.frame_count > MAX_FRAGMENTED_MESSAGE_FRAMES {
                return Err(MarketStreamError::FrameInvalid);
            }
        }
        match opcode {
            0x8 => Ok(Some(IncomingMessage::Close(payload))),
            0x9 => Ok(Some(IncomingMessage::ControlPing(payload))),
            0xA => Ok(Some(IncomingMessage::ControlPong(payload))),
            0x1 => {
                if self.fragmented_text.is_some() {
                    return Err(MarketStreamError::FrameInvalid);
                }
                if fin {
                    Ok(Some(IncomingMessage::Text(payload)))
                } else {
                    self.fragmented_text = Some(FragmentedText {
                        payload,
                        frame_count: 1,
                    });
                    Ok(None)
                }
            }
            0x0 => {
                let Some(fragmented) = self.fragmented_text.as_mut() else {
                    return Err(MarketStreamError::FrameInvalid);
                };
                if fragmented.payload.len().saturating_add(payload.len()) > MAX_MESSAGE_BYTES {
                    return Err(MarketStreamError::FrameInvalid);
                }
                fragmented.payload.extend(payload);
                if fin {
                    Ok(Some(IncomingMessage::Text(
                        self.fragmented_text.take().unwrap().payload,
                    )))
                } else {
                    Ok(None)
                }
            }
            _ => Err(MarketStreamError::Protocol),
        }
    }

    fn take_buffered_frame(&mut self) -> Result<Option<ReadFrame>, MarketStreamError> {
        let Some(header) = self.frame_header()? else {
            return Ok(None);
        };
        let frame_len = header.header_len.saturating_add(header.payload_len);
        if self.read_buffer.len() < frame_len {
            return Ok(None);
        }
        let frame: Vec<u8> = self.read_buffer.drain(..frame_len).collect();
        Ok(Some(ReadFrame::Frame(
            header.fin,
            header.opcode,
            frame[header.header_len..].to_vec(),
        )))
    }

    async fn read_frame(&mut self) -> Result<ReadFrame, MarketStreamError> {
        loop {
            if let Some(frame) = self.take_buffered_frame()? {
                return Ok(frame);
            }
            let Some(_) = self.frame_header()? else {
                if !self.read_more().await? {
                    return Ok(ReadFrame::Idle);
                }
                continue;
            };
            if !self.read_more().await? {
                return Ok(ReadFrame::Idle);
            }
        }
    }

    async fn read_frame_until(
        &mut self,
        deadline: Instant,
    ) -> Result<ReadFrame, MarketStreamError> {
        loop {
            if Instant::now() >= deadline {
                return Ok(ReadFrame::Idle);
            }
            if let Some(frame) = self.take_buffered_frame()? {
                return Ok(frame);
            }
            if !self.read_more_until(deadline).await? {
                return Ok(ReadFrame::Idle);
            }
        }
    }

    fn frame_header(&self) -> Result<Option<FrameHeader>, MarketStreamError> {
        if self.read_buffer.len() < 2 {
            return Ok(None);
        }
        let first = self.read_buffer[0];
        let second = self.read_buffer[1];
        let fin = first & 0x80 != 0;
        let rsv = first & 0x70;
        let opcode = first & 0x0F;
        let masked = second & 0x80 != 0;
        if masked {
            // Client-to-server frames are masked; server-to-client frames are
            // never masked.  Do not accept and silently unmask an invalid
            // peer frame.
            return Err(MarketStreamError::FrameInvalid);
        }
        let length_marker = second & 0x7F;
        if rsv != 0 {
            return Err(MarketStreamError::FrameInvalid);
        }
        let (header_len, payload_len) = match length_marker {
            0..=125 => (2, usize::from(length_marker)),
            126 => {
                if self.read_buffer.len() < 4 {
                    return Ok(None);
                }
                (
                    4,
                    usize::from(u16::from_be_bytes([
                        self.read_buffer[2],
                        self.read_buffer[3],
                    ])),
                )
            }
            127 => {
                if self.read_buffer.len() < 10 {
                    return Ok(None);
                }
                let length = u64::from_be_bytes(self.read_buffer[2..10].try_into().unwrap());
                if length & (1u64 << 63) != 0 || length > MAX_MESSAGE_BYTES as u64 {
                    return Err(MarketStreamError::FrameInvalid);
                }
                (10, length as usize)
            }
            _ => unreachable!(),
        };
        let is_control = opcode & 0x8 != 0;
        if (is_control && (!fin || payload_len > 125)) || payload_len > MAX_MESSAGE_BYTES {
            return Err(MarketStreamError::FrameInvalid);
        }
        Ok(Some(FrameHeader {
            fin,
            opcode,
            header_len,
            payload_len,
        }))
    }

    async fn read_more(&mut self) -> Result<bool, MarketStreamError> {
        match tokio::time::timeout(Duration::from_secs(1), self.stream.readable()).await {
            Ok(Ok(())) => {
                let mut bytes = [0u8; 8192];
                loop {
                    match self.stream.try_read(&mut bytes) {
                        Ok(0) => return Err(MarketStreamError::Closed),
                        Ok(read) => {
                            if self.read_buffer.len().saturating_add(read) > MAX_MESSAGE_BYTES + 14
                            {
                                return Err(MarketStreamError::FrameInvalid);
                            }
                            self.read_buffer.extend_from_slice(&bytes[..read]);
                            return Ok(true);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            return Ok(false);
                        }
                        Err(_) => return Err(MarketStreamError::Closed),
                    }
                }
            }
            Ok(Err(_)) => Err(MarketStreamError::Closed),
            Err(_) => Ok(false),
        }
    }

    async fn read_more_until(&mut self, deadline: Instant) -> Result<bool, MarketStreamError> {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(1));
        if wait.is_zero() {
            return Ok(false);
        }
        match tokio::time::timeout(wait, self.stream.readable()).await {
            Ok(Ok(())) => {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                let mut bytes = [0u8; 8192];
                loop {
                    if Instant::now() >= deadline {
                        return Ok(false);
                    }
                    match self.stream.try_read(&mut bytes) {
                        Ok(0) => return Err(MarketStreamError::Closed),
                        Ok(read) => {
                            if self.read_buffer.len().saturating_add(read) > MAX_MESSAGE_BYTES + 14
                            {
                                return Err(MarketStreamError::FrameInvalid);
                            }
                            self.read_buffer.extend_from_slice(&bytes[..read]);
                            return Ok(true);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            return Ok(false);
                        }
                        Err(_) => return Err(MarketStreamError::Closed),
                    }
                }
            }
            Ok(Err(_)) => Err(MarketStreamError::Closed),
            Err(_) => Ok(false),
        }
    }

    async fn send_text(&mut self, payload: &[u8]) -> Result<(), MarketStreamError> {
        if payload.len() > MAX_MESSAGE_BYTES {
            return Err(MarketStreamError::FrameInvalid);
        }
        self.send_frame(0x1, payload).await
    }

    async fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), MarketStreamError> {
        if self.is_poisoned() {
            return Err(MarketStreamError::Cancelled);
        }
        if payload.len() > MAX_MESSAGE_BYTES || (opcode & 0x8 != 0 && payload.len() > 125) {
            return Err(MarketStreamError::FrameInvalid);
        }
        let mask_full = Uuid::new_v4().into_bytes();
        let mask = [mask_full[0], mask_full[1], mask_full[2], mask_full[3]];
        let mut frame = Vec::with_capacity(payload.len() + 16);
        frame.push(0x80 | opcode);
        match payload.len() {
            0..=125 => frame.push(0x80 | payload.len() as u8),
            126..=65_535 => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            _ => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&mask);
        for (index, byte) in payload.iter().enumerate() {
            frame.push(*byte ^ mask[index % 4]);
        }
        // Set the poison marker before the first await.  If the caller drops
        // this future during a partial write, Drop cannot await a close frame;
        // the marker therefore makes every later operation fail closed.
        self.write_in_progress = true;
        let result = self.stream.write_all(&frame).await;
        if result.is_ok() {
            if let Some(barrier) = self.write_barrier.as_ref() {
                barrier.notified().await;
            }
        }
        self.write_in_progress = false;
        if let Err(_) = result {
            self.poisoned = true;
            return Err(MarketStreamError::Io);
        }
        Ok(())
    }

    fn is_poisoned(&self) -> bool {
        self.poisoned || self.write_in_progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_stream_state::MarketStreamDomain;

    #[test]
    fn endpoint_rejects_non_loopback_and_preserves_production_literal() {
        assert_eq!(
            MarketStreamEndpoint::production().literal(),
            PRODUCTION_WS_URL
        );
        assert!(MarketStreamEndpoint::loopback("wss://127.0.0.1:1/tryitout").is_err());
        assert!(MarketStreamEndpoint::loopback("ws://example.invalid:1/tryitout").is_err());
        assert!(MarketStreamEndpoint::loopback("ws://127.0.0.1:1/tryitout?proxy=x").is_err());
        assert!(MarketStreamEndpoint::loopback("ws://127.0.0.1:1/account").is_err());
        assert!(MarketStreamEndpoint::loopback("ws://127.0.0.1:1/tryitout").is_ok());
    }

    #[test]
    fn outbound_admission_is_the_canonical_exact30_set() {
        assert_eq!(APPROVED_MARKET_SYMBOLS.len(), 30);
        let unique = APPROVED_MARKET_SYMBOLS
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), APPROVED_MARKET_SYMBOLS.len());
        assert!(validate_symbol("005930").is_ok());
        assert_eq!(
            validate_symbol("123456"),
            Err(MarketStreamError::SymbolNotAllowed)
        );
        let canonical_identity_list = APPROVED_MARKET_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX\n"))
            .collect::<String>();
        let digest = <sha2::Sha256 as sha2::Digest>::digest(canonical_identity_list.as_bytes());
        assert_eq!(
            format!("{digest:x}"),
            "0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79",
            "canonical fixed-stock identity hash drifted"
        );
    }

    #[test]
    fn command_uses_subscribe_one_and_unsubscribe_two() {
        let key =
            crate::market_stream_approval::ApprovalKey::new("SYNTHETIC_KEY".to_owned()).unwrap();
        let subscribe = Command::new(MarketSubscriptionOperation::Subscribe, "005930", &key).json();
        let unsubscribe =
            Command::new(MarketSubscriptionOperation::Unsubscribe, "005930", &key).json();
        assert!(subscribe.contains(r#""tr_type":"1""#));
        assert!(unsubscribe.contains(r#""tr_type":"2""#));
        assert!(!unsubscribe.contains(r#""tr_type":"0""#));
    }

    #[test]
    fn client_debug_does_not_contain_approval_secret() {
        let directory = tempfile::tempdir().unwrap();
        let state = MarketStreamDomain::for_test(directory.path(), Uuid::new_v4()).unwrap();
        let approval = ApprovalClient::for_loopback(
            "http://127.0.0.1:34567",
            crate::secret::Secret::new("APP_SENTINEL".to_owned()),
            crate::secret::Secret::new("SECRET_SENTINEL".to_owned()),
            state.clone(),
            "1",
        )
        .unwrap();
        let client = MarketStreamClient::new(
            MarketStreamConfig::loopback(approval, "ws://127.0.0.1:34568/tryitout").unwrap(),
        );
        let debug = format!("{client:?}");
        assert!(!debug.contains("APP_SENTINEL"));
        assert!(!debug.contains("SECRET_SENTINEL"));
    }

    #[test]
    fn ack_duplicate_fields_and_nonempty_pingpong_body_fail_closed() {
        assert!(parse_ack(
            br#"{"header":{"tr_id":"H0STCNT0","tr_id":"H0STCNT0","tr_key":"005930","encrypt":"N"},"body":{"rt_cd":"0","msg_cd":"OPSP0000","msg1":"SUBSCRIBE SUCCESS"}}"#
        )
        .is_none());
        assert!(!is_application_pingpong(
            br#"{"header":{"tr_id":"PINGPONG"},"body":{}}"#
        ));
        assert!(is_application_pingpong(
            br#"{"header":{"tr_id":"PINGPONG"}}"#
        ));
    }

    #[test]
    fn acknowledgement_deadline_is_half_open() {
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(!ack_deadline_expired(
            deadline,
            deadline - Duration::from_nanos(1)
        ));
        assert!(ack_deadline_expired(deadline, deadline));
        assert!(ack_deadline_expired(
            deadline,
            deadline + Duration::from_nanos(1)
        ));
    }

    #[test]
    fn command_timing_checks_overflow_regression_disagreement_and_half_open_edges() {
        let origin = Instant::now();
        assert!(origin.checked_add(Duration::MAX).is_none());
        assert_eq!(
            CommandTiming::reserve(0, origin, Duration::MAX).unwrap_err(),
            MarketStreamError::CommandTimingOverflow,
            "monotonic checked_add overflow is rejected"
        );
        assert_eq!(
            CommandTiming::reserve(0, origin, Duration::from_secs(10_000_000_000_000_000),)
                .unwrap_err(),
            MarketStreamError::CommandTimingOverflow,
            "timeout-to-milliseconds conversion overflow is rejected"
        );
        assert!(
            Duration::from_secs(10_000_000_000_000_000).as_millis() > i64::MAX as u128,
            "conversion fixture exceeds the persisted signed millisecond type"
        );
        assert_eq!(
            CommandTiming::reserve(i64::MAX, origin, Duration::from_millis(1)).unwrap_err(),
            MarketStreamError::CommandTimingOverflow,
            "wall deadline checked_add overflow is rejected"
        );

        let timing = CommandTiming::reserve(10_000, origin, Duration::from_millis(5)).unwrap();
        assert_eq!(
            timing.check(9_999, origin + Duration::from_millis(1)),
            Err(MarketStreamError::CommandTimingInvalid),
            "wall-clock regression is rejected"
        );
        assert_eq!(
            timing.check(10_001, origin - Duration::from_nanos(1)),
            Err(MarketStreamError::CommandTimingInvalid),
            "monotonic regression is rejected"
        );
        assert_eq!(
            timing.check(10_001, timing.deadline_monotonic),
            Err(MarketStreamError::CommandTimingInvalid),
            "one clock inside and one at equality is inconsistent"
        );
        assert_eq!(
            timing.check(timing.deadline_at_ms, origin + Duration::from_millis(1)),
            Err(MarketStreamError::CommandTimingInvalid),
            "the inverse one-clock disagreement is rejected"
        );
        assert_eq!(
            timing.check(10_004, timing.deadline_monotonic - Duration::from_nanos(1)),
            Ok(()),
            "both clocks strictly before their deadlines are accepted"
        );
        assert_eq!(
            timing.check(timing.deadline_at_ms, timing.deadline_monotonic),
            Err(MarketStreamError::AckTimeout),
            "equality on both clocks is late"
        );
    }

    #[test]
    fn synchronous_available_read_budget_is_exactly_sixty_four() {
        assert!(!available_read_budget_exhausted(63));
        assert!(available_read_budget_exhausted(64));
        assert!(available_read_budget_exhausted(65));
    }
}

#[cfg(test)]
mod command_eligibility_unit_tests {
    use super::*;
    use crate::clock::TestClock;
    use crate::market_stream_state::{DurablePendingCommand, MarketStreamDomain, STATE_FILE};
    use crate::secret::Secret;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DurableCommandSnapshot {
        attempts_ms: Vec<i64>,
        next_ordinal: u64,
        pending: Option<(String, String, String, u64, i64, i64)>,
        epoch: Option<String>,
    }

    fn durable_snapshot(store: &WsStateStore) -> DurableCommandSnapshot {
        let state = store.load().expect("test state loads");
        DurableCommandSnapshot {
            attempts_ms: state.command_attempts_ms,
            next_ordinal: state.next_command_ordinal,
            pending: state.pending_command.map(|pending| {
                (
                    pending.epoch,
                    pending.operation,
                    pending.symbol,
                    pending.ordinal,
                    pending.sent_at_ms,
                    pending.deadline_ms,
                )
            }),
            epoch: state.current_epoch,
        }
    }

    fn command_state_fixture(
        now_ms: i64,
        attempts_ms: Vec<i64>,
        subscriptions: HashSet<String>,
    ) -> (
        TempDir,
        WsStateStore,
        Arc<TestClock>,
        MarketStreamSession,
        PathBuf,
    ) {
        let directory = tempfile::tempdir().expect("task-local state directory");
        let credential_slot_id = Uuid::new_v4();
        let domain = MarketStreamDomain::for_test(directory.path(), credential_slot_id)
            .expect("test state domain");
        let state_path = domain.state_directory().join(STATE_FILE);
        let store = domain.state().clone();
        let epoch = ConnectionEpoch::new();
        store
            .with_locked_state(|state| {
                state.current_epoch = Some(epoch.uuid().to_string());
                state.command_attempts_ms = attempts_ms;
                Ok(())
            })
            .expect("seed command state");
        let clock = Arc::new(TestClock::at(now_ms));
        let approval = ApprovalClient::for_loopback(
            "http://127.0.0.1:34567",
            Secret::new("SYNTHETIC_APP".to_owned()),
            Secret::new("SYNTHETIC_SECRET".to_owned()),
            domain,
            "1",
        )
        .expect("loopback approval config")
        .with_clock(clock.clone());
        let config = MarketStreamConfig::loopback(approval, "ws://127.0.0.1:34568/tryitout")
            .expect("loopback stream config")
            .with_clock(clock.clone());
        let session =
            MarketStreamSession::for_command_eligibility_test(config, epoch, subscriptions);
        (directory, store, clock, session, state_path)
    }

    fn subscription_snapshot(session: &MarketStreamSession) -> Vec<String> {
        let mut symbols = session.subscriptions.iter().cloned().collect::<Vec<_>>();
        symbols.sort();
        symbols
    }

    #[test]
    fn spacing_hint_is_read_only_and_exact_at_the_boundary() {
        let attempts = vec![10_000];
        assert_eq!(command_spacing_not_before_ms(&[], 10_000), Ok(None));
        assert_eq!(
            command_spacing_not_before_ms(&attempts, 10_999),
            Ok(Some(11_000))
        );
        assert_eq!(command_spacing_not_before_ms(&attempts, 11_000), Ok(None));
        assert_eq!(attempts, vec![10_000]);
        let mut authoritative = CommandBudget::from_attempts(attempts);
        assert_eq!(
            authoritative.reserve(10_999),
            Err(BudgetError::MinimumSpacing)
        );
        assert_eq!(authoritative.reserve(11_000), Ok(()));
        assert_eq!(
            authoritative.attempts().collect::<Vec<_>>(),
            vec![10_000, 11_000]
        );
    }

    #[test]
    fn eligibility_does_not_soften_rolling_or_daily_limits() {
        let rolling = (0..CommandBudget::ROLLING_LIMIT)
            .map(|i| i as i64 * CommandBudget::MIN_SPACING_MS)
            .collect::<Vec<_>>();
        let now = CommandBudget::ROLLING_LIMIT as i64 * CommandBudget::MIN_SPACING_MS;
        assert_eq!(
            command_spacing_not_before_ms(&rolling, now),
            Err(MarketStreamError::CommandBudget(BudgetError::RollingWindow))
        );
        // All attempts remain within one day, but fewer than 120 in ten minutes.
        let daily = (0..CommandBudget::DAILY_LIMIT)
            .map(|i| i as i64 * 60_000)
            .collect::<Vec<_>>();
        let now = CommandBudget::DAILY_LIMIT as i64 * 60_000;
        assert_eq!(
            command_spacing_not_before_ms(&daily, now),
            Err(MarketStreamError::CommandBudget(BudgetError::DailyLimit))
        );
        assert_eq!(
            command_spacing_not_before_ms(&rolling, 119_999),
            Ok(Some(120_000))
        );
        assert_eq!(rolling.len(), CommandBudget::ROLLING_LIMIT);
        assert_eq!(daily.len(), CommandBudget::DAILY_LIMIT);
    }

    #[test]
    fn eligibility_reports_clock_rollback_and_overflow_without_reserving() {
        assert_eq!(
            command_spacing_not_before_ms(&[10_000], 9_000),
            Ok(Some(11_000))
        );
        assert_eq!(
            command_spacing_not_before_ms(&[i64::MAX - 500], i64::MAX - 500),
            Err(MarketStreamError::CommandTimingOverflow)
        );
        let old = vec![0];
        assert_eq!(
            command_spacing_not_before_ms(&old, CommandBudget::DAILY_WINDOW_MS),
            Ok(None)
        );
        assert_eq!(old, vec![0]);
    }

    #[test]
    fn state_backed_hint_preserves_every_field_and_boundary_prepare_reserves_once() {
        let (_directory, store, clock, mut session, _state_path) =
            command_state_fixture(10_999, vec![10_000], HashSet::new());
        let durable_before = durable_snapshot(&store);
        let subscriptions_before = subscription_snapshot(&session);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Ok(Some(11_000)),
        );
        assert_eq!(durable_snapshot(&store), durable_before);
        assert_eq!(subscription_snapshot(&session), subscriptions_before);
        assert!(session.pending.is_none());
        assert_eq!(session.command_phase, CommandPhase::Idle);

        clock.advance_ms(1);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Ok(None),
        );
        assert_eq!(durable_snapshot(&store), durable_before);
        let prepared = session
            .prepare_subscribe("005930")
            .expect("authoritative prepare succeeds")
            .expect("one command is prepared");
        assert_eq!(prepared.ordinal, 1);
        let durable_after = durable_snapshot(&store);
        assert_eq!(durable_after.attempts_ms, vec![10_000, 11_000]);
        assert_eq!(durable_after.next_ordinal, 1);
        assert_eq!(
            durable_after.pending.as_ref().map(|pending| pending.3),
            Some(1)
        );
        assert_eq!(
            session.pending.as_ref().map(|pending| pending.ordinal),
            Some(1)
        );
        assert_eq!(session.command_phase, CommandPhase::Prepared);
    }

    #[test]
    fn request_noops_capacity_and_resubscribe_do_not_reserve() {
        let mut subscribed = HashSet::new();
        subscribed.insert("005930".to_owned());
        let (_directory, store, _clock, mut session, _state_path) =
            command_state_fixture(20_000, Vec::new(), subscribed);
        let before = durable_snapshot(&store);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Ok(None),
        );
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Unsubscribe,
                "000660",
            ),
            Ok(None),
        );
        assert_eq!(durable_snapshot(&store), before);

        session.subscriptions = (0..MAX_ACTIVE_SUBSCRIPTIONS)
            .map(|index| format!("synthetic-capacity-{index}"))
            .collect();
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "000660",
            ),
            Err(MarketStreamError::SubscriptionLimit),
        );
        assert_eq!(durable_snapshot(&store), before);
        assert!(!session.closed);

        session.subscriptions.clear();
        session.unsubscribed_in_epoch.insert("000660".to_owned());
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "000660",
            ),
            Err(MarketStreamError::ResubscribeRequiresNewEpoch),
        );
        assert_eq!(durable_snapshot(&store), before);
        assert!(!session.closed);
    }

    #[test]
    fn pending_phase_load_and_epoch_uncertainty_terminalize_without_reserving() {
        let (_directory, store, _clock, mut phase_session, _state_path) =
            command_state_fixture(30_000, Vec::new(), HashSet::new());
        let before = durable_snapshot(&store);
        phase_session.command_phase = CommandPhase::Prepared;
        assert_eq!(
            phase_session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::CommandPending),
        );
        assert!(phase_session.closed);
        assert_eq!(durable_snapshot(&store), before);

        let (_directory, store, _clock, mut pending_session, _state_path) =
            command_state_fixture(30_000, Vec::new(), HashSet::new());
        let epoch = pending_session.epoch.uuid().to_string();
        store
            .with_locked_state(|state| {
                state.next_command_ordinal = 1;
                state.pending_command = Some(DurablePendingCommand {
                    epoch,
                    operation: "subscribe".to_owned(),
                    symbol: "005930".to_owned(),
                    ordinal: 1,
                    sent_at_ms: 29_000,
                    deadline_ms: 34_000,
                });
                Ok(())
            })
            .expect("seed durable pending command");
        let before = durable_snapshot(&store);
        assert!(matches!(
            pending_session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::State(StateError::InvalidState))
        ));
        assert!(pending_session.closed);
        assert_eq!(durable_snapshot(&store), before);

        let (_directory, store, _clock, mut epoch_session, _state_path) =
            command_state_fixture(30_000, Vec::new(), HashSet::new());
        store
            .with_locked_state(|state| {
                state.current_epoch = Some(Uuid::new_v4().to_string());
                Ok(())
            })
            .expect("seed epoch mismatch");
        let before = durable_snapshot(&store);
        assert!(matches!(
            epoch_session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::State(StateError::InvalidState))
        ));
        assert!(epoch_session.closed);
        assert_eq!(durable_snapshot(&store), before);

        let (_directory, _store, _clock, mut load_session, state_path) =
            command_state_fixture(30_000, Vec::new(), HashSet::new());
        fs::write(state_path, b"{").expect("corrupt only the task-local test state");
        assert!(matches!(
            load_session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::State(_))
        ));
        assert!(load_session.closed);
    }

    #[test]
    fn rolling_daily_and_clock_rollback_preserve_durable_state() {
        let rolling = (0..CommandBudget::ROLLING_LIMIT)
            .map(|index| index as i64 * CommandBudget::MIN_SPACING_MS)
            .collect::<Vec<_>>();
        let rolling_now = CommandBudget::ROLLING_LIMIT as i64 * CommandBudget::MIN_SPACING_MS;
        let (_directory, store, _clock, mut session, _state_path) =
            command_state_fixture(rolling_now, rolling, HashSet::new());
        let before = durable_snapshot(&store);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::CommandBudget(BudgetError::RollingWindow)),
        );
        assert_eq!(durable_snapshot(&store), before);
        assert!(!session.closed);

        let daily = (0..CommandBudget::DAILY_LIMIT)
            .map(|index| index as i64 * 60_000)
            .collect::<Vec<_>>();
        let daily_now = CommandBudget::DAILY_LIMIT as i64 * 60_000;
        let (_directory, store, _clock, mut session, _state_path) =
            command_state_fixture(daily_now, daily, HashSet::new());
        let before = durable_snapshot(&store);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Err(MarketStreamError::CommandBudget(BudgetError::DailyLimit)),
        );
        assert_eq!(durable_snapshot(&store), before);
        assert!(!session.closed);

        let (_directory, store, _clock, mut session, _state_path) =
            command_state_fixture(9_000, vec![10_000], HashSet::new());
        let before = durable_snapshot(&store);
        assert_eq!(
            session.subscription_command_not_before_ms(
                MarketSubscriptionOperation::Subscribe,
                "005930",
            ),
            Ok(Some(11_000)),
        );
        assert_eq!(durable_snapshot(&store), before);
        assert!(!session.closed);
    }
}
