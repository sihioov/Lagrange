use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::owner_beta::simulation::{canonical_json, parse_strict_json};

use super::SimulationProcessConfig;

const HEALTH_SCHEMA: &str = "owner-beta-simulation-health-v1";
const HEALTH_SIZE_LIMIT: usize = 8 * 1024;
const MILLIS_PER_SECOND: u64 = 1_000;

const ROOT_KEYS: [&str; 10] = [
    "schema",
    "config_sha256",
    "receipt_sha256",
    "generation",
    "phase",
    "started_boot_ms",
    "observed_boot_ms",
    "progress_boot_ms",
    "sequence",
    "failure",
];
const GENERATION_KEYS: [&str; 3] = ["pid", "start_time_ticks", "boot_id"];

/// Identifies one process incarnation by boot, PID, and kernel start ticks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SimulationProcessGeneration {
    pid: u32,
    start_time_ticks: u64,
    boot_id: Uuid,
}

impl fmt::Debug for SimulationProcessGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationProcessGeneration")
    }
}

impl SimulationProcessGeneration {
    pub fn new(
        pid: u32,
        start_time_ticks: u64,
        boot_id: Uuid,
    ) -> Result<Self, SimulationHealthError> {
        if pid == 0 || pid > i32::MAX as u32 || start_time_ticks == 0 || boot_id.is_nil() {
            return Err(SimulationHealthError::Invalid);
        }

        Ok(Self {
            pid,
            start_time_ticks,
            boot_id,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn start_time_ticks(&self) -> u64 {
        self.start_time_ticks
    }

    pub fn boot_id(&self) -> Uuid {
        self.boot_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationHealthPhase {
    Starting,
    Ready,
    Busy,
    Draining,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationHealthFailure {
    CleanupUnconfirmed,
    CommitUnresolved,
    RuntimeFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationHealthEvent {
    StartupVerified,
    ClaimVerified,
    LeaseVerified,
    CycleVerified,
    ShutdownRequested,
    Fatal(SimulationHealthFailure),
    Sample,
}

/// Value-free failures at the checked health protocol boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimulationHealthError {
    #[error("simulation health record exceeds the size limit")]
    SizeLimitExceeded,
    #[error("simulation health record is invalid")]
    Invalid,
    #[error("simulation health binding does not match")]
    BindingMismatch,
    #[error("simulation process generation does not match")]
    GenerationMismatch,
    #[error("simulation health clock is invalid")]
    ClockInvalid,
    #[error("simulation health transition is invalid")]
    InvalidTransition,
    #[error("simulation health sequence is exhausted")]
    SequenceExhausted,
    #[error("simulation process is not ready")]
    NotReady,
    #[error("simulation health is stale")]
    Stale,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SimulationHealthRecord {
    generation: SimulationProcessGeneration,
    config_sha256: String,
    receipt_sha256: String,
    health_max_age_ms: u64,
    phase: SimulationHealthPhase,
    started_boot_ms: u64,
    observed_boot_ms: u64,
    progress_boot_ms: Option<u64>,
    sequence: u64,
    failure: Option<SimulationHealthFailure>,
}

impl fmt::Debug for SimulationHealthRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationHealthRecord")
    }
}

impl SimulationHealthRecord {
    pub fn from_json(
        bytes: &[u8],
        config: &SimulationProcessConfig,
        expected_generation: &SimulationProcessGeneration,
    ) -> Result<Self, SimulationHealthError> {
        if bytes.len() > HEALTH_SIZE_LIMIT {
            return Err(SimulationHealthError::SizeLimitExceeded);
        }

        let value = parse_strict_json(bytes).map_err(|_| SimulationHealthError::Invalid)?;
        let root = value.as_object().ok_or(SimulationHealthError::Invalid)?;
        if !has_exact_keys(root, &ROOT_KEYS) {
            return Err(SimulationHealthError::Invalid);
        }
        let generation_object = root
            .get("generation")
            .and_then(Value::as_object)
            .ok_or(SimulationHealthError::Invalid)?;
        if !has_exact_keys(generation_object, &GENERATION_KEYS) {
            return Err(SimulationHealthError::Invalid);
        }

        let wire = serde_json::from_value::<HealthRecordWire>(value)
            .map_err(|_| SimulationHealthError::Invalid)?;
        if wire.schema != HEALTH_SCHEMA
            || !is_sha256(&wire.config_sha256)
            || !is_sha256(&wire.receipt_sha256)
        {
            return Err(SimulationHealthError::Invalid);
        }
        if wire.config_sha256 != config.config_sha256()
            || wire.receipt_sha256 != config.receipt_sha256()
        {
            return Err(SimulationHealthError::BindingMismatch);
        }

        let boot_id = Uuid::parse_str(&wire.generation.boot_id)
            .map_err(|_| SimulationHealthError::Invalid)?;
        if boot_id.is_nil() || boot_id.to_string() != wire.generation.boot_id {
            return Err(SimulationHealthError::Invalid);
        }
        let generation = SimulationProcessGeneration::new(
            wire.generation.pid,
            wire.generation.start_time_ticks,
            boot_id,
        )?;
        if generation != *expected_generation {
            return Err(SimulationHealthError::GenerationMismatch);
        }

        let phase = parse_phase(&wire.phase).ok_or(SimulationHealthError::Invalid)?;
        let failure = match (phase, wire.failure.as_deref()) {
            (SimulationHealthPhase::Failed, Some(value)) => {
                Some(parse_failure(value).ok_or(SimulationHealthError::Invalid)?)
            }
            (SimulationHealthPhase::Failed, None) => return Err(SimulationHealthError::Invalid),
            (_, None) => None,
            (_, Some(_)) => return Err(SimulationHealthError::Invalid),
        };

        let record = Self {
            generation,
            config_sha256: wire.config_sha256,
            receipt_sha256: wire.receipt_sha256,
            health_max_age_ms: health_max_age_ms(config)?,
            phase,
            started_boot_ms: wire.started_boot_ms,
            observed_boot_ms: wire.observed_boot_ms,
            progress_boot_ms: wire.progress_boot_ms,
            sequence: wire.sequence,
            failure,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, SimulationHealthError> {
        let wire = HealthRecordWireRef {
            schema: HEALTH_SCHEMA,
            config_sha256: &self.config_sha256,
            receipt_sha256: &self.receipt_sha256,
            generation: GenerationWireRef {
                pid: self.generation.pid,
                start_time_ticks: self.generation.start_time_ticks,
                boot_id: self.generation.boot_id.to_string(),
            },
            phase: phase_name(self.phase),
            started_boot_ms: self.started_boot_ms,
            observed_boot_ms: self.observed_boot_ms,
            progress_boot_ms: self.progress_boot_ms,
            sequence: self.sequence,
            failure: self.failure.map(failure_name),
        };
        let bytes = canonical_json(&wire).map_err(|_| SimulationHealthError::Invalid)?;
        if bytes.len() > HEALTH_SIZE_LIMIT {
            return Err(SimulationHealthError::SizeLimitExceeded);
        }
        Ok(bytes)
    }

    pub fn check_ready_at(&self, now_boot_ms: u64) -> Result<(), SimulationHealthError> {
        if now_boot_ms < self.observed_boot_ms {
            return Err(SimulationHealthError::ClockInvalid);
        }
        if !matches!(
            self.phase,
            SimulationHealthPhase::Ready | SimulationHealthPhase::Busy
        ) {
            return Err(SimulationHealthError::NotReady);
        }
        let progress_boot_ms = self
            .progress_boot_ms
            .ok_or(SimulationHealthError::NotReady)?;
        let observed_age = now_boot_ms
            .checked_sub(self.observed_boot_ms)
            .ok_or(SimulationHealthError::ClockInvalid)?;
        let progress_age = now_boot_ms
            .checked_sub(progress_boot_ms)
            .ok_or(SimulationHealthError::ClockInvalid)?;
        if observed_age > self.health_max_age_ms || progress_age > self.health_max_age_ms {
            return Err(SimulationHealthError::Stale);
        }
        Ok(())
    }

    pub fn phase(&self) -> SimulationHealthPhase {
        self.phase
    }

    pub fn failure(&self) -> Option<SimulationHealthFailure> {
        self.failure
    }

    pub fn generation(&self) -> &SimulationProcessGeneration {
        &self.generation
    }

    pub fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub fn receipt_sha256(&self) -> &str {
        &self.receipt_sha256
    }

    pub fn started_boot_ms(&self) -> u64 {
        self.started_boot_ms
    }

    pub fn observed_boot_ms(&self) -> u64 {
        self.observed_boot_ms
    }

    pub fn progress_boot_ms(&self) -> Option<u64> {
        self.progress_boot_ms
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    fn validate(&self) -> Result<(), SimulationHealthError> {
        if self.observed_boot_ms < self.started_boot_ms
            || self.progress_boot_ms.is_some_and(|progress| {
                progress < self.started_boot_ms || progress > self.observed_boot_ms
            })
        {
            return Err(SimulationHealthError::Invalid);
        }

        match self.phase {
            SimulationHealthPhase::Starting if self.progress_boot_ms.is_some() => {
                return Err(SimulationHealthError::Invalid);
            }
            SimulationHealthPhase::Ready | SimulationHealthPhase::Busy
                if self.progress_boot_ms.is_none() =>
            {
                return Err(SimulationHealthError::Invalid);
            }
            _ => {}
        }
        if self.phase != SimulationHealthPhase::Starting && self.sequence == 0 {
            return Err(SimulationHealthError::Invalid);
        }
        if (self.phase == SimulationHealthPhase::Failed) != self.failure.is_some() {
            return Err(SimulationHealthError::Invalid);
        }
        Ok(())
    }
}

pub struct SimulationHealthTracker {
    record: SimulationHealthRecord,
}

impl fmt::Debug for SimulationHealthTracker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationHealthTracker")
    }
}

impl SimulationHealthTracker {
    pub fn new(
        config: &SimulationProcessConfig,
        generation: SimulationProcessGeneration,
        started_boot_ms: u64,
    ) -> Self {
        Self {
            record: SimulationHealthRecord {
                generation,
                config_sha256: config.config_sha256().to_owned(),
                receipt_sha256: config.receipt_sha256().to_owned(),
                health_max_age_ms: health_max_age_ms(config)
                    .expect("checked process config age fits boot-relative milliseconds"),
                phase: SimulationHealthPhase::Starting,
                started_boot_ms,
                observed_boot_ms: started_boot_ms,
                progress_boot_ms: None,
                sequence: 0,
                failure: None,
            },
        }
    }

    pub fn apply(
        &mut self,
        event: SimulationHealthEvent,
        now_boot_ms: u64,
    ) -> Result<(), SimulationHealthError> {
        if now_boot_ms < self.record.observed_boot_ms {
            return Err(SimulationHealthError::ClockInvalid);
        }
        let next_sequence = checked_next_sequence(self.record.sequence)?;
        let next = next_state(&self.record, event, now_boot_ms)?;

        self.record.phase = next.phase;
        self.record.progress_boot_ms = next.progress_boot_ms;
        self.record.failure = next.failure;
        self.record.observed_boot_ms = now_boot_ms;
        self.record.sequence = next_sequence;
        Ok(())
    }

    pub fn record(&self) -> SimulationHealthRecord {
        self.record.clone()
    }
}

struct NextState {
    phase: SimulationHealthPhase,
    progress_boot_ms: Option<u64>,
    failure: Option<SimulationHealthFailure>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthRecordWire {
    schema: String,
    config_sha256: String,
    receipt_sha256: String,
    generation: GenerationWire,
    phase: String,
    started_boot_ms: u64,
    observed_boot_ms: u64,
    progress_boot_ms: Option<u64>,
    sequence: u64,
    failure: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationWire {
    pid: u32,
    start_time_ticks: u64,
    boot_id: String,
}

#[derive(Serialize)]
struct HealthRecordWireRef<'a> {
    schema: &'static str,
    config_sha256: &'a str,
    receipt_sha256: &'a str,
    generation: GenerationWireRef,
    phase: &'static str,
    started_boot_ms: u64,
    observed_boot_ms: u64,
    progress_boot_ms: Option<u64>,
    sequence: u64,
    failure: Option<&'static str>,
}

#[derive(Serialize)]
struct GenerationWireRef {
    pid: u32,
    start_time_ticks: u64,
    boot_id: String,
}

fn next_state(
    record: &SimulationHealthRecord,
    event: SimulationHealthEvent,
    now_boot_ms: u64,
) -> Result<NextState, SimulationHealthError> {
    let unchanged = || NextState {
        phase: record.phase,
        progress_boot_ms: record.progress_boot_ms,
        failure: record.failure,
    };
    let progress = || Some(now_boot_ms);

    let next = match (record.phase, event) {
        (SimulationHealthPhase::Starting, SimulationHealthEvent::Sample)
        | (SimulationHealthPhase::Ready, SimulationHealthEvent::Sample)
        | (SimulationHealthPhase::Busy, SimulationHealthEvent::Sample)
        | (SimulationHealthPhase::Draining, SimulationHealthEvent::Sample)
        | (SimulationHealthPhase::Failed, SimulationHealthEvent::Sample)
        | (SimulationHealthPhase::Draining, SimulationHealthEvent::ShutdownRequested) => {
            unchanged()
        }
        (SimulationHealthPhase::Starting, SimulationHealthEvent::StartupVerified) => NextState {
            phase: SimulationHealthPhase::Ready,
            progress_boot_ms: progress(),
            failure: None,
        },
        (SimulationHealthPhase::Ready, SimulationHealthEvent::ClaimVerified) => NextState {
            phase: SimulationHealthPhase::Busy,
            progress_boot_ms: progress(),
            failure: None,
        },
        (SimulationHealthPhase::Ready, SimulationHealthEvent::CycleVerified)
        | (SimulationHealthPhase::Busy, SimulationHealthEvent::LeaseVerified) => NextState {
            phase: record.phase,
            progress_boot_ms: progress(),
            failure: None,
        },
        (SimulationHealthPhase::Busy, SimulationHealthEvent::CycleVerified) => NextState {
            phase: SimulationHealthPhase::Ready,
            progress_boot_ms: progress(),
            failure: None,
        },
        (
            SimulationHealthPhase::Starting
            | SimulationHealthPhase::Ready
            | SimulationHealthPhase::Busy,
            SimulationHealthEvent::ShutdownRequested,
        ) => NextState {
            phase: SimulationHealthPhase::Draining,
            progress_boot_ms: record.progress_boot_ms,
            failure: None,
        },
        (
            SimulationHealthPhase::Starting
            | SimulationHealthPhase::Ready
            | SimulationHealthPhase::Busy
            | SimulationHealthPhase::Draining,
            SimulationHealthEvent::Fatal(reason),
        ) => NextState {
            phase: SimulationHealthPhase::Failed,
            progress_boot_ms: record.progress_boot_ms,
            failure: Some(reason),
        },
        _ => return Err(SimulationHealthError::InvalidTransition),
    };
    Ok(next)
}

fn checked_next_sequence(sequence: u64) -> Result<u64, SimulationHealthError> {
    sequence
        .checked_add(1)
        .ok_or(SimulationHealthError::SequenceExhausted)
}

fn health_max_age_ms(config: &SimulationProcessConfig) -> Result<u64, SimulationHealthError> {
    config
        .limits()
        .health_max_age_seconds()
        .checked_mul(MILLIS_PER_SECOND)
        .ok_or(SimulationHealthError::Invalid)
}

fn has_exact_keys(object: &Map<String, Value>, required: &[&str]) -> bool {
    object.len() == required.len() && required.iter().all(|key| object.contains_key(*key))
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn parse_phase(value: &str) -> Option<SimulationHealthPhase> {
    match value {
        "starting" => Some(SimulationHealthPhase::Starting),
        "ready" => Some(SimulationHealthPhase::Ready),
        "busy" => Some(SimulationHealthPhase::Busy),
        "draining" => Some(SimulationHealthPhase::Draining),
        "failed" => Some(SimulationHealthPhase::Failed),
        _ => None,
    }
}

fn phase_name(value: SimulationHealthPhase) -> &'static str {
    match value {
        SimulationHealthPhase::Starting => "starting",
        SimulationHealthPhase::Ready => "ready",
        SimulationHealthPhase::Busy => "busy",
        SimulationHealthPhase::Draining => "draining",
        SimulationHealthPhase::Failed => "failed",
    }
}

fn parse_failure(value: &str) -> Option<SimulationHealthFailure> {
    match value {
        "cleanup-unconfirmed" => Some(SimulationHealthFailure::CleanupUnconfirmed),
        "commit-unresolved" => Some(SimulationHealthFailure::CommitUnresolved),
        "runtime-failure" => Some(SimulationHealthFailure::RuntimeFailure),
        _ => None,
    }
}

fn failure_name(value: SimulationHealthFailure) -> &'static str {
    match value {
        SimulationHealthFailure::CleanupUnconfirmed => "cleanup-unconfirmed",
        SimulationHealthFailure::CommitUnresolved => "commit-unresolved",
        SimulationHealthFailure::RuntimeFailure => "runtime-failure",
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        SimulationHealthError, SimulationHealthEvent, SimulationHealthPhase,
        SimulationHealthRecord, SimulationHealthTracker, SimulationProcessGeneration,
    };

    fn ready_tracker(sequence: u64) -> SimulationHealthTracker {
        let generation = SimulationProcessGeneration::new(
            4242,
            987_654,
            Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap(),
        )
        .unwrap();
        let record = SimulationHealthRecord {
            generation,
            config_sha256: format!("sha256:{}", "1".repeat(64)),
            receipt_sha256: format!("sha256:{}", "2".repeat(64)),
            health_max_age_ms: 234_000,
            phase: SimulationHealthPhase::Ready,
            started_boot_ms: 100,
            observed_boot_ms: 200,
            progress_boot_ms: Some(150),
            sequence,
            failure: None,
        };
        record.validate().unwrap();
        SimulationHealthTracker { record }
    }

    #[test]
    fn maximum_sequence_rejects_valid_transition_without_mutation() {
        let mut tracker = ready_tracker(u64::MAX);
        let before = tracker.record();
        let before_json = before.to_json().unwrap();

        assert_eq!(
            tracker.apply(SimulationHealthEvent::CycleVerified, 201),
            Err(SimulationHealthError::SequenceExhausted)
        );
        let after = tracker.record();
        assert_eq!(after, before);
        assert_eq!(after.to_json().unwrap(), before_json);
        assert_eq!(after.phase(), SimulationHealthPhase::Ready);
        assert_eq!(after.observed_boot_ms(), 200);
        assert_eq!(after.progress_boot_ms(), Some(150));
        assert_eq!(after.failure(), None);
        assert_eq!(after.sequence(), u64::MAX);
    }

    #[test]
    fn maximum_minus_one_can_advance_once_to_maximum() {
        let mut tracker = ready_tracker(u64::MAX - 1);
        tracker
            .apply(SimulationHealthEvent::CycleVerified, 201)
            .unwrap();
        let after = tracker.record();
        assert_eq!(after.phase(), SimulationHealthPhase::Ready);
        assert_eq!(after.observed_boot_ms(), 201);
        assert_eq!(after.progress_boot_ms(), Some(201));
        assert_eq!(after.failure(), None);
        assert_eq!(after.sequence(), u64::MAX);
    }
}
