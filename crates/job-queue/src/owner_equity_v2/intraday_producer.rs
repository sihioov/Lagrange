//! Provider-free owner intraday quote producer.
//!
//! This module owns scheduling around the already reviewed storage and KIS
//! one-shot seams.  It never builds an arbitrary request, performs a second
//! retry loop inside the client, or invents budget/receipt evidence.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use collectors::intraday_quotes::{
    INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID, IntradayQuoteCaptureError,
    IntradaySessionWindowContract, intraday_quote_query, parse_intraday_attempt,
};
use kis_client::{
    CredentialSource, IntradayAttemptError, IntradayAttemptOutcome,
    IntradayAttemptReservationMetadata, KisMarketDataClient, Sleeper, Transport,
};
use market_data::intraday_quotes::IntradayQuoteError;
use thiserror::Error;
use tokio::sync::watch;
use uuid::Uuid;

use super::{
    IntradayAttemptReservation, IntradayPublicationContext, IntradayQuoteFailureCode,
    IntradayQuoteIdentity, IntradaySessionProof, IntradaySessionWindow, IntradayStorageError,
    OwnerIntradayQuoteRepository, ProducerClaimKind, ProducerLease,
};

const INTRADAY_HOLDER_NAMESPACE: Uuid = Uuid::from_u128(0x3b6a_2d1f_9c40_4e7b_8a65_0f21_c4d8_5e93);
pub const INTRADAY_MAX_ATTEMPTS_PER_CYCLE: usize = 3;
pub const INTRADAY_RETRY_DELAY: Duration = Duration::from_secs(5);
pub const INTRADAY_HALT_DELAY: Duration = Duration::from_secs(60);
pub const INTRADAY_PRODUCER_POLL: Duration = Duration::from_secs(1);

pub type EligibilityFuture = Pin<Box<dyn Future<Output = Result<bool, ()>> + Send>>;
pub type EligibilityCheck = Box<dyn FnOnce() -> EligibilityFuture + Send>;

/// The producer-facing reader seam.  The live implementation below delegates
/// to `KisMarketDataClient`; tests can provide a counting fake without any
/// provider, credential, or HTTP dependency.
#[async_trait]
pub trait IntradayQuoteReader: Send + Sync {
    async fn get_intraday_attempt(
        &self,
        query: &[(String, String)],
        eligibility_check: EligibilityCheck,
    ) -> IntradayAttemptOutcome;
}

#[async_trait]
impl<T, S, C> IntradayQuoteReader for KisMarketDataClient<T, S, C>
where
    T: Transport + 'static,
    S: Sleeper + 'static,
    C: CredentialSource + Send + Sync + 'static,
{
    async fn get_intraday_attempt(
        &self,
        query: &[(String, String)],
        eligibility_check: EligibilityCheck,
    ) -> IntradayAttemptOutcome {
        self.get_intraday_attempt_outcome(INTRADAY_QUOTE_PATH, INTRADAY_QUOTE_TR_ID, query, || {
            eligibility_check()
        })
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntradayProducerConfig {
    pub holder_id: Uuid,
    pub poll_interval: Duration,
    pub heartbeat_interval: Duration,
}

impl IntradayProducerConfig {
    pub fn for_worker(worker_id: &str) -> Result<Self, IntradayProducerError> {
        if worker_id.is_empty()
            || worker_id.len() > 128
            || !worker_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(IntradayProducerError::InvalidConfiguration);
        }
        Ok(Self {
            holder_id: Uuid::new_v5(&INTRADAY_HOLDER_NAMESPACE, worker_id.as_bytes()),
            poll_interval: INTRADAY_PRODUCER_POLL,
            heartbeat_interval: Duration::from_secs(5),
        })
    }

    pub fn new(holder_id: Uuid) -> Result<Self, IntradayProducerError> {
        if holder_id.is_nil() {
            return Err(IntradayProducerError::InvalidConfiguration);
        }
        Ok(Self {
            holder_id,
            poll_interval: INTRADAY_PRODUCER_POLL,
            heartbeat_interval: Duration::from_secs(5),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntradayProducerCycleReport {
    pub owners_seen: usize,
    pub owners_claimed: usize,
    pub attempts_started: usize,
    pub successful_quotes: usize,
    pub failures_recorded: usize,
    pub skipped: usize,
}

#[derive(Debug, Error)]
pub enum IntradayProducerError {
    #[error("INTRADAY_PRODUCER_CONFIGURATION_INVALID")]
    InvalidConfiguration,
    #[error("INTRADAY_PRODUCER_EVIDENCE_INVALID")]
    EvidenceInvalid,
    #[error("INTRADAY_PRODUCER_DATABASE")]
    Storage(#[from] IntradayStorageError),
    #[error("INTRADAY_PRODUCER_WINDOW")]
    Window(#[from] collectors::intraday_quotes::IntradaySessionWindowError),
}

pub struct IntradayProducer<R: IntradayQuoteReader> {
    repository: OwnerIntradayQuoteRepository,
    reader: Arc<R>,
    windows: Arc<IntradaySessionWindowContract>,
    config: IntradayProducerConfig,
    busy_until: Arc<tokio::sync::Mutex<Option<tokio::time::Instant>>>,
}

impl<R: IntradayQuoteReader> std::fmt::Debug for IntradayProducer<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntradayProducer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<R: IntradayQuoteReader + 'static> IntradayProducer<R> {
    pub fn new(
        repository: OwnerIntradayQuoteRepository,
        reader: Arc<R>,
        windows: Arc<IntradaySessionWindowContract>,
        config: IntradayProducerConfig,
    ) -> Self {
        Self {
            repository,
            reader,
            windows,
            config,
            busy_until: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub fn repository(&self) -> &OwnerIntradayQuoteRepository {
        &self.repository
    }

    /// Run one bounded worker cycle.  It discovers active owners, checks the
    /// exact current session proof, and gives at most three guarded attempts
    /// across the cycle.  The first due identity owns the cycle so a retry
    /// cannot starve the next identity's round-robin turn.
    pub async fn run_cycle(
        &self,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<IntradayProducerCycleReport, IntradayProducerError> {
        let owners = self.repository.active_demand_owners().await?;
        let mut report = IntradayProducerCycleReport {
            owners_seen: owners.len(),
            owners_claimed: 0,
            attempts_started: 0,
            successful_quotes: 0,
            failures_recorded: 0,
            skipped: 0,
        };
        if self.shared_gate_is_busy().await {
            report.skipped = report.owners_seen;
            return Ok(report);
        }
        let mut attempts_remaining = INTRADAY_MAX_ATTEMPTS_PER_CYCLE;
        for owner_user_id in owners {
            if *shutdown.borrow() || attempts_remaining == 0 {
                break;
            }
            let owner_report = self
                .run_owner(
                    owner_user_id,
                    shutdown,
                    &mut attempts_remaining,
                    &mut report,
                )
                .await?;
            if owner_report {
                report.owners_claimed += 1;
            }
            if self.shared_gate_is_busy().await {
                break;
            }
        }
        Ok(report)
    }

    /// Long-running daemon loop.  It is spawned independently from the EOD
    /// queue runner and only receives cancellation through its own watch copy.
    pub async fn run_daemon(
        &self,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), IntradayProducerError> {
        let mut ticker = tokio::time::interval(self.config.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                _ = ticker.tick() => {
                    let _ = self.run_cycle(&mut shutdown).await?;
                }
            }
        }
    }

    async fn run_owner(
        &self,
        owner_user_id: Uuid,
        shutdown: &mut watch::Receiver<bool>,
        attempts_remaining: &mut usize,
        report: &mut IntradayProducerCycleReport,
    ) -> Result<bool, IntradayProducerError> {
        let now = self.repository.current_database_time().await?;
        let window_hash = self.windows.window_contract_sha256().to_owned();
        let session = match self
            .repository
            .resolve_current_session_proof(owner_user_id, &window_hash)
            .await
        {
            Ok(Some(session)) => session,
            Ok(None)
            | Err(
                IntradayStorageError::CalendarProofUnavailable
                | IntradayStorageError::SessionProofInvalid,
            ) => {
                report.skipped += 1;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        if session.session_date
            != now
                .with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).expect("KST"))
                .date_naive()
        {
            report.skipped += 1;
            return Ok(false);
        }
        let Some(entry) = self.windows.entry(session.session_date) else {
            report.skipped += 1;
            return Ok(false);
        };
        if self.windows.state_at(now, false)
            == collectors::intraday_quotes::IntradayMarketState::Unknown
            || self.windows.state_at(now, false)
                == collectors::intraday_quotes::IntradayMarketState::Closed
        {
            report.skipped += 1;
            return Ok(false);
        }
        let Some((open_at, close_at)) = entry.utc_bounds() else {
            report.skipped += 1;
            return Ok(false);
        };
        let window = IntradaySessionWindow::new(session.session_date, open_at, close_at)?;
        let work = self.repository.active_quote_work(owner_user_id).await?;
        let Some(item) = work.into_iter().find(|item| due_for_attempt(item, now)) else {
            report.skipped += 1;
            return Ok(false);
        };
        let claim = match self
            .repository
            .claim_producer(owner_user_id, self.config.holder_id)
            .await
        {
            Ok(claim) => claim,
            Err(IntradayStorageError::ProducerLeaseHeld) => {
                report.skipped += 1;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let claimed = matches!(
            claim.kind,
            ProducerClaimKind::Acquired
                | ProducerClaimKind::AlreadyHeld
                | ProducerClaimKind::TakenOver
        );
        if !claimed {
            report.skipped += 1;
            return Ok(false);
        }

        let (lease_tx, lease_rx) = watch::channel(claim.lease);
        let (lease_alive_tx, lease_alive_rx) = watch::channel(true);
        let (heartbeat_stop_tx, heartbeat_stop_rx) = watch::channel(false);
        let heartbeat = tokio::spawn(heartbeat_loop(
            self.repository.clone(),
            lease_rx.clone(),
            lease_tx,
            lease_alive_tx,
            heartbeat_stop_rx,
            shutdown.clone(),
            self.config.heartbeat_interval,
        ));
        let result = self
            .run_identity(
                lease_rx,
                lease_alive_rx,
                item.identity,
                session,
                window,
                shutdown,
                attempts_remaining,
                report,
            )
            .await;
        let _ = heartbeat_stop_tx.send(true);
        let _ = heartbeat.await;
        result.map(|_| true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_identity(
        &self,
        lease_rx: watch::Receiver<ProducerLease>,
        mut lease_alive_rx: watch::Receiver<bool>,
        identity: IntradayQuoteIdentity,
        session: IntradaySessionProof,
        window: IntradaySessionWindow,
        shutdown: &mut watch::Receiver<bool>,
        attempts_remaining: &mut usize,
        report: &mut IntradayProducerCycleReport,
    ) -> Result<(), IntradayProducerError> {
        let mut attempts_for_identity = 0usize;
        let mut unauthorized_seen = false;
        loop {
            if *shutdown.borrow()
                || !*lease_alive_rx.borrow()
                || *attempts_remaining == 0
                || attempts_for_identity >= INTRADAY_MAX_ATTEMPTS_PER_CYCLE
            {
                return Ok(());
            }
            let lease = lease_rx.borrow().clone();
            let query = intraday_quote_query(identity.symbol())
                .map_err(|_| IntradayProducerError::EvidenceInvalid)?;
            let eligibility_repository = self.repository.clone();
            let eligibility_lease = lease.clone();
            let eligibility_identity = identity.clone();
            let eligibility_session = session.clone();
            let eligibility_window = window;
            let eligibility: EligibilityCheck = Box::new(move || {
                Box::pin(async move {
                    eligibility_repository
                        .quote_attempt_eligible(
                            &eligibility_lease,
                            &eligibility_identity,
                            &eligibility_session,
                            &eligibility_window,
                        )
                        .await
                        .map_err(|_| ())
                })
            });

            let reader = Arc::clone(&self.reader);
            let attempt = reader.get_intraday_attempt(&query, eligibility);
            let outcome = tokio::select! {
                outcome = attempt => outcome,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                    continue;
                }
                changed = lease_alive_rx.changed() => {
                    if changed.is_err() || !*lease_alive_rx.borrow() { return Ok(()); }
                    continue;
                }
            };
            *attempts_remaining -= 1;
            attempts_for_identity += 1;
            report.attempts_started += 1;

            let capture = parse_intraday_attempt(identity.symbol(), outcome);
            match capture {
                Ok(capture) => {
                    let (_, metadata) = capture.clone().into_parts();
                    let Ok(context) = context_from_metadata(
                        &lease,
                        &identity,
                        &session,
                        metadata.reservation_metadata(),
                    ) else {
                        // A KST rollover or an invalidated reservation can
                        // make a late response unusable.  It is discarded;
                        // no success/failure row may be fabricated from it.
                        return Ok(());
                    };
                    let Some(received_at) =
                        DateTime::from_timestamp_millis(metadata.received_at_ms())
                    else {
                        self.record_failure_or_stop(
                            &context,
                            IntradayQuoteFailureCode::ProviderResponseInvalid,
                            &window,
                            report,
                        )
                        .await?;
                        return Ok(());
                    };
                    match self
                        .repository
                        .publish_success_in_window(
                            &context,
                            &capture.quote,
                            super::IntradayQuoteReceipt::captured(received_at),
                            &window,
                        )
                        .await
                    {
                        Ok(_) => report.successful_quotes += 1,
                        Err(
                            IntradayStorageError::ProducerLeaseLost
                            | IntradayStorageError::ActiveDemandRequired
                            | IntradayStorageError::MembershipNotReady
                            | IntradayStorageError::SessionProofInvalid
                            | IntradayStorageError::CalendarProofUnavailable,
                        ) => return Ok(()),
                        Err(error) => return Err(error.into()),
                    }
                    return Ok(());
                }
                Err(error) => {
                    if matches!(error.attempt_error(), Some(IntradayAttemptError::Busy)) {
                        self.defer_after_shared_busy().await;
                    }
                    let retry_after = retry_after_for(&error);
                    let retryable = is_retryable(&error, &mut unauthorized_seen);
                    if let Some(reservation) = error.reservation() {
                        let Ok(context) =
                            context_from_metadata(&lease, &identity, &session, reservation)
                        else {
                            return Ok(());
                        };
                        let failure = failure_code(&error);
                        self.record_failure_or_stop(&context, failure, &window, report)
                            .await?;
                    }
                    if !retryable || *attempts_remaining == 0 {
                        return Ok(());
                    }
                    let delay = retry_after
                        .unwrap_or(INTRADAY_RETRY_DELAY)
                        .max(INTRADAY_RETRY_DELAY);
                    let db_delay = match chrono::Duration::from_std(delay) {
                        Ok(delay) => delay,
                        Err(_) => return Ok(()),
                    };
                    if !self
                        .repository
                        .quote_retry_allowed(&lease, &identity, &session, &window, db_delay)
                        .await?
                    {
                        return Ok(());
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                        }
                        changed = lease_alive_rx.changed() => {
                            if changed.is_err() || !*lease_alive_rx.borrow() { return Ok(()); }
                        }
                    }
                }
            }
        }
    }

    async fn record_failure_or_stop(
        &self,
        context: &IntradayPublicationContext,
        failure: IntradayQuoteFailureCode,
        window: &IntradaySessionWindow,
        report: &mut IntradayProducerCycleReport,
    ) -> Result<(), IntradayProducerError> {
        match self
            .repository
            .record_failure_in_window(context, failure, window)
            .await
        {
            Ok(_) => {
                report.failures_recorded += 1;
                Ok(())
            }
            Err(
                IntradayStorageError::ProducerLeaseLost
                | IntradayStorageError::ActiveDemandRequired
                | IntradayStorageError::MembershipNotReady
                | IntradayStorageError::SessionProofInvalid
                | IntradayStorageError::CalendarProofUnavailable,
            ) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn shared_gate_is_busy(&self) -> bool {
        let now = tokio::time::Instant::now();
        let mut busy_until = self.busy_until.lock().await;
        if busy_until.is_some_and(|until| until > now) {
            return true;
        }
        *busy_until = None;
        false
    }

    async fn defer_after_shared_busy(&self) {
        let until = tokio::time::Instant::now() + INTRADAY_RETRY_DELAY;
        let mut busy_until = self.busy_until.lock().await;
        *busy_until = Some(until);
    }
}

async fn heartbeat_loop(
    repository: OwnerIntradayQuoteRepository,
    lease_rx: watch::Receiver<ProducerLease>,
    lease_tx: watch::Sender<ProducerLease>,
    lease_alive_tx: watch::Sender<bool>,
    mut stop_rx: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
    heartbeat_interval: Duration,
) {
    let mut ticker = tokio::time::interval(heartbeat_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() { return; }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
            }
            _ = ticker.tick() => {
                let lease = lease_rx.borrow().clone();
                match repository.heartbeat_producer(&lease).await {
                    Ok(updated) => {
                        let _ = lease_tx.send(updated);
                    }
                    Err(_) => {
                        let _ = lease_alive_tx.send(false);
                        return;
                    }
                }
            }
        }
    }
}

fn due_for_attempt(item: &super::IntradayQuoteWorkItem, now: DateTime<Utc>) -> bool {
    let delay = if item.halted {
        chrono::Duration::seconds(INTRADAY_HALT_DELAY.as_secs() as i64)
    } else {
        chrono::Duration::seconds(INTRADAY_RETRY_DELAY.as_secs() as i64)
    };
    item.last_attempt_at
        .map(|last| last <= now && now.signed_duration_since(last) >= delay)
        .unwrap_or(true)
}

fn context_from_metadata(
    lease: &ProducerLease,
    identity: &IntradayQuoteIdentity,
    session: &IntradaySessionProof,
    metadata: IntradayAttemptReservationMetadata,
) -> Result<IntradayPublicationContext, IntradayProducerError> {
    let date = NaiveDate::parse_from_str(metadata.kst_date(), "%Y-%m-%d")
        .map_err(|_| IntradayProducerError::EvidenceInvalid)?;
    let attempt = IntradayAttemptReservation::from_shared_evidence(
        identity.owner_user_id,
        lease.holder_id,
        date,
        u64::from(metadata.daily_attempt_ordinal()),
        metadata.reservation_fence(),
    )
    .map_err(|_| IntradayProducerError::EvidenceInvalid)?;
    IntradayPublicationContext::new(lease.clone(), identity.clone(), session.clone(), attempt)
        .map_err(|_| IntradayProducerError::EvidenceInvalid)
}

fn failure_code(error: &IntradayQuoteCaptureError) -> IntradayQuoteFailureCode {
    if let Some(response_error) = error.response_error() {
        return match response_error {
            IntradayQuoteError::ProviderResponseInvalid => {
                IntradayQuoteFailureCode::ProviderResponseInvalid
            }
            IntradayQuoteError::QuoteValueInvalid => IntradayQuoteFailureCode::QuoteValueInvalid,
        };
    }
    match error.attempt_error() {
        Some(IntradayAttemptError::Timeout | IntradayAttemptError::DispatchWindowMissed) => {
            IntradayQuoteFailureCode::ProviderTimeout
        }
        Some(IntradayAttemptError::RateLimited { .. }) => {
            IntradayQuoteFailureCode::ProviderRateLimited
        }
        Some(IntradayAttemptError::ResponseInvalid) => {
            IntradayQuoteFailureCode::ProviderResponseInvalid
        }
        Some(
            IntradayAttemptError::ProviderUnavailable
            | IntradayAttemptError::Transport
            | IntradayAttemptError::Unauthorized,
        ) => IntradayQuoteFailureCode::ProviderUnavailable,
        Some(IntradayAttemptError::CallerIneligible) => IntradayQuoteFailureCode::NoActiveDemand,
        Some(IntradayAttemptError::CallerEligibilityFailed)
        | Some(IntradayAttemptError::Coordination(_))
        | Some(IntradayAttemptError::UnsupportedEndpoint)
        | Some(IntradayAttemptError::SharedReadRequired)
        | Some(IntradayAttemptError::Busy)
        | None => IntradayQuoteFailureCode::ProducerUnavailable,
    }
}

fn is_retryable(error: &IntradayQuoteCaptureError, unauthorized_seen: &mut bool) -> bool {
    match error.attempt_error() {
        Some(IntradayAttemptError::Timeout | IntradayAttemptError::Transport)
        | Some(IntradayAttemptError::ProviderUnavailable)
        | Some(IntradayAttemptError::RateLimited { .. }) => true,
        Some(IntradayAttemptError::Unauthorized) if !*unauthorized_seen => {
            *unauthorized_seen = true;
            true
        }
        _ => false,
    }
}

fn retry_after_for(error: &IntradayQuoteCaptureError) -> Option<Duration> {
    match error.attempt_error() {
        Some(IntradayAttemptError::RateLimited { retry_after_ms }) => {
            Some(Duration::from_millis(*retry_after_ms))
        }
        _ => None,
    }
}
