//! Fixture-only scalar evidence. Heap census and process RSS have distinct scopes.

use std::alloc::System;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

use job_queue::owner_equity_v2::StreamBufferProbeSummary;
use serde::Serialize;

use crate::heap_census::{Census, Window, WindowSnapshot};

const MAX_PROCESS_STATUS_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResourceError {
    HeapWindow,
    HeapDiagnostic,
    CounterInvalid,
    RssRead,
    RssInvalid,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct HeapObservation {
    pub schema_version: u32,
    pub scope: &'static str,
    pub generation: u64,
    pub elapsed_ms: u64,
    pub baseline_current_usable_bytes: u64,
    pub current_usable_bytes: u64,
    pub conservative_peak_usable_bytes: u64,
    pub diagnostic_error: bool,
}

pub(super) struct HeapWindow {
    window: Window<'static, System>,
    started: Instant,
}

impl HeapWindow {
    pub(super) fn begin(census: &'static Census) -> Result<Self, ResourceError> {
        let window = census
            .begin_window()
            .map_err(|_| ResourceError::HeapWindow)?;
        Ok(Self {
            window,
            started: Instant::now(),
        })
    }

    pub(super) fn snapshot(&self) -> Result<HeapObservation, ResourceError> {
        heap_observation(
            self.window
                .snapshot()
                .map_err(|_| ResourceError::HeapWindow)?,
            self.started,
        )
    }

    pub(super) fn finish(self) -> Result<HeapObservation, ResourceError> {
        heap_observation(
            self.window
                .finish()
                .map_err(|_| ResourceError::HeapWindow)?,
            self.started,
        )
    }
}

fn heap_observation(
    value: WindowSnapshot,
    started: Instant,
) -> Result<HeapObservation, ResourceError> {
    if value.diagnostic_error
        || value.generation == 0
        || value.peak_usable_bytes < value.baseline_current_usable_bytes
        || value.peak_usable_bytes < value.current_usable_bytes
    {
        return Err(ResourceError::HeapDiagnostic);
    }
    let elapsed_ms =
        u64::try_from(started.elapsed().as_millis()).map_err(|_| ResourceError::CounterInvalid)?;
    Ok(HeapObservation {
        schema_version: 1,
        scope: "global_allocator_usable_blocks",
        generation: value.generation,
        elapsed_ms,
        baseline_current_usable_bytes: value.baseline_current_usable_bytes,
        current_usable_bytes: value.current_usable_bytes,
        conservative_peak_usable_bytes: value.peak_usable_bytes,
        diagnostic_error: value.diagnostic_error,
    })
}

pub(super) fn buffer_summary_is_valid(value: &StreamBufferProbeSummary) -> bool {
    let sum = [
        value.buffered,
        value.replaced,
        value.stale,
        value.offer_age_lag,
        value.rejected,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add);
    !value.overflowed
        && !value.diagnostic_error
        && sum == Some(value.adapter_calls)
        && value.peak_pending_slots <= 30
        && value.peak_high_water_slots <= 30
        && value.peak_outstanding_slots <= 30
}

pub(super) fn process_rss_bytes() -> Result<u64, ResourceError> {
    let mut file = File::open("/proc/self/status").map_err(|_| ResourceError::RssRead)?;
    let mut bytes = [0_u8; MAX_PROCESS_STATUS_BYTES + 1];
    let mut used = 0;
    loop {
        let read = file
            .read(&mut bytes[used..])
            .map_err(|_| ResourceError::RssRead)?;
        if read == 0 {
            break;
        }
        used += read;
        if used > MAX_PROCESS_STATUS_BYTES {
            return Err(ResourceError::RssInvalid);
        }
    }
    parse_rss_bytes(&bytes[..used])
}

fn parse_rss_bytes(bytes: &[u8]) -> Result<u64, ResourceError> {
    if bytes.len() > MAX_PROCESS_STATUS_BYTES {
        return Err(ResourceError::RssInvalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ResourceError::RssInvalid)?;
    let mut rss = None;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        if rss.is_some() {
            return Err(ResourceError::RssInvalid);
        }
        let mut fields = rest.split_ascii_whitespace();
        let digits = fields.next().ok_or(ResourceError::RssInvalid)?;
        if digits.is_empty()
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || fields.next() != Some("kB")
            || fields.next().is_some()
        {
            return Err(ResourceError::RssInvalid);
        }
        let kib = digits
            .parse::<u64>()
            .map_err(|_| ResourceError::RssInvalid)?;
        if kib == 0 {
            return Err(ResourceError::RssInvalid);
        }
        rss = Some(kib.checked_mul(1024).ok_or(ResourceError::RssInvalid)?);
    }
    rss.ok_or(ResourceError::RssInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_parser_requires_one_positive_exact_kib_field() {
        assert_eq!(
            parse_rss_bytes(b"Name:\tfixture\nVmRSS:\t 123 kB\nThreads:\t4\n"),
            Ok(123 * 1024)
        );
        for bytes in [
            &b"Name: fixture\n"[..],
            &b"VmRSS: 0 kB\n"[..],
            &b"VmRSS: 1 kB\nVmRSS: 2 kB\n"[..],
            &b"VmRSS: 2 MB\n"[..],
            &b"VmRSS: +2 kB\n"[..],
            &b"VmRSS: 2 kB trailing\n"[..],
            &b"VmRSS: 18446744073709551615 kB\n"[..],
            &b"\xff"[..],
        ] {
            assert_eq!(parse_rss_bytes(bytes), Err(ResourceError::RssInvalid));
        }
        assert_eq!(
            parse_rss_bytes(&vec![b' '; MAX_PROCESS_STATUS_BYTES + 1]),
            Err(ResourceError::RssInvalid)
        );
    }

    #[test]
    fn heap_evidence_keeps_scope_and_rejects_bad_peaks() {
        let value = WindowSnapshot {
            generation: 1,
            baseline_current_usable_bytes: 4096,
            peak_usable_bytes: 8192,
            current_usable_bytes: 2048,
            diagnostic_error: false,
        };
        let evidence = heap_observation(value, Instant::now()).unwrap();
        let encoded = serde_json::to_value(evidence).unwrap();
        assert_eq!(encoded["scope"], "global_allocator_usable_blocks");
        assert_eq!(encoded["conservative_peak_usable_bytes"], 8192);
        assert!(encoded.get("rss").is_none());
        for bad in [
            WindowSnapshot {
                generation: 0,
                ..value
            },
            WindowSnapshot {
                diagnostic_error: true,
                ..value
            },
            WindowSnapshot {
                peak_usable_bytes: 4095,
                ..value
            },
            WindowSnapshot {
                current_usable_bytes: 8193,
                ..value
            },
        ] {
            assert_eq!(
                heap_observation(bad, Instant::now()),
                Err(ResourceError::HeapDiagnostic)
            );
        }
    }

    #[test]
    fn buffer_summary_checks_conservation_and_caps() {
        let mut value = StreamBufferProbeSummary {
            adapter_calls: 5,
            buffered: 1,
            replaced: 1,
            stale: 1,
            offer_age_lag: 1,
            rejected: 1,
            handoff_age_lag_drops: 3,
            peak_pending_slots: 30,
            peak_high_water_slots: 30,
            peak_outstanding_slots: 30,
            ..Default::default()
        };
        assert!(buffer_summary_is_valid(&value));
        value.adapter_calls = 6;
        assert!(!buffer_summary_is_valid(&value));
        value.adapter_calls = 5;
        value.peak_pending_slots = 31;
        assert!(!buffer_summary_is_valid(&value));
        value.peak_pending_slots = 30;
        value.peak_high_water_slots = 31;
        assert!(!buffer_summary_is_valid(&value));
        value.peak_high_water_slots = 30;
        value.peak_outstanding_slots = 31;
        assert!(!buffer_summary_is_valid(&value));
    }

    #[test]
    fn buffer_summary_fails_closed_on_flags_or_counter_overflow() {
        assert!(buffer_summary_is_valid(&StreamBufferProbeSummary::default()));
        for value in [
            StreamBufferProbeSummary {
                overflowed: true,
                ..Default::default()
            },
            StreamBufferProbeSummary {
                diagnostic_error: true,
                ..Default::default()
            },
            StreamBufferProbeSummary {
                adapter_calls: u64::MAX,
                buffered: u64::MAX,
                replaced: 1,
                ..Default::default()
            },
        ] {
            assert!(!buffer_summary_is_valid(&value));
        }
    }
}
