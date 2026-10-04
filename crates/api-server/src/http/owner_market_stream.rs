//! Owner/session-authenticated latest-value market delivery. This module has
//! no broker client, account path, credential access or demand-on-GET behavior.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use auth::sessions::cookie;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use job_queue::owner_equity_v2::{MarketStreamStorageError as StorageError, StreamSnapshot};
use sqlx::postgres::PgListener;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, MissedTickBehavior, timeout};
use uuid::Uuid;

use super::error::{api_error, code_error, request_id};
use super::idempotency::{self, CachedResult};
use super::owner_market_stream_config::{MarketStreamReadConfig, MarketStreamReadPins};
use super::owner_market_stream_contract::{
    self as contract, InvalidContract, LeaseDto, MAX_REQUEST_BYTES, StatusBody,
};
use super::owner_market_stream_delivery::{
    Batch, ConsumerPermit, DeliveryEncoder, DeliverySignals, LatestMailbox, READ_DEADLINE,
};
use super::owner_market_stream_projection as projection;
use super::session::{Session, require_csrf, resolve_session};
use super::state::ApiState;
use crate::actor_tx::actor_uuid;
use crate::repos::owner_market_stream::OwnerMarketStreamRepo;

const PREFIX: &str = "/api/v1/research/owner-beta/equity-universe-v2";
const NOTIFY_CHANNEL: &str = "owner_market_stream_changed";

/// Kept only in this request's memory. Deliberately no Debug/Serialize.
struct SessionIdentity {
    cookie: String,
    hash: String,
    owner: Uuid,
}

impl SessionIdentity {
    fn new(session: &Session, headers: &HeaderMap) -> Result<Self, InvalidContract> {
        let cookie = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| cookie::parse(v, cookie::NAME))
            .ok_or(InvalidContract)?;
        let owner = actor_uuid(&session.actor()).map_err(|_| InvalidContract)?;
        Ok(Self {
            hash: cookie::hash(&cookie),
            cookie,
            owner,
        })
    }

    async fn authorize(&self, state: &ApiState) -> Result<auth::entitlement::Actor, EndReason> {
        let session = timeout(READ_DEADLINE, resolve_session(state, &self.cookie))
            .await
            .map_err(|_| EndReason::Unavailable)?
            .map_err(|error| {
                if error.is_session_denied() {
                    EndReason::AccessRevoked
                } else {
                    EndReason::Unavailable
                }
            })?;
        let actor = session.actor();
        if !actor.is_owner() || actor_uuid(&actor).ok() != Some(self.owner) {
            return Err(EndReason::AccessRevoked);
        }
        Ok(actor)
    }
}

#[derive(Clone, Copy)]
enum EndReason {
    AccessRevoked = 1,
    Unavailable = 2,
}

fn configuration<'a>(
    state: &'a ApiState,
    session: &Session,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<&'a MarketStreamReadPins, Response> {
    let rid = request_id(headers);
    if !session.actor().is_owner() {
        return Err(code_error("FORBIDDEN", "forbidden", &rid));
    }
    let MarketStreamReadConfig::OwnerOnly(pins) = &state.cfg.owner_market_stream else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "FEATURE_DISABLED",
            "market stream disabled",
            &rid,
            None,
        ));
    };
    if !pins.allows_request(headers, uri) {
        return Err(code_error("FORBIDDEN", "origin denied", &rid));
    }
    Ok(pins)
}

fn unavailable(headers: &HeaderMap) -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "MARKET_STREAM_UNAVAILABLE",
        "market stream unavailable",
        &request_id(headers),
        None,
    )
}

fn storage_error(error: StorageError, headers: &HeaderMap) -> Response {
    let rid = request_id(headers);
    match error {
        StorageError::InvalidInput => {
            code_error("INVALID_PARAMETER", "invalid stream request", &rid)
        }
        StorageError::LeaseNotFound
        | StorageError::LeaseSessionMismatch
        | StorageError::LeaseReleased
        | StorageError::LeaseExpired
        | StorageError::MembershipNotReady => {
            code_error("RESOURCE_NOT_FOUND", "resource not found", &rid)
        }
        StorageError::SequenceConflict => api_error(
            StatusCode::CONFLICT,
            "STREAM_LEASE_SEQUENCE_CONFLICT",
            "lease sequence conflict",
            &rid,
            None,
        ),
        StorageError::IdempotencyMismatch => {
            code_error("IDEMPOTENCY_KEY_MISMATCH", "idempotency key mismatch", &rid)
        }
        StorageError::LeaseCapacity | StorageError::IdentityCapacity => api_error(
            StatusCode::CONFLICT,
            "STREAM_LEASE_CAPACITY",
            "stream capacity exceeded",
            &rid,
            None,
        ),
        StorageError::SessionInvalid => code_error("SESSION_UNKNOWN", "session required", &rid),
        StorageError::RightsInvalid => code_error("FORBIDDEN", "stream access unavailable", &rid),
        _ => unavailable(headers),
    }
}

fn key(headers: &HeaderMap) -> Result<String, Response> {
    let values = headers.get_all(idempotency::HEADER);
    let mut iter = values.iter();
    let raw = iter.next().and_then(|v| v.to_str().ok()).filter(|v| {
        !v.is_empty()
            && v.len() <= idempotency::MAX_KEY_BYTES
            && v.bytes().all(|b| b.is_ascii_graphic())
    });
    if iter.next().is_some() || raw.is_none() {
        return Err(code_error(
            "INVALID_PARAMETER",
            "idempotency key required",
            &request_id(headers),
        ));
    }
    Ok(raw.unwrap().to_owned())
}

async fn body(req: Request, headers: &HeaderMap) -> Result<Bytes, Response> {
    if req.headers().get_all(header::CONTENT_TYPE).iter().count() != 1
        || req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.split(';').next() != Some("application/json"))
    {
        return Err(code_error(
            "INVALID_PARAMETER",
            "JSON body required",
            &request_id(headers),
        ));
    }
    timeout(
        Duration::from_secs(5),
        to_bytes(req.into_body(), MAX_REQUEST_BYTES),
    )
    .await
    .map_err(|_| {
        code_error(
            "INVALID_PARAMETER",
            "request body timed out",
            &request_id(headers),
        )
    })?
    .map_err(|_| {
        api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "stream body too large",
            &request_id(headers),
            None,
        )
    })
}

fn private_response(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn create_or_renew(
    State(state): State<ApiState>,
    session: Session,
    OriginalUri(uri): OriginalUri,
    req: Request,
) -> Response {
    let headers = req.headers().clone();
    let pins = match configuration(&state, &session, &headers, &uri) {
        Ok(v) => v.clone(),
        Err(r) => return r,
    };
    if let Err(r) = require_csrf(&headers, &session.0) {
        return r;
    }
    if uri.query().is_some() {
        return code_error(
            "INVALID_PARAMETER",
            "stream lease query is not supported",
            &request_id(&headers),
        );
    }
    let key = match key(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let identity = match SessionIdentity::new(&session, &headers) {
        Ok(v) => v,
        Err(_) => return code_error("SESSION_UNKNOWN", "session required", &request_id(&headers)),
    };
    let bytes = match body(req, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let request = match contract::parse_lease(&bytes).and_then(|v| v.into_storage(key)) {
        Ok(v) => v,
        Err(_) => {
            return code_error(
                "INVALID_PARAMETER",
                "invalid stream lease",
                &request_id(&headers),
            );
        }
    };
    let repo = OwnerMarketStreamRepo::new(state.app_pool.clone(), pins);
    let result = timeout(
        Duration::from_secs(5),
        repo.create_or_renew(&session.actor(), &identity.hash, &request),
    )
    .await;
    private_response(match result {
        Ok(Ok(lease)) => match LeaseDto::try_from(lease) {
            Ok(dto) => axum::Json(dto).into_response(),
            Err(_) => unavailable(&headers),
        },
        Ok(Err(error)) => storage_error(error, &headers),
        Err(_) => unavailable(&headers),
    })
}

pub async fn release(
    State(state): State<ApiState>,
    session: Session,
    OriginalUri(uri): OriginalUri,
    req: Request,
) -> Response {
    let headers = req.headers().clone();
    let pins = match configuration(&state, &session, &headers, &uri) {
        Ok(v) => v.clone(),
        Err(r) => return r,
    };
    if let Err(r) = require_csrf(&headers, &session.0) {
        return r;
    }
    let key = match key(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(raw) = uri.path().strip_prefix(&format!("{PREFIX}/stream-leases/")) else {
        return code_error(
            "RESOURCE_NOT_FOUND",
            "resource not found",
            &request_id(&headers),
        );
    };
    let lease = match contract::canonical_uuid(raw) {
        Ok(v) if uri.query().is_none() => v,
        _ => {
            return code_error(
                "INVALID_PARAMETER",
                "invalid lease id",
                &request_id(&headers),
            );
        }
    };
    let identity = match SessionIdentity::new(&session, &headers) {
        Ok(v) => v,
        Err(_) => return code_error("SESSION_UNKNOWN", "session required", &request_id(&headers)),
    };
    let bytes = match body(req, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let request = match contract::parse_release(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return code_error(
                "INVALID_PARAMETER",
                "invalid release",
                &request_id(&headers),
            );
        }
    };
    let replay_key = format!(
        "stream-release:{}:{}:{lease}:{key}",
        identity.owner, identity.hash
    );
    let hash = idempotency::body_hash(
        &serde_json::json!({"consumer_id":request.consumer_id,"renewal_sequence":request.renewal_sequence,"schema_version":2}),
    );
    let gate = state.idempotency.gate(&replay_key);
    let Ok(_guard) = timeout(Duration::from_secs(5), gate.lock()).await else {
        return unavailable(&headers);
    };
    if state
        .idempotency
        .get(&replay_key)
        .is_some_and(|old| old.body_hash != hash)
    {
        return code_error(
            "IDEMPOTENCY_KEY_MISMATCH",
            "idempotency key mismatch",
            &request_id(&headers),
        );
    }
    // A replay still enters the session/consumer-bound durable tombstone
    // operation. A cached HTTP response never replaces current authorization.
    let repo = OwnerMarketStreamRepo::new(state.app_pool.clone(), pins);
    let result = timeout(
        Duration::from_secs(5),
        repo.release(
            &session.actor(),
            &identity.hash,
            lease,
            request.consumer_id,
            request.renewal_sequence,
        ),
    )
    .await;
    private_response(match result {
        Ok(Ok(outcome)) => {
            let body = serde_json::json!({"schema_version":2,"lease_id":outcome.lease_id,"released":outcome.released});
            state.idempotency.insert(
                &replay_key,
                CachedResult {
                    body_hash: hash,
                    status: StatusCode::OK,
                    body: body.clone(),
                },
            );
            axum::Json(body).into_response()
        }
        Ok(Err(error)) => storage_error(error, &headers),
        Err(_) => unavailable(&headers),
    })
}

fn lease_query(uri: &Uri, headers: &HeaderMap) -> Result<Uuid, InvalidContract> {
    let query = uri
        .query()
        .filter(|v| v.len() <= 80)
        .ok_or(InvalidContract)?;
    let raw = query.strip_prefix("lease_id=").ok_or(InvalidContract)?;
    let lease = contract::canonical_uuid(raw)?;
    // A cursor is syntactically bounded only. It is never replay authority.
    let mut cursors = headers.get_all("last-event-id").iter();
    let cursor = cursors.next();
    if cursors.next().is_some() {
        return Err(InvalidContract);
    }
    if let Some(cursor) = cursor {
        let cursor = cursor.to_str().map_err(|_| InvalidContract)?;
        if cursor.len() > 80 {
            return Err(InvalidContract);
        }
        let (stream, sequence) = cursor.split_once(':').ok_or(InvalidContract)?;
        contract::canonical_uuid(stream)?;
        if sequence.is_empty()
            || sequence.starts_with('0')
            || !sequence.bytes().all(|b| b.is_ascii_digit())
            || sequence.parse::<i64>().ok().is_none_or(|v| v <= 0)
        {
            return Err(InvalidContract);
        }
    }
    Ok(lease)
}

async fn snapshot(
    state: &ApiState,
    identity: &SessionIdentity,
    repo: &OwnerMarketStreamRepo,
    lease: Uuid,
    read_gate: &Mutex<()>,
) -> Result<StreamSnapshot, EndReason> {
    // A single deadline covers serialization, canonical session lookup, the
    // actor/RLS snapshot and final deployment-binding check. Timer ticks do
    // not cancel/restart an in-flight read or permit an older read to win.
    timeout(READ_DEADLINE, async {
        let _read = read_gate.lock().await;
        let actor = identity.authorize(state).await?;
        let snapshot = repo
            .snapshot(&actor, &identity.hash, lease)
            .await
            .map_err(read_error)?;
        repo.check_binding(&actor, &identity.hash)
            .await
            .map_err(read_error)?;
        Ok(snapshot)
    })
    .await
    .map_err(|_| EndReason::Unavailable)?
}

fn read_error(error: StorageError) -> EndReason {
    match error {
        StorageError::SessionInvalid
        | StorageError::RightsInvalid
        | StorageError::LeaseNotFound
        | StorageError::LeaseSessionMismatch
        | StorageError::LeaseReleased
        | StorageError::LeaseExpired
        | StorageError::MembershipNotReady => EndReason::AccessRevoked,
        _ => EndReason::Unavailable,
    }
}

struct ResponseOwner {
    task: Option<JoinHandle<()>>,
    signals: Arc<DeliverySignals>,
    _permit: ConsumerPermit,
}

impl Drop for ResponseOwner {
    fn drop(&mut self) {
        self.signals.closed.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct ResponseStream {
    receiver: watch::Receiver<Option<Arc<Batch>>>,
    owner: ResponseOwner,
    state: ApiState,
    identity: Arc<SessionIdentity>,
    repo: OwnerMarketStreamRepo,
    lease: Uuid,
    read_gate: Arc<Mutex<()>>,
    has_window: bool,
    encoder: DeliveryEncoder,
    last_comment: Instant,
    last_pulse: u64,
    stream_id: Uuid,
    end_reason: Arc<AtomicU8>,
    finished: bool,
}

impl ResponseStream {
    async fn finish(&mut self, reason: EndReason) -> Option<Bytes> {
        self.finished = true;
        self.owner.signals.closed.store(true, Ordering::Release);
        if let Some(mut task) = self.owner.task.take() {
            task.abort();
            let _ = (&mut task).await;
        }
        let sequence = self.encoder.last_sequence().checked_add(1)?;
        let body = StatusBody {
            connection: "STOPPED",
            reason_code: Some(match reason {
                EndReason::AccessRevoked => "ACCESS_REVOKED",
                EndReason::Unavailable => "PRODUCER_UNAVAILABLE",
            }),
            gap_open: false,
            session_has_gap: false,
            gap_generation: "0".into(),
        };
        let json = contract::event_json(
            self.stream_id,
            sequence,
            (self.state.cfg.intraday_now)(),
            &body,
        )
        .ok()?;
        Some(Bytes::from(format!(
            "event: status\nid: {}:{sequence}\ndata: {json}\n\n",
            self.stream_id
        )))
    }

    async fn next(&mut self) -> Option<Bytes> {
        if self.finished {
            return None;
        }
        loop {
            if self.owner.signals.closed.load(Ordering::Acquire)
                || self.receiver.changed().await.is_err()
                || self.owner.signals.closed.load(Ordering::Acquire)
            {
                let reason =
                    if self.end_reason.load(Ordering::Acquire) == EndReason::AccessRevoked as u8 {
                        EndReason::AccessRevoked
                    } else {
                        EndReason::Unavailable
                    };
                return self.finish(reason).await;
            }
            // No quote bytes are queued by the watchdog. Read and authorize
            // the complete current set immediately before serialization,
            // including lease, current generation, rights, day and source pins.
            let current = match snapshot(
                &self.state,
                &self.identity,
                &self.repo,
                self.lease,
                &self.read_gate,
            )
            .await
            {
                Ok(value) => value,
                Err(reason) => return self.finish(reason).await,
            };
            if self.owner.signals.closed.load(Ordering::Acquire) {
                continue;
            }
            let pulse = self
                .receiver
                .borrow_and_update()
                .as_ref()
                .map(|v| v.last_sequence);
            let Some(pulse) = pulse else {
                continue;
            };
            let rows = match projection::rows(&current, self.has_window) {
                Ok(value) => value,
                Err(_) => return self.finish(EndReason::Unavailable).await,
            };
            let status = match projection::status(&rows) {
                Ok(value) => value,
                Err(_) => return self.finish(EndReason::Unavailable).await,
            };
            let heartbeat = self.last_comment.elapsed() >= Duration::from_secs(15);
            let batch = self.encoder.encode(
                self.lease,
                current.lease_expires_at,
                rows,
                status,
                (self.state.cfg.intraday_now)(),
                pulse > self.last_pulse.saturating_add(1),
                heartbeat,
            );
            let batch = match batch {
                Ok(value) => value,
                Err(_) => return self.finish(EndReason::Unavailable).await,
            };
            if heartbeat {
                self.last_comment = Instant::now();
            }
            self.last_pulse = pulse;
            self.owner.signals.delivered.store(pulse, Ordering::Release);
            return Some(batch.bytes);
        }
    }
}

async fn drive(
    state: ApiState,
    identity: Arc<SessionIdentity>,
    repo: OwnerMarketStreamRepo,
    mut listener: PgListener,
    lease: Uuid,
    read_gate: Arc<Mutex<()>>,
    mut mailbox: LatestMailbox,
    end: Arc<AtomicU8>,
) {
    let result = async {
        let mut pulse = 0_u64;
        let mut listening = true;
        let mut dirty = true;
        let mut timer = tokio::time::interval(Duration::from_millis(250));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            // Revalidate independently of HTTP backpressure. Only a bounded
            // wake ordinal crosses the mailbox, never cached prices or auth.
            let _ = snapshot(&state, &identity, &repo, lease, &read_gate).await?;
            pulse = pulse.checked_add(1).ok_or(EndReason::Unavailable)?;
            mailbox.publish(Batch { bytes: Bytes::new(), last_sequence: pulse }, Instant::now())
                .map_err(|_| EndReason::Unavailable)?;
            let last_read = Instant::now();
            loop {
                tokio::select! {
                    _ = timer.tick() => {
                        if mailbox.signals.closed.load(Ordering::Acquire) || mailbox.unwritable(Instant::now()) {
                            return Err(EndReason::Unavailable);
                        }
                        if dirty || last_read.elapsed() >= Duration::from_secs(1) { break; }
                    }
                    notification = listener.try_recv(), if listening => {
                        dirty = true;
                        if !matches!(notification, Ok(Some(_))) { listening = false; }
                    }
                }
            }
            dirty = false;
        }
        #[allow(unreachable_code)] Ok::<(), EndReason>(())
    }.await;
    end.store(
        result.err().unwrap_or(EndReason::Unavailable) as u8,
        Ordering::Release,
    );
    mailbox.close();
    let _ = timeout(READ_DEADLINE, listener.unlisten_all()).await;
}

pub async fn get_stream(
    State(state): State<ApiState>,
    session: Session,
    OriginalUri(uri): OriginalUri,
    req: Request,
) -> Response {
    let headers = req.headers().clone();
    let pins = match configuration(&state, &session, &headers, &uri) {
        Ok(v) => v.clone(),
        Err(r) => return r,
    };
    let lease = match lease_query(&uri, &headers) {
        Ok(v) => v,
        Err(_) => {
            return code_error(
                "INVALID_PARAMETER",
                "invalid stream query",
                &request_id(&headers),
            );
        }
    };
    let identity = match SessionIdentity::new(&session, &headers) {
        Ok(v) => Arc::new(v),
        Err(_) => return code_error("SESSION_UNKNOWN", "session required", &request_id(&headers)),
    };
    let permit = match state.market_stream_consumers.acquire(identity.owner) {
        Ok(v) => v,
        Err(_) => {
            return api_error(
                StatusCode::CONFLICT,
                "STREAM_CONSUMER_CAPACITY",
                "stream consumer capacity exceeded",
                &request_id(&headers),
                None,
            );
        }
    };
    let repo = OwnerMarketStreamRepo::new(state.app_pool.clone(), pins.clone());
    // LISTEN is installed before the consistent RLS snapshot. This connection
    // is never used for application queries, so notifications carry no rows.
    let setup = async {
        let mut listener = PgListener::connect_with(&state.app_pool).await?;
        listener.eager_reconnect(false);
        listener.listen(NOTIFY_CHANNEL).await?;
        Ok::<_, sqlx::Error>(listener)
    };
    let mut listener = match timeout(READ_DEADLINE, setup).await {
        Ok(Ok(v)) => v,
        _ => return unavailable(&headers),
    };
    let initial = match timeout(
        READ_DEADLINE,
        repo.snapshot(&session.actor(), &identity.hash, lease),
    )
    .await
    {
        Ok(Ok(v)) => v,
        result => {
            let _ = timeout(READ_DEADLINE, listener.unlisten_all()).await;
            return match result {
                Ok(Err(error)) => storage_error(error, &headers),
                _ => unavailable(&headers),
            };
        }
    };
    drop(initial);
    let (mailbox, receiver) = LatestMailbox::new();
    let signals = mailbox.signals.clone();
    let stream_id = Uuid::new_v4();
    let end_reason = Arc::new(AtomicU8::new(0));
    let read_gate = Arc::new(Mutex::new(()));
    let task = tokio::spawn(drive(
        state.clone(),
        identity.clone(),
        repo.clone(),
        listener,
        lease,
        read_gate.clone(),
        mailbox,
        end_reason.clone(),
    ));
    let source = ResponseStream {
        receiver,
        owner: ResponseOwner {
            task: Some(task),
            signals,
            _permit: permit,
        },
        state,
        identity,
        repo,
        lease,
        read_gate,
        has_window: pins.window.is_some(),
        encoder: DeliveryEncoder::new(stream_id),
        last_comment: Instant::now(),
        last_pulse: 0,
        stream_id,
        end_reason,
        finished: false,
    };
    let stream = stream::unfold(source, |mut source| async move {
        source
            .next()
            .await
            .map(|bytes| (Ok::<_, Infallible>(bytes), source))
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}
