// The fake backend exposes trace handles and literal byte fixtures for inspection.
#![allow(
    dead_code,
    clippy::new_ret_no_self,
    clippy::byte_char_slices,
    clippy::collapsible_if
)]

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use uuid::Uuid;

use super::{
    ClockReading, ObservationBackend, SimulationProcessObservation,
    SimulationProcessObservationError as Error, capture_current_with_backend_factory,
    capture_with_backend_factory, clock_to_boot_ms, parse_boot_id, parse_stat,
};

const PID: u32 = 4_242;
const TICKS: u64 = 73_101;
const BOOT_ID: &str = "00112233-4455-6677-8899-aabbccddeeff";

type TraceRef = Rc<RefCell<FakeTrace>>;

#[derive(Default)]
struct FakeTrace {
    operations: Vec<&'static str>,
}

#[derive(Clone)]
struct Trace(TraceRef);

impl Trace {
    fn event(&self, operation: &'static str) {
        self.0.borrow_mut().operations.push(operation);
    }
}

struct FakeBackend {
    trace: Trace,
    boots: VecDeque<Result<Vec<u8>, Error>>,
    stats: VecDeque<Result<Vec<u8>, Error>>,
    clocks: VecDeque<Result<ClockReading, Error>>,
    polls: VecDeque<Result<(), Error>>,
    chains: VecDeque<Result<(), Error>>,
}

impl FakeBackend {
    fn new(inputs: FakeInputs) -> (Box<dyn ObservationBackend>, TraceRef) {
        let trace = Trace(Rc::new(RefCell::new(FakeTrace::default())));

        let backend = Self {
            trace: trace.clone(),
            boots: inputs.boots.into(),
            stats: inputs.stats.into(),
            clocks: inputs.clocks.into(),
            polls: inputs.polls.into(),
            chains: inputs.chains.into(),
        };
        (Box::new(backend), trace.0)
    }

    fn read_leaf(&mut self, operation: &'static str) -> Result<Vec<u8>, Error> {
        self.trace.event(operation);
        if operation == "boot-read" {
            self.boots.pop_front().unwrap_or(Err(Error::Unavailable))
        } else {
            self.stats.pop_front().unwrap_or(Err(Error::Unavailable))
        }
    }
}

impl ObservationBackend for FakeBackend {
    fn validate_chain(&mut self) -> Result<(), Error> {
        self.trace.event("chain");
        self.chains.pop_front().unwrap_or(Ok(()))
    }

    fn poll_pidfd(&mut self) -> Result<(), Error> {
        self.trace.event("poll");
        self.polls.pop_front().unwrap_or(Err(Error::Unavailable))
    }

    fn read_boot_id(&mut self) -> Result<Vec<u8>, Error> {
        self.read_leaf("boot-read")
    }

    fn read_stat(&mut self) -> Result<Vec<u8>, Error> {
        self.read_leaf("stat-read")
    }

    fn boottime(&mut self) -> Result<ClockReading, Error> {
        self.trace.event("clock");
        self.clocks.pop_front().unwrap_or(Err(Error::Unavailable))
    }
}

struct FakeInputs {
    boots: Vec<Result<Vec<u8>, Error>>,
    stats: Vec<Result<Vec<u8>, Error>>,
    clocks: Vec<Result<ClockReading, Error>>,
    polls: Vec<Result<(), Error>>,
    chains: Vec<Result<(), Error>>,
}

fn successful_inputs(
    pid: u32,
    ticks: u64,
    boot_ids: [&str; 4],
    states: [u8; 4],
    clocks: [(i128, i128); 4],
) -> FakeInputs {
    FakeInputs {
        boots: boot_ids
            .into_iter()
            .map(|boot_id| Ok(boot_bytes(boot_id)))
            .collect(),
        stats: states
            .into_iter()
            .map(|state| Ok(stat_bytes(pid, b"worker", &[state], &ticks.to_string())))
            .collect(),
        clocks: clocks
            .into_iter()
            .map(|(seconds, nanoseconds)| {
                Ok(ClockReading {
                    seconds,
                    nanoseconds,
                })
            })
            .collect(),
        polls: vec![Ok(()); 4],
        chains: vec![Ok(()); 4],
    }
}

fn boot_bytes(boot_id: &str) -> Vec<u8> {
    let mut bytes = boot_id.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes
}

fn stat_bytes(pid: u32, comm: &[u8], state: &[u8], start_ticks: &str) -> Vec<u8> {
    let mut bytes = pid.to_string().into_bytes();
    bytes.extend_from_slice(b" (");
    bytes.extend_from_slice(comm);
    bytes.extend_from_slice(b") ");
    bytes.extend_from_slice(state);
    for field in 1..19 {
        bytes.push(b' ');
        if field == 1 {
            bytes.extend_from_slice(b"-2");
        } else {
            bytes.push(b'0');
        }
    }
    bytes.push(b' ');
    bytes.extend_from_slice(start_ticks.as_bytes());
    bytes.push(b'\n');
    bytes
}

fn clock(seconds: i128, nanoseconds: i128) -> (i128, i128) {
    (seconds, nanoseconds)
}

fn uuid(boot_id: &str) -> Uuid {
    Uuid::parse_str(boot_id).expect("fixed test UUID is valid")
}

fn capture_with_inputs(
    pid: u32,
    inputs: FakeInputs,
) -> Result<(SimulationProcessObservation, TraceRef), Error> {
    let (backend, trace) = FakeBackend::new(inputs);
    let observation = capture_with_backend_factory(pid, move |_| Ok(backend))?;
    Ok((observation, trace))
}

fn successful_capture_inputs(pid: u32, ticks: u64) -> FakeInputs {
    successful_inputs(
        pid,
        ticks,
        [BOOT_ID; 4],
        [b'S', b'R', b'D', b'I'],
        [
            clock(10, 100_000_000),
            clock(10, 200_000_000),
            clock(10, 300_000_000),
            clock(11, 100_000_000),
        ],
    )
}

fn flow_operations(trace: &TraceRef) -> Vec<&'static str> {
    trace
        .borrow()
        .operations
        .iter()
        .copied()
        .filter(|operation| {
            matches!(
                *operation,
                "chain" | "poll" | "boot-read" | "stat-read" | "clock"
            )
        })
        .collect()
}

fn assert_invalid_stat(bytes: &[u8]) {
    assert_eq!(
        parse_stat(PID, uuid(BOOT_ID), bytes),
        Err(Error::InvalidProcess)
    );
}

#[test]
fn stat_parser_uses_field_22_and_accepts_opaque_comm_bytes() {
    let comm = b"space ) nested ( close )\n non-utf8-\xff";
    let bytes = stat_bytes(PID, comm, b"R", &TICKS.to_string());
    let generation = parse_stat(PID, uuid(BOOT_ID), &bytes).expect("valid stat baseline");

    assert_eq!(generation.pid(), PID);
    assert_eq!(generation.start_time_ticks(), TICKS);
    assert_eq!(generation.boot_id(), uuid(BOOT_ID));
}

#[test]
fn stat_parser_rejects_wrong_pid_nul_short_state_and_unknown_states() {
    assert_invalid_stat(&stat_bytes(PID + 1, b"worker", b"R", &TICKS.to_string()));
    let mut wrong_prefix = stat_bytes(PID, b"worker", b"R", &TICKS.to_string());
    wrong_prefix[0] = b'0';
    assert_invalid_stat(&wrong_prefix);
    assert_invalid_stat(&stat_bytes(
        PID,
        b"worker\0hidden",
        b"R",
        &TICKS.to_string(),
    ));

    let mut short = PID.to_string().into_bytes();
    short.extend_from_slice(b" (worker) R 1 2 3\n");
    assert_invalid_stat(&short);

    for state in [b"RS".as_slice(), b"?", b"Z", b"X", b"x", b"\xc3\xa9"] {
        assert_invalid_stat(&stat_bytes(PID, b"worker", state, &TICKS.to_string()));
    }
}

#[test]
fn stat_parser_rejects_noncanonical_or_overflowed_start_ticks() {
    for ticks in ["0", "00", "-1", "+1", "18446744073709551616", "١"] {
        assert_invalid_stat(&stat_bytes(PID, b"worker", b"S", ticks));
    }
}

#[test]
fn stat_parser_validates_optional_suffix_fields_without_parsing_the_identity_ticks() {
    let mut extra_numeric = stat_bytes(PID, b"worker", b"S", &TICKS.to_string());
    extra_numeric.extend_from_slice(b" 8 -9\n");
    let generation = parse_stat(PID, uuid(BOOT_ID), &extra_numeric).expect("signed numeric suffix");
    assert_eq!(generation.start_time_ticks(), TICKS);

    extra_numeric.extend_from_slice(b" +10");
    assert_invalid_stat(&extra_numeric);
}

#[test]
fn stat_parser_enforces_the_exact_read_cap() {
    let mut exact = stat_bytes(PID, b"worker", b"S", &TICKS.to_string());
    exact.resize(4096, b' ');
    assert_eq!(exact.len(), 4096);
    assert!(parse_stat(PID, uuid(BOOT_ID), &exact).is_ok());

    exact.push(b' ');
    assert_eq!(exact.len(), 4097);
    assert_eq!(
        parse_stat(PID, uuid(BOOT_ID), &exact),
        Err(Error::SizeLimit)
    );
}

#[test]
fn boot_parser_requires_one_canonical_non_nil_uuid_line() {
    assert_eq!(
        parse_boot_id(&boot_bytes(BOOT_ID)).expect("canonical UUID line"),
        uuid(BOOT_ID)
    );

    let mut missing_lf = boot_bytes(BOOT_ID);
    missing_lf.pop();
    assert_eq!(parse_boot_id(&missing_lf), Err(Error::InvalidProcess));

    let mut extra = boot_bytes(BOOT_ID);
    extra.push(b'\n');
    assert_eq!(parse_boot_id(&extra), Err(Error::SizeLimit));

    let mut crlf = BOOT_ID.as_bytes().to_vec();
    crlf.extend_from_slice(b"\r\n");
    assert_eq!(parse_boot_id(&crlf), Err(Error::SizeLimit));

    let mut uppercase = boot_bytes(BOOT_ID);
    for byte in &mut uppercase[..36] {
        *byte = byte.to_ascii_uppercase();
    }
    assert_eq!(parse_boot_id(&uppercase), Err(Error::InvalidProcess));

    assert_eq!(
        parse_boot_id(&boot_bytes("00000000-0000-0000-0000-000000000000")),
        Err(Error::InvalidProcess)
    );

    let mut invalid_utf8 = boot_bytes(BOOT_ID);
    invalid_utf8[0] = 0xff;
    assert_eq!(parse_boot_id(&invalid_utf8), Err(Error::InvalidProcess));

    let mut bad_hyphen = boot_bytes(BOOT_ID);
    bad_hyphen[8] = b' ';
    assert_eq!(parse_boot_id(&bad_hyphen), Err(Error::InvalidProcess));
}

#[test]
fn clock_conversion_checks_signs_ranges_and_arithmetic_overflow() {
    assert_eq!(
        clock_to_boot_ms(ClockReading {
            seconds: 0,
            nanoseconds: 0
        }),
        Ok(0)
    );
    assert_eq!(
        clock_to_boot_ms(ClockReading {
            seconds: 1,
            nanoseconds: 999_999_999,
        }),
        Ok(1_999)
    );

    for invalid in [
        ClockReading {
            seconds: -1,
            nanoseconds: 0,
        },
        ClockReading {
            seconds: 0,
            nanoseconds: -1,
        },
        ClockReading {
            seconds: 0,
            nanoseconds: 1_000_000_000,
        },
        ClockReading {
            seconds: i128::from(u64::MAX) + 1,
            nanoseconds: 0,
        },
        ClockReading {
            seconds: i128::from(u64::MAX / 1_000),
            nanoseconds: 999_999_999,
        },
    ] {
        assert_eq!(clock_to_boot_ms(invalid), Err(Error::ClockInvalid));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn capture_and_revalidate_share_ordered_flow_and_retain_one_generation() {
    let (mut observation, trace) =
        capture_with_inputs(PID, successful_capture_inputs(PID, TICKS)).expect("capture baseline");
    assert_eq!(observation.generation().pid(), PID);
    assert_eq!(observation.generation().start_time_ticks(), TICKS);
    assert_eq!(observation.observed_boot_ms(), 10_200);
    assert_eq!(
        flow_operations(&trace),
        [
            "chain",
            "poll",
            "boot-read",
            "stat-read",
            "clock",
            "stat-read",
            "boot-read",
            "chain",
            "clock",
            "poll",
        ]
    );

    assert_eq!(observation.revalidate(), Ok(11_100));
    assert_eq!(observation.observed_boot_ms(), 11_100);
    assert_eq!(observation.generation().start_time_ticks(), TICKS);
    assert_eq!(
        flow_operations(&trace),
        [
            "chain",
            "poll",
            "boot-read",
            "stat-read",
            "clock",
            "stat-read",
            "boot-read",
            "chain",
            "clock",
            "poll",
            "chain",
            "poll",
            "boot-read",
            "stat-read",
            "clock",
            "stat-read",
            "boot-read",
            "chain",
            "clock",
            "poll",
        ]
    );

    drop(observation);
}

#[test]
fn current_pid_helper_uses_the_supplied_current_id_without_assuming_pid_one() {
    let requested_pid = 42_424;
    let inputs = successful_inputs(
        requested_pid,
        TICKS,
        [BOOT_ID; 4],
        [b'S', b'R', b'R', b'S'],
        [clock(1, 0), clock(1, 1), clock(1, 2), clock(2, 0)],
    );
    let (backend, _) = FakeBackend::new(inputs);
    let observation = capture_current_with_backend_factory(
        || requested_pid,
        move |pid| {
            assert_eq!(pid, requested_pid);
            Ok(backend)
        },
    )
    .expect("current PID selected and observed");

    assert_eq!(observation.generation().pid(), requested_pid);
}

#[test]
fn capture_rejects_boot_or_start_tick_changes_between_bounded_reads() {
    let changed_boot = successful_inputs(
        PID,
        TICKS,
        [
            BOOT_ID,
            "11112233-4455-6677-8899-aabbccddeeff",
            BOOT_ID,
            BOOT_ID,
        ],
        [b'S', b'R', b'S', b'R'],
        [clock(1, 0), clock(1, 1), clock(2, 0), clock(2, 1)],
    );
    let (mut boot_backend, _) = FakeBackend::new(changed_boot);
    let boot_result = super::observe_flow(boot_backend.as_mut(), PID, None, None);
    assert_eq!(boot_result, Err(Error::Changed));
    drop(boot_backend);

    let mut changed_ticks = successful_inputs(
        PID,
        TICKS,
        [BOOT_ID; 4],
        [b'S', b'R', b'S', b'R'],
        [clock(1, 0), clock(1, 1), clock(2, 0), clock(2, 1)],
    );
    changed_ticks.stats[1] = Ok(stat_bytes(PID, b"worker", b"R", "73102"));
    let (mut stat_backend, _) = FakeBackend::new(changed_ticks);
    let stat_result = super::observe_flow(stat_backend.as_mut(), PID, None, None);
    assert_eq!(stat_result, Err(Error::Changed));
    drop(stat_backend);
}

#[cfg(target_os = "linux")]
#[test]
fn failed_revalidation_preserves_snapshot_then_invalidates_without_more_io() {
    let mut inputs = successful_capture_inputs(PID, TICKS);
    inputs.boots[2] = Ok(boot_bytes(BOOT_ID));
    inputs.boots[3] = Ok(boot_bytes(BOOT_ID));
    inputs.stats[2] = Ok(stat_bytes(PID, b"worker", b"S", &TICKS.to_string()));
    inputs.stats[3] = Ok(stat_bytes(PID, b"worker", b"R", "73102"));
    inputs.polls[2] = Ok(());
    inputs.polls[3] = Ok(());
    let (mut observation, trace) = capture_with_inputs(PID, inputs).expect("capture baseline");
    let old_generation = observation.generation();
    let old_timestamp = observation.observed_boot_ms();

    assert_eq!(observation.revalidate(), Err(Error::Changed));
    assert_eq!(observation.generation(), old_generation);
    assert_eq!(observation.observed_boot_ms(), old_timestamp);
    let operations_after_failure = trace.borrow().operations.len();

    assert_eq!(observation.revalidate(), Err(Error::Invalidated));
    assert_eq!(trace.borrow().operations.len(), operations_after_failure);
    drop(observation);
}

#[cfg(target_os = "linux")]
#[test]
fn failed_revalidation_on_boot_change_preserves_snapshot_and_poison_state() {
    let mut inputs = successful_capture_inputs(PID, TICKS);
    inputs.boots[2] = Ok(boot_bytes(BOOT_ID));
    inputs.boots[3] = Ok(boot_bytes("11112233-4455-6677-8899-aabbccddeeff"));
    inputs.stats[2] = Ok(stat_bytes(PID, b"worker", b"S", &TICKS.to_string()));
    inputs.stats[3] = Ok(stat_bytes(PID, b"worker", b"R", &TICKS.to_string()));
    let (mut observation, trace) = capture_with_inputs(PID, inputs).expect("capture baseline");
    let generation = observation.generation();
    let timestamp = observation.observed_boot_ms();

    assert_eq!(observation.revalidate(), Err(Error::Changed));
    assert_eq!(observation.generation(), generation);
    assert_eq!(observation.observed_boot_ms(), timestamp);
    let operations_after_failure = trace.borrow().operations.len();
    assert_eq!(observation.revalidate(), Err(Error::Invalidated));
    assert_eq!(trace.borrow().operations.len(), operations_after_failure);

    drop(observation);
}

#[cfg(target_os = "linux")]
#[test]
fn revalidation_accepts_equal_time_and_suspend_sized_forward_jumps() {
    for (prior_read, final_time, expected_ms) in [
        (clock(10, 200_000_000), clock(10, 200_000_000), 10_200),
        (clock(10, 300_000_000), clock(86_410, 0), 86_410_000),
    ] {
        let mut inputs = successful_capture_inputs(PID, TICKS);
        inputs.clocks[2] = Ok(ClockReading {
            seconds: prior_read.0,
            nanoseconds: prior_read.1,
        });
        inputs.clocks[3] = Ok(ClockReading {
            seconds: final_time.0,
            nanoseconds: final_time.1,
        });
        let (mut observation, _) = capture_with_inputs(PID, inputs).expect("capture baseline");
        assert_eq!(observation.revalidate(), Ok(expected_ms));
        drop(observation);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn capture_and_revalidation_reject_backward_boot_clock_values() {
    let mut capture_inputs = successful_capture_inputs(PID, TICKS);
    capture_inputs.clocks[1] = Ok(ClockReading {
        seconds: 9,
        nanoseconds: 999_000_000,
    });
    let (backend, _) = FakeBackend::new(capture_inputs);
    let result = capture_with_backend_factory(PID, move |_| Ok(backend));
    assert_eq!(result.err(), Some(Error::ClockInvalid));

    let mut revalidation_inputs = successful_capture_inputs(PID, TICKS);
    revalidation_inputs.clocks[2] = Ok(ClockReading {
        seconds: 10,
        nanoseconds: 200_000_000,
    });
    revalidation_inputs.clocks[3] = Ok(ClockReading {
        seconds: 10,
        nanoseconds: 199_000_000,
    });
    let (mut observation, _) =
        capture_with_inputs(PID, revalidation_inputs).expect("capture baseline");
    assert_eq!(observation.revalidate(), Err(Error::ClockInvalid));
    drop(observation);
}

#[derive(Clone, Copy)]
enum FailureQueue {
    Chain(usize),
    Poll(usize),
    Boot(usize),
    Stat(usize),
    Clock(usize),
}

#[test]
fn capture_failure_boundaries_preserve_classified_flow_errors() {
    let cases = [
        (FailureQueue::Chain(0), Error::UnsafeProcfs),
        (FailureQueue::Poll(0), Error::Changed),
        (FailureQueue::Boot(0), Error::Unavailable),
        (FailureQueue::Stat(0), Error::Unavailable),
        (FailureQueue::Poll(1), Error::Unavailable),
        (FailureQueue::Clock(0), Error::ClockInvalid),
        (FailureQueue::Stat(1), Error::Unavailable),
        (FailureQueue::Boot(1), Error::Unavailable),
        (FailureQueue::Chain(1), Error::Changed),
        (FailureQueue::Clock(1), Error::ClockInvalid),
        (FailureQueue::Poll(1), Error::Changed),
    ];

    for (failure, expected) in cases {
        let mut inputs = successful_capture_inputs(PID, TICKS);
        match failure {
            FailureQueue::Chain(index) => inputs.chains[index] = Err(expected),
            FailureQueue::Poll(index) => inputs.polls[index] = Err(expected),
            FailureQueue::Boot(index) => inputs.boots[index] = Err(expected),
            FailureQueue::Stat(index) => inputs.stats[index] = Err(expected),
            FailureQueue::Clock(index) => inputs.clocks[index] = Err(expected),
        }

        let (backend, _) = FakeBackend::new(inputs);
        let result = capture_with_backend_factory(PID, move |_| Ok(backend));
        assert_eq!(result.err(), Some(expected));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pidfd_poll_failures_at_capture_and_revalidation_boundaries_poison_handles() {
    for (index, expected) in [(0, Error::Changed), (1, Error::Changed)] {
        let mut inputs = successful_capture_inputs(PID, TICKS);
        inputs.polls[index] = Err(expected);
        let (backend, _) = FakeBackend::new(inputs);
        let result = capture_with_backend_factory(PID, move |_| Ok(backend));
        assert_eq!(result.err(), Some(expected));
    }

    for index in [2, 3] {
        let mut inputs = successful_capture_inputs(PID, TICKS);
        inputs.polls[index] = Err(Error::Changed);
        let (mut observation, _) =
            capture_with_inputs(PID, inputs).expect("capture before revalidation boundary");
        assert_eq!(observation.revalidate(), Err(Error::Changed));
        assert_eq!(observation.observed_boot_ms(), 10_200);
        drop(observation);
    }
}

#[test]
fn unsupported_pidfd_factory_fails_without_replacement_fallback() {
    let called = Rc::new(RefCell::new(false));
    let called_by_factory = called.clone();
    let result = capture_with_backend_factory(PID, move |_| {
        *called_by_factory.borrow_mut() = true;
        Err(Error::UnsupportedPlatform)
    });
    assert_eq!(result.err(), Some(Error::UnsupportedPlatform));
    assert!(*called.borrow());

    for pid in [0, i32::MAX as u32 + 1] {
        let invalid_pid_factory_called = Rc::new(RefCell::new(false));
        let factory_marker = invalid_pid_factory_called.clone();
        let invalid = capture_with_backend_factory(pid, move |_| {
            *factory_marker.borrow_mut() = true;
            Err(Error::Unavailable)
        });
        assert_eq!(invalid.err(), Some(Error::InvalidProcess));
        assert!(!*invalid_pid_factory_called.borrow());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_errno_classification_is_fail_closed_and_value_free() {
    use super::linux::{classify_named_error, classify_path_open_error, classify_pidfd_open_error};

    assert_eq!(
        classify_pidfd_open_error(libc::ENOSYS),
        Error::UnsupportedPlatform
    );
    assert_eq!(
        classify_pidfd_open_error(libc::EINVAL),
        Error::UnsupportedPlatform
    );
    assert_eq!(classify_pidfd_open_error(libc::EACCES), Error::Unavailable);
    assert_eq!(classify_pidfd_open_error(libc::EPERM), Error::Unavailable);

    for errno in [libc::ELOOP, libc::ENOTDIR, libc::EISDIR] {
        assert_eq!(classify_path_open_error(errno), Error::UnsafeProcfs);
    }
    assert_eq!(classify_path_open_error(libc::ENOENT), Error::Unavailable);
    assert_eq!(classify_path_open_error(libc::EINTR), Error::Unavailable);

    assert_eq!(classify_named_error(libc::ENOENT), Error::Unavailable);
    assert_eq!(classify_named_error(libc::ENOTDIR), Error::UnsafeProcfs);
    assert_eq!(classify_named_error(libc::EACCES), Error::Unavailable);
}

#[test]
fn diagnostics_expose_only_static_type_and_error_names() {
    let (observation, _) =
        capture_with_inputs(PID, successful_capture_inputs(PID, TICKS)).expect("capture baseline");
    let debug = format!("{observation:?}");
    assert_eq!(debug, "SimulationProcessObservation");
    assert!(!debug.contains(&PID.to_string()));
    assert!(!debug.contains(&TICKS.to_string()));
    assert!(!debug.contains(BOOT_ID));

    let error = Error::Unavailable;
    assert_eq!(format!("{error:?}"), "Unavailable");
    assert_eq!(
        error.to_string(),
        "a process observation operation is unavailable"
    );
    assert!(std::error::Error::source(&error).is_none());
    drop(observation);
}

#[cfg(target_os = "linux")]
mod descriptor_regressions {
    use super::*;
    use std::{collections::HashMap, ffi::CStr};

    use super::super::linux::{FileIdentity, LinuxBackend, LinuxSyscalls, PollOutcome};

    type KernelRef = Rc<RefCell<FakeKernel>>;

    const BOOT_PATH: &str = "/proc/sys/kernel/random/boot_id";
    const STAT_PATH: &str = "/proc/4242/stat";

    #[derive(Clone, Copy)]
    enum ReadAction {
        Limit(usize),
        Eof,
        Error(libc::c_int),
    }

    #[derive(Default)]
    struct FakeKernel {
        events: Vec<String>,
        opened_handles: Vec<(usize, String)>,
        dropped_handles: Vec<(usize, String)>,
        next_handle: usize,
        open_calls: usize,
        open_failure: Option<(usize, libc::c_int)>,
        identities: HashMap<String, FileIdentity>,
        held_overrides: HashMap<String, FileIdentity>,
        named_overrides: HashMap<(String, usize), FileIdentity>,
        named_errors: HashMap<(String, usize), libc::c_int>,
        named_calls: HashMap<String, usize>,
        filesystem_types: HashMap<String, u64>,
        read_files: HashMap<String, Vec<u8>>,
        read_offsets: HashMap<usize, usize>,
        read_chunk_limit: Option<usize>,
        read_actions: VecDeque<ReadAction>,
        poll_results: VecDeque<Result<PollOutcome, libc::c_int>>,
        clocks: VecDeque<Result<ClockReading, libc::c_int>>,
    }

    struct FakeDescriptor {
        id: usize,
        path: String,
        kernel: KernelRef,
    }

    impl Drop for FakeDescriptor {
        fn drop(&mut self) {
            let mut kernel = self.kernel.borrow_mut();
            kernel
                .events
                .push(format!("drop:{}:{}", self.id, self.path));
            kernel.dropped_handles.push((self.id, self.path.clone()));
        }
    }

    struct FakeLinuxSyscalls {
        kernel: KernelRef,
    }

    impl FakeLinuxSyscalls {
        fn new(pid: u32) -> (Self, KernelRef) {
            let mut kernel = FakeKernel::default();
            kernel
                .identities
                .insert("/".to_owned(), identity(1, 1, libc::S_IFDIR));
            kernel
                .identities
                .insert(BOOT_PATH.to_owned(), identity(1, 150, libc::S_IFREG));
            kernel
                .identities
                .insert(format!("/proc/{pid}/stat"), identity(1, 151, libc::S_IFREG));
            kernel
                .read_files
                .insert(BOOT_PATH.to_owned(), boot_bytes(BOOT_ID));
            kernel.read_files.insert(
                format!("/proc/{pid}/stat"),
                stat_bytes(pid, b"worker", b"S", &TICKS.to_string()),
            );
            let kernel = Rc::new(RefCell::new(kernel));
            (
                Self {
                    kernel: kernel.clone(),
                },
                kernel,
            )
        }

        fn record_open(&self, event: String) -> Result<(), libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            let index = kernel.open_calls;
            kernel.open_calls += 1;
            kernel.events.push(event);
            if let Some((failed_index, errno)) = kernel.open_failure {
                if index == failed_index {
                    return Err(errno);
                }
            }
            Ok(())
        }

        fn acquire(&self, path: String, kind: libc::mode_t) -> FakeDescriptor {
            let mut kernel = self.kernel.borrow_mut();
            kernel.next_handle += 1;
            let id = kernel.next_handle;
            let initial = identity(1, id as u64 + 100, kind);
            let descriptor_identity = *kernel.identities.entry(path.clone()).or_insert(initial);
            kernel
                .events
                .push(format!("acquire:{id}:{path}:{}", descriptor_identity.kind));
            kernel.opened_handles.push((id, path.clone()));
            kernel.read_offsets.insert(id, 0);
            drop(kernel);
            FakeDescriptor {
                id,
                path,
                kernel: self.kernel.clone(),
            }
        }

        fn joined_path(parent: &FakeDescriptor, name: &CStr) -> String {
            let name = String::from_utf8_lossy(name.to_bytes()).into_owned();
            if name.starts_with('/') {
                name
            } else if parent.path == "/" {
                format!("/{name}")
            } else {
                format!("{}/{name}", parent.path)
            }
        }
    }

    impl LinuxSyscalls for FakeLinuxSyscalls {
        type Handle = FakeDescriptor;

        fn open_path(
            &mut self,
            path: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let path = String::from_utf8_lossy(path.to_bytes()).into_owned();
            self.record_open(format!("open-path:{path}:{flags}"))?;
            let kind = if flags & libc::O_DIRECTORY != 0 {
                libc::S_IFDIR
            } else {
                libc::S_IFREG
            };
            Ok(self.acquire(path, kind))
        }

        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let path = Self::joined_path(parent, name);
            let name = String::from_utf8_lossy(name.to_bytes()).into_owned();
            self.record_open(format!("open-at:{}:{name}:{flags}", parent.path))?;
            let kind = if flags & libc::O_DIRECTORY != 0 {
                libc::S_IFDIR
            } else {
                libc::S_IFREG
            };
            Ok(self.acquire(path, kind))
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<FileIdentity, libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            kernel.events.push(format!("fstat:{}", descriptor.path));
            Ok(kernel
                .held_overrides
                .get(&descriptor.path)
                .copied()
                .or_else(|| kernel.identities.get(&descriptor.path).copied())
                .unwrap_or_else(|| identity(1, descriptor.id as u64 + 100, libc::S_IFREG)))
        }

        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<FileIdentity, libc::c_int> {
            let path = Self::joined_path(parent, name);
            let mut kernel = self.kernel.borrow_mut();
            let occurrence = {
                let count = kernel.named_calls.entry(path.clone()).or_insert(0);
                *count += 1;
                *count
            };
            kernel
                .events
                .push(format!("fstat-at:{path}:{flags}:{occurrence}"));
            if let Some(errno) = kernel.named_errors.get(&(path.clone(), occurrence)) {
                return Err(*errno);
            }
            Ok(kernel
                .named_overrides
                .get(&(path.clone(), occurrence))
                .copied()
                .or_else(|| kernel.identities.get(&path).copied())
                .unwrap_or_else(|| identity(1, 0, libc::S_IFREG)))
        }

        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            kernel.events.push(format!("fstatfs:{}", descriptor.path));
            Ok(kernel
                .filesystem_types
                .get(&descriptor.path)
                .copied()
                .unwrap_or(libc::PROC_SUPER_MAGIC as u64))
        }

        fn read(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            let action = kernel.read_actions.pop_front();
            if let Some(ReadAction::Error(errno)) = action {
                kernel.events.push(format!(
                    "read:{}:{}:error:{errno}",
                    descriptor.path,
                    buffer.len()
                ));
                return Err(errno);
            }
            if matches!(action, Some(ReadAction::Eof)) {
                kernel
                    .events
                    .push(format!("read:{}:{}:0", descriptor.path, buffer.len()));
                return Ok(0);
            }

            let data = kernel
                .read_files
                .get(&descriptor.path)
                .cloned()
                .unwrap_or_default();
            let offset = kernel
                .read_offsets
                .get(&descriptor.id)
                .copied()
                .unwrap_or(0);
            let mut capacity = buffer.len();
            if let Some(limit) = kernel.read_chunk_limit {
                capacity = capacity.min(limit);
            }
            if let Some(ReadAction::Limit(limit)) = action {
                capacity = capacity.min(limit);
            }
            let amount = capacity.min(data.len().saturating_sub(offset));
            buffer[..amount].copy_from_slice(&data[offset..offset + amount]);
            kernel.read_offsets.insert(descriptor.id, offset + amount);
            kernel.events.push(format!(
                "read:{}:{}:{amount}",
                descriptor.path,
                buffer.len()
            ));
            Ok(amount)
        }

        fn pidfd_open(
            &mut self,
            pid: u32,
            flags: libc::c_uint,
        ) -> Result<Self::Handle, libc::c_int> {
            self.record_open(format!("pidfd-open:{pid}:{flags}"))?;
            Ok(self.acquire(format!("<pidfd:{pid}>"), libc::S_IFREG))
        }

        fn poll(
            &mut self,
            _descriptor: &Self::Handle,
            events: libc::c_short,
            timeout_ms: libc::c_int,
        ) -> Result<PollOutcome, libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            let result = kernel.poll_results.pop_front().unwrap_or(Ok(PollOutcome {
                ready: 0,
                revents: 0,
            }));
            match result {
                Ok(outcome) => {
                    kernel.events.push(format!(
                        "poll:{events}:{timeout_ms}:{}:{}",
                        outcome.ready, outcome.revents
                    ));
                    Ok(outcome)
                }
                Err(errno) => {
                    kernel
                        .events
                        .push(format!("poll:{events}:{timeout_ms}:error:{errno}"));
                    Err(errno)
                }
            }
        }

        fn boottime(&mut self) -> Result<ClockReading, libc::c_int> {
            let mut kernel = self.kernel.borrow_mut();
            kernel.events.push("clock-boottime".to_owned());
            kernel.clocks.pop_front().unwrap_or(Ok(ClockReading {
                seconds: 10,
                nanoseconds: 200_000_000,
            }))
        }
    }

    fn identity(device: u64, inode: u64, kind: libc::mode_t) -> FileIdentity {
        FileIdentity {
            device,
            inode,
            kind,
        }
    }

    fn open_backend(pid: u32) -> (LinuxBackend<FakeLinuxSyscalls>, KernelRef) {
        let (syscalls, kernel) = FakeLinuxSyscalls::new(pid);
        let backend = LinuxBackend::open_with(syscalls, pid)
            .unwrap_or_else(|_| panic!("synthetic Linux descriptor setup failed"));
        (backend, kernel)
    }

    fn assert_all_handles_released(kernel: &KernelRef) {
        let state = kernel.borrow();
        let mut opened = state
            .opened_handles
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut dropped = state
            .dropped_handles
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        opened.sort_unstable();
        dropped.sort_unstable();
        assert_eq!(opened, dropped);
        assert_eq!(
            opened.len(),
            opened
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }

    fn retained_descriptors(kernel: &KernelRef) -> Vec<(usize, String)> {
        kernel
            .borrow()
            .opened_handles
            .iter()
            .filter(|(_, path)| path.as_str() != BOOT_PATH && path.as_str() != STAT_PATH)
            .cloned()
            .collect()
    }

    fn result_error<T>(result: Result<T, Error>) -> Option<Error> {
        result.err()
    }

    #[test]
    fn partial_descriptor_acquisition_drops_each_actual_handle_once() {
        for failing_open in 0..7 {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            kernel.borrow_mut().open_failure = Some((failing_open, libc::EACCES));
            let result = LinuxBackend::open_with(syscalls, PID);
            assert_eq!(result_error(result), Some(Error::Unavailable));
            assert_eq!(kernel.borrow().open_calls, failing_open + 1);
            assert_eq!(kernel.borrow().opened_handles.len(), failing_open);
            assert_all_handles_released(&kernel);
        }

        for pid in [0, i32::MAX as u32 + 1] {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            assert_eq!(
                result_error(LinuxBackend::open_with(syscalls, pid)),
                Some(Error::InvalidProcess)
            );
            assert_eq!(kernel.borrow().open_calls, 0);
            assert!(kernel.borrow().opened_handles.is_empty());
        }
    }

    #[test]
    fn capture_failure_after_descriptor_acquisition_drops_every_actual_handle() {
        for failure in [Error::Changed, Error::Unavailable] {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            if failure == Error::Changed {
                kernel.borrow_mut().poll_results.push_back(Ok(PollOutcome {
                    ready: 1,
                    revents: 0,
                }));
            } else {
                kernel
                    .borrow_mut()
                    .read_actions
                    .push_back(ReadAction::Error(libc::EINTR));
            }
            let backend = LinuxBackend::open_with(syscalls, PID)
                .unwrap_or_else(|_| panic!("synthetic Linux descriptor setup failed"));

            let result = capture_with_backend_factory(PID, move |_| {
                Ok(Box::new(backend) as Box<dyn ObservationBackend>)
            });

            assert_eq!(result.err(), Some(failure));
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn production_policy_requests_fixed_nofollow_flags_and_retains_descriptors() {
        let (mut backend, kernel) = open_backend(PID);
        assert_eq!(
            backend.read_boot_id().expect("synthetic boot read"),
            boot_bytes(BOOT_ID)
        );
        assert_eq!(
            backend.read_stat().expect("synthetic stat read"),
            stat_bytes(PID, b"worker", b"S", &TICKS.to_string())
        );
        assert_eq!(backend.poll_pidfd(), Ok(()));

        let directory_flags =
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let regular_flags = libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let events = kernel.borrow().events.clone();
        assert!(events.contains(&format!("open-path:/:{directory_flags}")));
        assert!(
            events
                .iter()
                .any(|event| event == &format!("open-at:/:proc:{directory_flags}"))
        );
        assert!(
            events
                .iter()
                .any(|event| event == &format!("open-at:/proc:sys:{directory_flags}"))
        );
        assert!(
            events
                .iter()
                .any(|event| event == &format!("open-at:/proc/sys:kernel:{directory_flags}"))
        );
        assert!(events.iter().any(|event| event == &format!("open-at:/proc/sys/kernel:random:{directory_flags}")));
        assert!(
            events
                .iter()
                .any(|event| event == &format!("open-at:/proc:4242:{directory_flags}"))
        );
        assert!(
            events.iter().any(|event| event
                == &format!("open-at:/proc/sys/kernel/random:boot_id:{regular_flags}"))
        );
        assert!(
            events
                .iter()
                .any(|event| event == &format!("open-at:/proc/4242:stat:{regular_flags}"))
        );
        assert!(events.contains(&format!("pidfd-open:{PID}:0")));
        assert!(events.contains(&format!("poll:{}:0:0:0", libc::POLLIN)));
        assert!(
            events
                .iter()
                .filter(|event| event.starts_with("fstat-at:"))
                .all(|event| event.contains(&format!(":{}:", libc::AT_SYMLINK_NOFOLLOW)))
        );

        let opened = kernel.borrow().opened_handles.len();
        assert_eq!(opened, 9);
        drop(backend);
        assert_all_handles_released(&kernel);
    }

    #[test]
    fn filesystem_type_and_held_or_named_types_fail_closed() {
        let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
        kernel
            .borrow_mut()
            .filesystem_types
            .insert("/proc".to_owned(), 0);
        assert_eq!(
            result_error(LinuxBackend::open_with(syscalls, PID)),
            Some(Error::UnsafeProcfs)
        );
        assert_all_handles_released(&kernel);

        let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
        kernel
            .borrow_mut()
            .held_overrides
            .insert("/proc".to_owned(), identity(1, 102, libc::S_IFREG));
        assert_eq!(
            result_error(LinuxBackend::open_with(syscalls, PID)),
            Some(Error::UnsafeProcfs)
        );
        assert_all_handles_released(&kernel);

        let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
        kernel
            .borrow_mut()
            .named_overrides
            .insert(("/proc".to_owned(), 1), identity(1, 102, libc::S_IFLNK));
        assert_eq!(
            result_error(LinuxBackend::open_with(syscalls, PID)),
            Some(Error::UnsafeProcfs)
        );
        assert_all_handles_released(&kernel);

        for (device, inode) in [(2, 102), (1, 999)] {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            kernel.borrow_mut().named_overrides.insert(
                ("/proc".to_owned(), 1),
                identity(device, inode, libc::S_IFDIR),
            );
            assert_eq!(
                result_error(LinuxBackend::open_with(syscalls, PID)),
                Some(Error::Changed)
            );
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn leaf_type_symlink_and_identity_replacements_use_shared_metadata_checks() {
        for (held, named, expected) in [
            (
                Some(identity(1, 999, libc::S_IFDIR)),
                None,
                Error::UnsafeProcfs,
            ),
            (
                None,
                Some(identity(1, 999, libc::S_IFLNK)),
                Error::UnsafeProcfs,
            ),
            (None, Some(identity(2, 999, libc::S_IFREG)), Error::Changed),
            (None, Some(identity(1, 999, libc::S_IFREG)), Error::Changed),
        ] {
            let (mut backend, kernel) = open_backend(PID);
            if let Some(identity) = held {
                kernel
                    .borrow_mut()
                    .held_overrides
                    .insert(STAT_PATH.to_owned(), identity);
            }
            if let Some(identity) = named {
                kernel
                    .borrow_mut()
                    .named_overrides
                    .insert((STAT_PATH.to_owned(), 1), identity);
            }
            assert_eq!(backend.read_stat().err(), Some(expected));
            let leaf_drops = kernel
                .borrow()
                .dropped_handles
                .iter()
                .filter(|(_, path)| path == STAT_PATH)
                .count();
            assert_eq!(leaf_drops, 1);
            drop(backend);
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn directory_and_leaf_replacements_are_checked_before_and_after_reads() {
        for (path, occurrence, expected_read) in [
            ("/proc", 2, false),
            ("/proc", 3, true),
            (STAT_PATH, 1, false),
            (STAT_PATH, 2, true),
        ] {
            let (mut backend, kernel) = open_backend(PID);
            let current = kernel
                .borrow()
                .identities
                .get(path)
                .copied()
                .expect("synthetic path identity");
            let replacement = identity(current.device + 1, current.inode + 1, current.kind);
            kernel
                .borrow_mut()
                .named_overrides
                .insert((path.to_owned(), occurrence), replacement);

            assert_eq!(backend.read_stat().err(), Some(Error::Changed));
            let read_seen = kernel
                .borrow()
                .events
                .iter()
                .any(|event| event.starts_with(&format!("read:{STAT_PATH}:")));
            assert_eq!(read_seen, expected_read);
            drop(backend);
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn ordinary_missing_and_permission_errors_are_unavailable_on_the_shared_path() {
        for errno in [libc::ENOENT, libc::EACCES] {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            kernel
                .borrow_mut()
                .named_errors
                .insert(("/proc".to_owned(), 1), errno);
            assert_eq!(
                result_error(LinuxBackend::open_with(syscalls, PID)),
                Some(Error::Unavailable)
            );
            assert_all_handles_released(&kernel);

            let (mut backend, kernel) = open_backend(PID);
            kernel
                .borrow_mut()
                .named_errors
                .insert((STAT_PATH.to_owned(), 1), errno);
            assert_eq!(backend.read_stat().err(), Some(Error::Unavailable));
            drop(backend);
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn bounded_leaf_reader_handles_short_reads_eof_overflow_and_eintr_without_retry() {
        let (mut backend, kernel) = open_backend(PID);
        kernel.borrow_mut().read_chunk_limit = Some(3);
        assert_eq!(
            backend.read_boot_id().expect("short read sequence"),
            boot_bytes(BOOT_ID)
        );
        let boot_reads = kernel
            .borrow()
            .events
            .iter()
            .filter(|event| event.starts_with(&format!("read:{BOOT_PATH}:")))
            .cloned()
            .collect::<Vec<_>>();
        assert!(boot_reads.len() > 2);
        assert!(boot_reads.last().is_some_and(|event| event.ends_with(":0")));
        drop(backend);
        assert_all_handles_released(&kernel);

        let (mut backend, kernel) = open_backend(PID);
        kernel.borrow_mut().read_actions.push_back(ReadAction::Eof);
        assert_eq!(
            backend.read_boot_id().expect("synthetic EOF"),
            Vec::<u8>::new()
        );
        drop(backend);
        assert_all_handles_released(&kernel);

        let (mut backend, kernel) = open_backend(PID);
        kernel
            .borrow_mut()
            .read_files
            .insert(BOOT_PATH.to_owned(), vec![b'x'; 38]);
        assert_eq!(backend.read_boot_id().err(), Some(Error::SizeLimit));
        drop(backend);
        assert_all_handles_released(&kernel);

        for errno in [libc::EINTR, libc::EACCES] {
            let (mut backend, kernel) = open_backend(PID);
            kernel
                .borrow_mut()
                .read_actions
                .push_back(ReadAction::Error(errno));
            assert_eq!(backend.read_boot_id().err(), Some(Error::Unavailable));
            let reads = kernel
                .borrow()
                .events
                .iter()
                .filter(|event| event.starts_with(&format!("read:{BOOT_PATH}:")))
                .count();
            assert_eq!(reads, 1);
            drop(backend);
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn pidfd_poll_statuses_events_and_syscall_errors_are_fail_closed() {
        let cases = [
            (
                Ok(PollOutcome {
                    ready: 0,
                    revents: 0,
                }),
                Ok(()),
            ),
            (
                Ok(PollOutcome {
                    ready: 1,
                    revents: 0,
                }),
                Err(Error::Changed),
            ),
            (
                Ok(PollOutcome {
                    ready: 1,
                    revents: libc::POLLIN,
                }),
                Err(Error::Changed),
            ),
            (
                Ok(PollOutcome {
                    ready: 0,
                    revents: libc::POLLHUP,
                }),
                Err(Error::Changed),
            ),
            (
                Ok(PollOutcome {
                    ready: 0,
                    revents: libc::POLLERR,
                }),
                Err(Error::Changed),
            ),
            (
                Ok(PollOutcome {
                    ready: 0,
                    revents: libc::POLLNVAL,
                }),
                Err(Error::Changed),
            ),
            (Err(libc::EINTR), Err(Error::Unavailable)),
        ];

        for (poll_result, expected) in cases {
            let (mut backend, kernel) = open_backend(PID);
            kernel.borrow_mut().poll_results.push_back(poll_result);
            assert_eq!(backend.poll_pidfd(), expected);
            assert!(
                kernel
                    .borrow()
                    .events
                    .iter()
                    .any(|event| event.starts_with(&format!("poll:{}:0:", libc::POLLIN)))
            );
            drop(backend);
            assert_all_handles_released(&kernel);
        }

        for errno in [libc::ENOSYS, libc::EINVAL, libc::EPERM] {
            let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
            kernel.borrow_mut().open_failure = Some((6, errno));
            let expected = if errno == libc::ENOSYS || errno == libc::EINVAL {
                Error::UnsupportedPlatform
            } else {
                Error::Unavailable
            };
            assert_eq!(
                result_error(LinuxBackend::open_with(syscalls, PID)),
                Some(expected)
            );
            assert_eq!(kernel.borrow().open_calls, 7);
            assert_all_handles_released(&kernel);
        }
    }

    #[test]
    fn successful_revalidation_uses_the_same_retained_pidfd_and_directories() {
        let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
        let backend = LinuxBackend::open_with(syscalls, PID)
            .unwrap_or_else(|_| panic!("synthetic Linux descriptor setup failed"));
        let mut observation = capture_with_backend_factory(PID, move |_| {
            Ok(Box::new(backend) as Box<dyn ObservationBackend>)
        })
        .expect("synthetic capture baseline");
        let retained_before = retained_descriptors(&kernel);
        let boot_ms = observation.observed_boot_ms();
        assert_eq!(retained_before.len(), 7);

        assert_eq!(observation.revalidate(), Ok(boot_ms));
        assert_eq!(retained_descriptors(&kernel), retained_before);
        assert_eq!(
            kernel
                .borrow()
                .opened_handles
                .iter()
                .filter(|(_, path)| path.starts_with("<pidfd:"))
                .count(),
            1
        );
        drop(observation);
        assert_all_handles_released(&kernel);
    }

    #[test]
    fn missing_named_entry_during_revalidation_poisoned_without_more_io() {
        let (syscalls, kernel) = FakeLinuxSyscalls::new(PID);
        let backend = LinuxBackend::open_with(syscalls, PID)
            .unwrap_or_else(|_| panic!("synthetic Linux descriptor setup failed"));
        let mut observation = capture_with_backend_factory(PID, move |_| {
            Ok(Box::new(backend) as Box<dyn ObservationBackend>)
        })
        .expect("synthetic capture baseline");
        let generation = observation.generation();
        let boot_ms = observation.observed_boot_ms();
        let next_proc_lookup = kernel.borrow().named_calls["/proc"] + 1;
        kernel
            .borrow_mut()
            .named_errors
            .insert(("/proc".to_owned(), next_proc_lookup), libc::ENOENT);

        assert_eq!(observation.revalidate(), Err(Error::Unavailable));
        assert_eq!(observation.generation(), generation);
        assert_eq!(observation.observed_boot_ms(), boot_ms);
        let events_after_failure = kernel.borrow().events.len();
        assert_eq!(observation.revalidate(), Err(Error::Invalidated));
        assert_eq!(kernel.borrow().events.len(), events_after_failure);
        drop(observation);
        assert_all_handles_released(&kernel);
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_factories_return_unsupported_before_observation() {
    assert!(matches!(
        super::observe_simulation_process(PID),
        Err(Error::UnsupportedPlatform)
    ));
    assert!(matches!(
        super::observe_current_simulation_process(),
        Err(Error::UnsupportedPlatform)
    ));
}
