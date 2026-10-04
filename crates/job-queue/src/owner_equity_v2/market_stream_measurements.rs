//! Bounded, opt-in QA observations for owner market-stream publication calls.
//!
//! This module records metadata only. It is enabled for tests or the dedicated
//! database-test feature and is not part of the default runtime surface.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_PUBLICATION_RECORDS: usize = 12_000;
const MAX_PUBLICATION_ROWS: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPublicationProbeError {
    InvalidSlot,
    AlreadyRegistered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamPublicationProbeOutcome {
    InFlight,
    Committed,
    ConfirmedByReread,
    CommitUncertain,
    DroppedBeforeCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPublicationProbeRecord {
    pub sequence: u64,
    pub started_elapsed_ns: u64,
    pub finished_elapsed_ns: Option<u64>,
    pub planned_rows: u8,
    pub changed_rows: u8,
    pub outcome: StreamPublicationProbeOutcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPublicationProbeSummary {
    pub record_count: u64,
    pub in_flight: u64,
    pub committed: u64,
    pub confirmed_by_reread: u64,
    pub commit_uncertain: u64,
    pub dropped_before_commit: u64,
    pub attempted_changed_rows: u64,
    pub known_committed_changed_rows: u64,
    pub overflowed: bool,
    pub invalid_record: bool,
    pub diagnostic_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferOfferOutcome {
    Buffered,
    Replaced,
    Stale,
    OfferAgeLag,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamBufferProbeSummary {
    pub adapter_calls: u64,
    pub buffered: u64,
    pub replaced: u64,
    pub stale: u64,
    pub offer_age_lag: u64,
    pub rejected: u64,
    pub handoff_age_lag_drops: u64,
    pub peak_pending_slots: u8,
    pub peak_high_water_slots: u8,
    pub peak_outstanding_slots: u8,
    pub overflowed: bool,
    pub diagnostic_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPublicationProbeSnapshot {
    pub summary: StreamPublicationProbeSummary,
    pub records: Vec<StreamPublicationProbeRecord>,
}

struct ProbeState {
    credential_slot_id: Uuid,
    registered_at: Instant,
    records: Vec<StreamPublicationProbeRecord>,
    next_sequence: Option<u64>,
    overflowed: bool,
    invalid_record: bool,
    diagnostic_error: bool,
    buffer_summary: StreamBufferProbeSummary,
}

impl ProbeState {
    fn new(credential_slot_id: Uuid, registered_at: Instant) -> Self {
        Self {
            credential_slot_id,
            registered_at,
            records: Vec::with_capacity(MAX_PUBLICATION_RECORDS),
            next_sequence: Some(1),
            overflowed: false,
            invalid_record: false,
            diagnostic_error: false,
            buffer_summary: StreamBufferProbeSummary::default(),
        }
    }

    fn increment_buffer_counter(&mut self, counter: BufferCounter, amount: u64) {
        let current = match counter {
            BufferCounter::AdapterCalls => self.buffer_summary.adapter_calls,
            BufferCounter::Buffered => self.buffer_summary.buffered,
            BufferCounter::Replaced => self.buffer_summary.replaced,
            BufferCounter::Stale => self.buffer_summary.stale,
            BufferCounter::OfferAgeLag => self.buffer_summary.offer_age_lag,
            BufferCounter::Rejected => self.buffer_summary.rejected,
            BufferCounter::HandoffAgeLagDrops => self.buffer_summary.handoff_age_lag_drops,
        };
        let next = current.checked_add(amount);
        match (counter, next) {
            (BufferCounter::AdapterCalls, Some(value)) => {
                self.buffer_summary.adapter_calls = value;
            }
            (BufferCounter::Buffered, Some(value)) => self.buffer_summary.buffered = value,
            (BufferCounter::Replaced, Some(value)) => self.buffer_summary.replaced = value,
            (BufferCounter::Stale, Some(value)) => self.buffer_summary.stale = value,
            (BufferCounter::OfferAgeLag, Some(value)) => self.buffer_summary.offer_age_lag = value,
            (BufferCounter::Rejected, Some(value)) => self.buffer_summary.rejected = value,
            (BufferCounter::HandoffAgeLagDrops, Some(value)) => {
                self.buffer_summary.handoff_age_lag_drops = value;
            }
            (counter, None) => {
                match counter {
                    BufferCounter::AdapterCalls => self.buffer_summary.adapter_calls = u64::MAX,
                    BufferCounter::Buffered => self.buffer_summary.buffered = u64::MAX,
                    BufferCounter::Replaced => self.buffer_summary.replaced = u64::MAX,
                    BufferCounter::Stale => self.buffer_summary.stale = u64::MAX,
                    BufferCounter::OfferAgeLag => self.buffer_summary.offer_age_lag = u64::MAX,
                    BufferCounter::Rejected => self.buffer_summary.rejected = u64::MAX,
                    BufferCounter::HandoffAgeLagDrops => {
                        self.buffer_summary.handoff_age_lag_drops = u64::MAX;
                    }
                }
                self.buffer_summary.overflowed = true;
                self.buffer_summary.diagnostic_error = true;
                self.diagnostic_error = true;
            }
        }
    }

    fn record_buffer_offer(
        &mut self,
        outcome: BufferOfferOutcome,
        pending: u8,
        high_water: u8,
        outstanding: u8,
    ) {
        self.increment_buffer_counter(BufferCounter::AdapterCalls, 1);
        let counter = match outcome {
            BufferOfferOutcome::Buffered => BufferCounter::Buffered,
            BufferOfferOutcome::Replaced => BufferCounter::Replaced,
            BufferOfferOutcome::Stale => BufferCounter::Stale,
            BufferOfferOutcome::OfferAgeLag => BufferCounter::OfferAgeLag,
            BufferOfferOutcome::Rejected => BufferCounter::Rejected,
        };
        self.increment_buffer_counter(counter, 1);
        self.record_buffer_occupancy(pending, high_water, outstanding);
    }

    fn record_buffer_handoff(
        &mut self,
        age_before: u64,
        age_after: u64,
        pending: u8,
        high_water: u8,
        outstanding: u8,
    ) {
        self.record_buffer_occupancy(pending, high_water, outstanding);
        if age_before == u64::MAX || age_after == u64::MAX {
            self.buffer_summary.overflowed = true;
            self.buffer_summary.diagnostic_error = true;
            self.diagnostic_error = true;
            return;
        }
        let Some(age_delta) = age_after.checked_sub(age_before) else {
            self.buffer_summary.diagnostic_error = true;
            self.diagnostic_error = true;
            return;
        };
        self.increment_buffer_counter(BufferCounter::HandoffAgeLagDrops, age_delta);
    }

    fn record_buffer_occupancy(&mut self, pending: u8, high_water: u8, outstanding: u8) {
        let maximum = MAX_PUBLICATION_ROWS as u8;
        if pending > maximum || high_water > maximum || outstanding > maximum {
            self.buffer_summary.diagnostic_error = true;
            self.diagnostic_error = true;
            return;
        }
        self.buffer_summary.peak_pending_slots =
            self.buffer_summary.peak_pending_slots.max(pending);
        self.buffer_summary.peak_high_water_slots =
            self.buffer_summary.peak_high_water_slots.max(high_water);
        self.buffer_summary.peak_outstanding_slots =
            self.buffer_summary.peak_outstanding_slots.max(outstanding);
    }

    fn elapsed_ns(&mut self, now: Instant) -> u64 {
        let Some(elapsed) = now.checked_duration_since(self.registered_at) else {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return 0;
        };
        match u64::try_from(elapsed.as_nanos()) {
            Ok(value) => value,
            Err(_) => {
                self.overflowed = true;
                self.invalid_record = true;
                self.diagnostic_error = true;
                u64::MAX
            }
        }
    }

    fn begin_record(
        &mut self,
        planned_rows: usize,
        started_elapsed_ns: u64,
    ) -> Option<(usize, u64)> {
        if planned_rows == 0 || planned_rows > MAX_PUBLICATION_ROWS {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return None;
        }
        if self.records.len() >= MAX_PUBLICATION_RECORDS {
            self.overflowed = true;
            self.diagnostic_error = true;
            return None;
        }
        let Some(sequence) = self.next_sequence else {
            self.overflowed = true;
            self.diagnostic_error = true;
            return None;
        };
        let Some(next_sequence) = sequence.checked_add(1) else {
            self.overflowed = true;
            self.diagnostic_error = true;
            self.next_sequence = None;
            self.records.push(StreamPublicationProbeRecord {
                sequence,
                started_elapsed_ns,
                finished_elapsed_ns: None,
                planned_rows: planned_rows as u8,
                changed_rows: 0,
                outcome: StreamPublicationProbeOutcome::InFlight,
            });
            return Some((self.records.len() - 1, sequence));
        };
        self.next_sequence = Some(next_sequence);
        self.records.push(StreamPublicationProbeRecord {
            sequence,
            started_elapsed_ns,
            finished_elapsed_ns: None,
            planned_rows: planned_rows as u8,
            changed_rows: 0,
            outcome: StreamPublicationProbeOutcome::InFlight,
        });
        Some((self.records.len() - 1, sequence))
    }

    fn row_changed(&mut self, index: usize, sequence: u64) {
        let Some(record) = self.records.get(index) else {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return;
        };
        if record.sequence != sequence
            || record.outcome != StreamPublicationProbeOutcome::InFlight
            || record.finished_elapsed_ns.is_some()
            || record.changed_rows >= record.planned_rows
            || record.changed_rows as usize >= MAX_PUBLICATION_ROWS
        {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return;
        }
        let Some(changed_rows) = self.records[index].changed_rows.checked_add(1) else {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return;
        };
        self.records[index].changed_rows = changed_rows;
    }

    fn finish_record(
        &mut self,
        index: usize,
        sequence: u64,
        outcome: StreamPublicationProbeOutcome,
        finished_elapsed_ns: u64,
    ) {
        let Some(record) = self.records.get(index) else {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return;
        };
        if record.sequence != sequence
            || record.outcome != StreamPublicationProbeOutcome::InFlight
            || record.finished_elapsed_ns.is_some()
            || outcome == StreamPublicationProbeOutcome::InFlight
        {
            self.invalid_record = true;
            self.diagnostic_error = true;
            return;
        }
        let started_elapsed_ns = record.started_elapsed_ns;
        let effective_finished = if finished_elapsed_ns < started_elapsed_ns {
            self.invalid_record = true;
            self.diagnostic_error = true;
            started_elapsed_ns
        } else {
            finished_elapsed_ns
        };
        let record = &mut self.records[index];
        record.finished_elapsed_ns = Some(effective_finished);
        record.outcome = outcome;
    }

    fn summary(&self) -> StreamPublicationProbeSummary {
        let mut summary = StreamPublicationProbeSummary {
            overflowed: self.overflowed,
            invalid_record: self.invalid_record,
            diagnostic_error: self.diagnostic_error,
            ..StreamPublicationProbeSummary::default()
        };
        match u64::try_from(self.records.len()) {
            Ok(record_count) => add_counter(
                &mut summary.record_count,
                record_count,
                &mut summary.overflowed,
                &mut summary.diagnostic_error,
            ),
            Err(_) => {
                summary.record_count = u64::MAX;
                summary.overflowed = true;
                summary.diagnostic_error = true;
            }
        }
        for record in &self.records {
            if record.planned_rows == 0
                || record.planned_rows as usize > MAX_PUBLICATION_ROWS
                || record.changed_rows > record.planned_rows
                || (record.outcome == StreamPublicationProbeOutcome::InFlight)
                    != record.finished_elapsed_ns.is_none()
            {
                summary.invalid_record = true;
                summary.diagnostic_error = true;
            }
            let counter = match record.outcome {
                StreamPublicationProbeOutcome::InFlight => &mut summary.in_flight,
                StreamPublicationProbeOutcome::Committed => &mut summary.committed,
                StreamPublicationProbeOutcome::ConfirmedByReread => {
                    &mut summary.confirmed_by_reread
                }
                StreamPublicationProbeOutcome::CommitUncertain => &mut summary.commit_uncertain,
                StreamPublicationProbeOutcome::DroppedBeforeCommit => {
                    &mut summary.dropped_before_commit
                }
            };
            add_counter(
                counter,
                1,
                &mut summary.overflowed,
                &mut summary.diagnostic_error,
            );
            add_counter(
                &mut summary.attempted_changed_rows,
                record.changed_rows as u64,
                &mut summary.overflowed,
                &mut summary.diagnostic_error,
            );
            if record.outcome == StreamPublicationProbeOutcome::Committed {
                add_counter(
                    &mut summary.known_committed_changed_rows,
                    record.changed_rows as u64,
                    &mut summary.overflowed,
                    &mut summary.diagnostic_error,
                );
            }
        }
        summary
    }
}

#[derive(Clone, Copy)]
enum BufferCounter {
    AdapterCalls,
    Buffered,
    Replaced,
    Stale,
    OfferAgeLag,
    Rejected,
    HandoffAgeLagDrops,
}

fn add_counter(total: &mut u64, amount: u64, overflowed: &mut bool, diagnostic_error: &mut bool) {
    match total.checked_add(amount) {
        Some(value) => *total = value,
        None => {
            *total = u64::MAX;
            *overflowed = true;
            *diagnostic_error = true;
        }
    }
}

struct ProbeRegistry {
    current: Option<Weak<Mutex<ProbeState>>>,
    diagnostic_error: bool,
}

fn registry_mutex() -> &'static Mutex<ProbeRegistry> {
    static REGISTRY: OnceLock<Mutex<ProbeRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(ProbeRegistry {
            current: None,
            diagnostic_error: false,
        })
    })
}

fn lock_state(state: &Mutex<ProbeState>) -> MutexGuard<'_, ProbeState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.diagnostic_error = true;
            guard
        }
    }
}

fn lock_registry() -> MutexGuard<'static, ProbeRegistry> {
    match registry_mutex().lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.diagnostic_error = true;
            if let Some(state) = guard.current.as_ref().and_then(Weak::upgrade) {
                lock_state(&state).diagnostic_error = true;
            }
            guard
        }
    }
}

fn update_live_probe(slot: Uuid, update: impl FnOnce(&mut ProbeState)) {
    if slot.is_nil() {
        return;
    }
    let registry = lock_registry();
    let Some(state) = registry.current.as_ref().and_then(Weak::upgrade) else {
        return;
    };
    let mut state = lock_state(&state);
    if state.credential_slot_id != slot {
        return;
    }
    update(&mut state);
}

pub(super) fn record_buffer_offer(
    slot: Uuid,
    outcome: BufferOfferOutcome,
    pending: u8,
    high_water: u8,
    outstanding: u8,
) {
    update_live_probe(slot, |state| {
        state.record_buffer_offer(outcome, pending, high_water, outstanding);
    });
}

pub(super) fn record_buffer_handoff(
    slot: Uuid,
    age_before: u64,
    age_after: u64,
    pending: u8,
    high_water: u8,
    outstanding: u8,
) {
    update_live_probe(slot, |state| {
        state.record_buffer_handoff(age_before, age_after, pending, high_water, outstanding);
    });
}

/// Owns the single active slot registration. Dropping it unregisters only the
/// state this guard registered; traces can outlive the guard without retargeting.
pub struct StreamPublicationProbe {
    credential_slot_id: Uuid,
    state: Arc<Mutex<ProbeState>>,
}

impl StreamPublicationProbe {
    pub fn register(credential_slot_id: Uuid) -> Result<Self, StreamPublicationProbeError> {
        if credential_slot_id.is_nil() {
            return Err(StreamPublicationProbeError::InvalidSlot);
        }
        let mut registry = lock_registry();
        if registry.current.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(StreamPublicationProbeError::AlreadyRegistered);
        }
        registry.current = None;
        let mut state = ProbeState::new(credential_slot_id, Instant::now());
        state.diagnostic_error = registry.diagnostic_error;
        let state = Arc::new(Mutex::new(state));
        registry.current = Some(Arc::downgrade(&state));
        Ok(Self {
            credential_slot_id,
            state,
        })
    }

    /// Returns bounded aggregate metadata without cloning the event vector.
    pub fn summary(&self) -> Result<StreamPublicationProbeSummary, StreamPublicationProbeError> {
        let registry_diagnostic = {
            let registry = lock_registry();
            registry.diagnostic_error
                || !registry
                    .current
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|state| Arc::ptr_eq(&state, &self.state))
        };
        let mut state = lock_state(&self.state);
        if registry_diagnostic || state.credential_slot_id != self.credential_slot_id {
            state.diagnostic_error = true;
        }
        Ok(state.summary())
    }

    /// Returns bounded receipt-buffer counters without cloning publication events.
    pub fn buffer_summary(&self) -> Result<StreamBufferProbeSummary, StreamPublicationProbeError> {
        let registry_diagnostic = {
            let registry = lock_registry();
            registry.diagnostic_error
                || !registry
                    .current
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|state| Arc::ptr_eq(&state, &self.state))
        };
        let mut state = lock_state(&self.state);
        if registry_diagnostic || state.credential_slot_id != self.credential_slot_id {
            state.diagnostic_error = true;
        }
        let publication_diagnostic = state.summary().diagnostic_error;
        if publication_diagnostic {
            state.diagnostic_error = true;
        }
        let mut summary = state.buffer_summary.clone();
        summary.diagnostic_error |= state.diagnostic_error || publication_diagnostic;
        Ok(summary)
    }

    /// Copies the bounded event list for final QA evidence only.
    pub fn snapshot(&self) -> Result<StreamPublicationProbeSnapshot, StreamPublicationProbeError> {
        let registry_diagnostic = {
            let registry = lock_registry();
            registry.diagnostic_error
                || !registry
                    .current
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|state| Arc::ptr_eq(&state, &self.state))
        };
        let mut state = lock_state(&self.state);
        if registry_diagnostic || state.credential_slot_id != self.credential_slot_id {
            state.diagnostic_error = true;
        }
        Ok(StreamPublicationProbeSnapshot {
            summary: state.summary(),
            records: state.records.clone(),
        })
    }
}

impl Drop for StreamPublicationProbe {
    fn drop(&mut self) {
        let mut registry = lock_registry();
        let owns_current = registry
            .current
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|state| Arc::ptr_eq(&state, &self.state));
        if owns_current {
            registry.current = None;
        }
    }
}

pub(super) struct StreamPublicationTrace {
    state: Arc<Mutex<ProbeState>>,
    index: usize,
    sequence: u64,
    commit_started: bool,
    finished: bool,
}

impl StreamPublicationTrace {
    fn start(
        state: Arc<Mutex<ProbeState>>,
        planned_rows: usize,
        started_elapsed_ns: u64,
    ) -> Option<Self> {
        let (index, sequence) =
            lock_state(&state).begin_record(planned_rows, started_elapsed_ns)?;
        Some(Self {
            state,
            index,
            sequence,
            commit_started: false,
            finished: false,
        })
    }

    fn flag_invalid_protocol(&self) {
        let mut state = lock_state(&self.state);
        state.invalid_record = true;
        state.diagnostic_error = true;
    }

    pub(super) fn row_changed(&mut self) {
        if self.finished || self.commit_started {
            self.flag_invalid_protocol();
            return;
        }
        lock_state(&self.state).row_changed(self.index, self.sequence);
    }

    pub(super) fn commit_started(&mut self) {
        if self.finished || self.commit_started {
            self.flag_invalid_protocol();
            return;
        }
        self.commit_started = true;
    }

    fn finish_at_inner(
        &mut self,
        requested: StreamPublicationProbeOutcome,
        finished_elapsed_ns: u64,
    ) {
        if self.finished {
            self.flag_invalid_protocol();
            return;
        }
        let outcome = match (self.commit_started, requested) {
            (false, StreamPublicationProbeOutcome::DroppedBeforeCommit) => requested,
            (true, StreamPublicationProbeOutcome::Committed)
            | (true, StreamPublicationProbeOutcome::ConfirmedByReread)
            | (true, StreamPublicationProbeOutcome::CommitUncertain) => requested,
            (false, _) => {
                self.flag_invalid_protocol();
                StreamPublicationProbeOutcome::DroppedBeforeCommit
            }
            (true, _) => {
                self.flag_invalid_protocol();
                StreamPublicationProbeOutcome::CommitUncertain
            }
        };
        lock_state(&self.state).finish_record(
            self.index,
            self.sequence,
            outcome,
            finished_elapsed_ns,
        );
        self.finished = true;
    }

    pub(super) fn finish(mut self, outcome: StreamPublicationProbeOutcome) {
        let finished_elapsed_ns = lock_state(&self.state).elapsed_ns(Instant::now());
        self.finish_at_inner(outcome, finished_elapsed_ns);
    }

    #[cfg(test)]
    fn finish_at_for_test(mut self, outcome: StreamPublicationProbeOutcome, elapsed_ns: u64) {
        self.finish_at_inner(outcome, elapsed_ns);
    }

    #[cfg(test)]
    fn drop_at_for_test(mut self, elapsed_ns: u64) {
        let outcome = if self.commit_started {
            StreamPublicationProbeOutcome::CommitUncertain
        } else {
            StreamPublicationProbeOutcome::DroppedBeforeCommit
        };
        self.finish_at_inner(outcome, elapsed_ns);
    }
}

impl Drop for StreamPublicationTrace {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let outcome = if self.commit_started {
            StreamPublicationProbeOutcome::CommitUncertain
        } else {
            StreamPublicationProbeOutcome::DroppedBeforeCommit
        };
        let elapsed_ns = lock_state(&self.state).elapsed_ns(Instant::now());
        self.finish_at_inner(outcome, elapsed_ns);
    }
}

pub(super) fn begin_publication(
    credential_slot_id: Uuid,
    planned_rows: usize,
) -> Option<StreamPublicationTrace> {
    let registry = lock_registry();
    let state = registry.current.as_ref()?.upgrade()?;
    if lock_state(&state).credential_slot_id != credential_slot_id {
        return None;
    }
    let started_elapsed_ns = lock_state(&state).elapsed_ns(Instant::now());
    StreamPublicationTrace::start(state, planned_rows, started_elapsed_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    static REGISTRY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn registry_test_lock() -> std::sync::MutexGuard<'static, ()> {
        REGISTRY_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fresh_state(slot: Uuid) -> Arc<Mutex<ProbeState>> {
        Arc::new(Mutex::new(ProbeState::new(slot, Instant::now())))
    }

    #[test]
    fn publication_probe_requires_exact_registered_slot() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x100);
        let other_slot = Uuid::from_u128(0x101);
        assert!(begin_publication(slot, 1).is_none());
        assert!(matches!(
            StreamPublicationProbe::register(Uuid::nil()),
            Err(StreamPublicationProbeError::InvalidSlot)
        ));

        let probe = StreamPublicationProbe::register(slot).unwrap();
        assert!(begin_publication(other_slot, 1).is_none());
        let trace = begin_publication(slot, 1).unwrap();
        drop(trace);
        assert_eq!(probe.summary().unwrap().record_count, 1);
        assert!(matches!(
            StreamPublicationProbe::register(other_slot),
            Err(StreamPublicationProbeError::AlreadyRegistered)
        ));
        drop(probe);
        let replacement = StreamPublicationProbe::register(other_slot).unwrap();
        assert!(begin_publication(slot, 1).is_none());
        assert!(begin_publication(other_slot, 1).is_some());
        drop(replacement);
    }

    #[test]
    fn publication_probe_distinguishes_commit_and_drop_outcomes() {
        let _serial = registry_test_lock();
        let state = fresh_state(Uuid::from_u128(0x200));

        let mut committed = StreamPublicationTrace::start(Arc::clone(&state), 2, 10).unwrap();
        committed.row_changed();
        committed.row_changed();
        committed.commit_started();
        committed.finish_at_for_test(StreamPublicationProbeOutcome::Committed, 20);

        let before_commit = StreamPublicationTrace::start(Arc::clone(&state), 1, 30).unwrap();
        before_commit.drop_at_for_test(35);

        let mut cancelled_in_commit =
            StreamPublicationTrace::start(Arc::clone(&state), 1, 40).unwrap();
        cancelled_in_commit.row_changed();
        cancelled_in_commit.commit_started();
        cancelled_in_commit.drop_at_for_test(45);

        let mut reread = StreamPublicationTrace::start(Arc::clone(&state), 1, 50).unwrap();
        reread.commit_started();
        reread.finish_at_for_test(StreamPublicationProbeOutcome::ConfirmedByReread, 60);

        let mut unresolved = StreamPublicationTrace::start(Arc::clone(&state), 1, 70).unwrap();
        unresolved.commit_started();
        unresolved.finish_at_for_test(StreamPublicationProbeOutcome::CommitUncertain, 80);

        let summary = lock_state(&state).summary();
        assert_eq!(summary.record_count, 5);
        assert_eq!(summary.committed, 1);
        assert_eq!(summary.dropped_before_commit, 1);
        assert_eq!(summary.commit_uncertain, 2);
        assert_eq!(summary.confirmed_by_reread, 1);
        assert_eq!(summary.attempted_changed_rows, 3);
        assert_eq!(summary.known_committed_changed_rows, 2);
        assert!(!summary.invalid_record);
        assert!(!summary.diagnostic_error);
        assert!(!summary.overflowed);

        // Exercise the actual destructor, not only the deterministic finish kernel.
        let dropped_state = fresh_state(Uuid::from_u128(0x201));
        let mut actual_before =
            StreamPublicationTrace::start(Arc::clone(&dropped_state), 1, 0).unwrap();
        actual_before.row_changed();
        drop(actual_before);
        let mut actual_during =
            StreamPublicationTrace::start(Arc::clone(&dropped_state), 1, 0).unwrap();
        actual_during.row_changed();
        actual_during.commit_started();
        drop(actual_during);
        let dropped = lock_state(&dropped_state);
        let actual_summary = dropped.summary();
        assert_eq!(actual_summary.record_count, 2);
        assert_eq!(actual_summary.in_flight, 0);
        assert_eq!(actual_summary.dropped_before_commit, 1);
        assert_eq!(actual_summary.commit_uncertain, 1);
        assert_eq!(actual_summary.attempted_changed_rows, 2);
        assert_eq!(actual_summary.known_committed_changed_rows, 0);
        assert!(!actual_summary.invalid_record);
        assert!(!actual_summary.diagnostic_error);
        assert!(!actual_summary.overflowed);
        assert!(dropped.records.iter().all(|record| record
            .finished_elapsed_ns
            .is_some_and(|finished| finished >= record.started_elapsed_ns)));
    }

    #[test]
    fn publication_probe_keeps_fixed_capacity_and_flags_overflow() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x300);
        let state = fresh_state(slot);
        {
            let mut state = lock_state(&state);
            for index in 0..MAX_PUBLICATION_RECORDS {
                assert!(state.begin_record(1, index as u64).is_some());
            }
            assert_eq!(state.records.len(), MAX_PUBLICATION_RECORDS);
            assert!(state
                .begin_record(1, MAX_PUBLICATION_RECORDS as u64)
                .is_none());
            assert_eq!(state.records.len(), MAX_PUBLICATION_RECORDS);
            let summary = state.summary();
            assert_eq!(summary.record_count, MAX_PUBLICATION_RECORDS as u64);
            assert_eq!(summary.in_flight, MAX_PUBLICATION_RECORDS as u64);
            assert!(summary.overflowed);
            assert!(!summary.invalid_record);
            assert!(summary.diagnostic_error);
        }

        let row_state = fresh_state(Uuid::from_u128(0x301));
        let mut trace = StreamPublicationTrace::start(Arc::clone(&row_state), 1, 0).unwrap();
        trace.row_changed();
        trace.row_changed();
        trace.drop_at_for_test(1);
        let summary = lock_state(&row_state).summary();
        assert_eq!(summary.attempted_changed_rows, 1);
        assert!(summary.invalid_record);
        assert!(summary.diagnostic_error);

        let mut total = u64::MAX - 1;
        let mut overflowed = false;
        let mut diagnostic_error = false;
        add_counter(&mut total, 2, &mut overflowed, &mut diagnostic_error);
        assert_eq!(total, u64::MAX);
        assert!(overflowed);
        assert!(diagnostic_error);

        let sequence_state = fresh_state(Uuid::from_u128(0x302));
        {
            let mut state = lock_state(&sequence_state);
            state.next_sequence = Some(u64::MAX);
            assert!(state.begin_record(1, 0).is_some());
            assert!(state.summary().overflowed);
            assert!(state.summary().diagnostic_error);
            assert!(state.begin_record(1, 1).is_none());
            assert!(state.overflowed);
            assert!(state.diagnostic_error);
        }
    }

    #[test]
    fn publication_probe_registration_lifetime_isolated() {
        let _serial = registry_test_lock();
        let old_slot = Uuid::from_u128(0x400);
        let new_slot = Uuid::from_u128(0x401);
        let old_probe = StreamPublicationProbe::register(old_slot).unwrap();
        let mut old_trace = begin_publication(old_slot, 1).unwrap();
        old_trace.row_changed();
        let old_state = Arc::clone(&old_trace.state);
        drop(old_probe);

        let new_probe = StreamPublicationProbe::register(new_slot).unwrap();
        assert!(begin_publication(old_slot, 1).is_none());
        assert_eq!(new_probe.summary().unwrap().record_count, 0);
        old_trace.commit_started();
        old_trace.finish(StreamPublicationProbeOutcome::ConfirmedByReread);
        assert_eq!(new_probe.summary().unwrap().record_count, 0);
        assert_eq!(lock_state(&old_state).summary().confirmed_by_reread, 1);

        let mut new_trace = begin_publication(new_slot, 1).unwrap();
        new_trace.commit_started();
        new_trace.finish(StreamPublicationProbeOutcome::Committed);
        let snapshot = new_probe.snapshot().unwrap();
        assert_eq!(snapshot.records.len(), 1);
        assert_eq!(snapshot.summary.known_committed_changed_rows, 0);
        drop(new_probe);
        assert!(begin_publication(new_slot, 1).is_none());
    }

    #[test]
    fn buffer_probe_exact_slot_and_registration_lifetime() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x500);
        let replacement_slot = Uuid::from_u128(0x501);

        record_buffer_offer(slot, BufferOfferOutcome::Buffered, 1, 1, 0);
        record_buffer_handoff(slot, 0, 1, 1, 1, 0);
        record_buffer_offer(Uuid::nil(), BufferOfferOutcome::Rejected, 30, 30, 30);

        let probe = StreamPublicationProbe::register(slot).unwrap();
        assert_eq!(
            probe.buffer_summary().unwrap(),
            StreamBufferProbeSummary::default()
        );
        record_buffer_offer(replacement_slot, BufferOfferOutcome::Rejected, 30, 30, 30);
        record_buffer_handoff(replacement_slot, 0, 30, 30, 30, 30);
        assert_eq!(
            probe.buffer_summary().unwrap(),
            StreamBufferProbeSummary::default()
        );

        record_buffer_offer(slot, BufferOfferOutcome::Buffered, 2, 2, 0);
        assert_eq!(probe.buffer_summary().unwrap().adapter_calls, 1);
        drop(probe);

        record_buffer_offer(slot, BufferOfferOutcome::Replaced, 3, 3, 1);
        let fresh_same_slot = StreamPublicationProbe::register(slot).unwrap();
        assert_eq!(
            fresh_same_slot.buffer_summary().unwrap(),
            StreamBufferProbeSummary::default()
        );
        drop(fresh_same_slot);

        record_buffer_offer(slot, BufferOfferOutcome::Replaced, 3, 3, 1);
        let replacement = StreamPublicationProbe::register(replacement_slot).unwrap();
        record_buffer_handoff(slot, 0, 3, 3, 3, 1);
        assert_eq!(
            replacement.buffer_summary().unwrap(),
            StreamBufferProbeSummary::default()
        );
        record_buffer_offer(replacement_slot, BufferOfferOutcome::Stale, 1, 3, 0);
        assert_eq!(replacement.buffer_summary().unwrap().stale, 1);
    }

    #[test]
    fn buffer_probe_classifies_offers_and_handoff_drops() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x510);
        let probe = StreamPublicationProbe::register(slot).unwrap();

        for outcome in [
            BufferOfferOutcome::Buffered,
            BufferOfferOutcome::Replaced,
            BufferOfferOutcome::Stale,
            BufferOfferOutcome::OfferAgeLag,
            BufferOfferOutcome::Rejected,
        ] {
            record_buffer_offer(slot, outcome, 4, 8, 2);
        }
        record_buffer_handoff(slot, 2, 5, 7, 9, 10);

        let summary = probe.buffer_summary().unwrap();
        assert_eq!(summary.adapter_calls, 5);
        assert_eq!(summary.buffered, 1);
        assert_eq!(summary.replaced, 1);
        assert_eq!(summary.stale, 1);
        assert_eq!(summary.offer_age_lag, 1);
        assert_eq!(summary.rejected, 1);
        assert_eq!(summary.handoff_age_lag_drops, 3);
        assert!(!summary.diagnostic_error);
        assert_eq!(probe.summary().unwrap().record_count, 0);
    }

    #[test]
    fn buffer_probe_records_bounded_peaks_without_event_history() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x520);
        let probe = StreamPublicationProbe::register(slot).unwrap();

        for occupancy in 0..=MAX_PUBLICATION_ROWS as u8 {
            record_buffer_offer(slot, BufferOfferOutcome::Buffered, occupancy, occupancy, 0);
        }
        record_buffer_handoff(slot, 0, 0, 30, 30, 30);
        record_buffer_offer(slot, BufferOfferOutcome::Rejected, 31, 1, 1);

        let summary = probe.buffer_summary().unwrap();
        assert_eq!(summary.peak_pending_slots, 30);
        assert_eq!(summary.peak_high_water_slots, 30);
        assert_eq!(summary.peak_outstanding_slots, 30);
        assert_eq!(summary.adapter_calls, 32);
        assert!(summary.diagnostic_error);
        let snapshot = probe.snapshot().unwrap();
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.summary.record_count, 0);
    }

    #[test]
    fn buffer_probe_overflow_and_invalid_observations_fail_closed() {
        let _serial = registry_test_lock();
        let slot = Uuid::from_u128(0x530);
        let probe = StreamPublicationProbe::register(slot).unwrap();
        {
            let mut state = lock_state(&probe.state);
            state.buffer_summary.adapter_calls = u64::MAX - 1;
            state.buffer_summary.buffered = u64::MAX;
            state.buffer_summary.handoff_age_lag_drops = u64::MAX - 1;
        }

        record_buffer_offer(slot, BufferOfferOutcome::Buffered, 1, 1, 0);
        record_buffer_handoff(slot, 8, 7, 1, 1, 0);
        record_buffer_handoff(slot, 0, 2, 1, 1, 0);
        record_buffer_handoff(slot, 0, u64::MAX, 1, 1, 0);

        let summary = probe.buffer_summary().unwrap();
        assert_eq!(summary.adapter_calls, u64::MAX);
        assert_eq!(summary.buffered, u64::MAX);
        assert_eq!(summary.handoff_age_lag_drops, u64::MAX);
        assert!(summary.overflowed);
        assert!(summary.diagnostic_error);
        assert!(probe.summary().unwrap().diagnostic_error);
    }
}
