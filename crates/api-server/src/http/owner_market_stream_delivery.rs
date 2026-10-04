//! Bounded latest-value delivery. A slow consumer owns one replaceable batch;
//! it never creates a price-history queue or a second lease.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use chrono::{DateTime, Utc};
use tokio::sync::watch;
use tokio::time::{Duration, Instant};
use uuid::Uuid;

use super::owner_market_stream_contract::{
    DeltaBody, InvalidContract, MAX_FRAME_BYTES, MAX_IDENTITIES, ResetBody, RowDto, SnapshotBody,
    StatusBody, event_json,
};

pub(crate) const WRITE_DEADLINE: Duration = Duration::from_secs(5);
pub(crate) const READ_DEADLINE: Duration = Duration::from_secs(1);
pub(crate) const MAX_CONSUMERS_PER_OWNER: usize = 20;

#[derive(Clone, Default)]
pub(crate) struct ConsumerRegistry(Arc<Mutex<HashMap<Uuid, usize>>>);

pub(crate) struct ConsumerPermit {
    registry: ConsumerRegistry,
    owner: Uuid,
}

impl ConsumerRegistry {
    pub(crate) fn acquire(&self, owner: Uuid) -> Result<ConsumerPermit, InvalidContract> {
        if owner.is_nil() {
            return Err(InvalidContract);
        }
        let mut counts = self.0.lock().map_err(|_| InvalidContract)?;
        let count = counts.entry(owner).or_default();
        if *count >= MAX_CONSUMERS_PER_OWNER {
            return Err(InvalidContract);
        }
        *count += 1;
        Ok(ConsumerPermit {
            registry: self.clone(),
            owner,
        })
    }
}

impl Drop for ConsumerPermit {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.registry.0.lock() {
            if let Some(count) = counts.get_mut(&self.owner) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&self.owner);
                }
            }
        }
    }
}

pub(crate) struct Batch {
    pub(crate) bytes: Bytes,
    pub(crate) last_sequence: u64,
}

#[derive(Default)]
pub(crate) struct DeliverySignals {
    pub(crate) delivered: AtomicU64,
    pub(crate) closed: AtomicBool,
}

pub(crate) struct LatestMailbox {
    sender: watch::Sender<Option<Arc<Batch>>>,
    pub(crate) signals: Arc<DeliverySignals>,
    pending_since: Option<Instant>,
    last_sequence: u64,
    last_delivered: u64,
}

impl LatestMailbox {
    pub(crate) fn new() -> (Self, watch::Receiver<Option<Arc<Batch>>>) {
        let (sender, receiver) = watch::channel(None);
        (
            Self {
                sender,
                signals: Arc::new(DeliverySignals::default()),
                pending_since: None,
                last_sequence: 0,
                last_delivered: 0,
            },
            receiver,
        )
    }

    pub(crate) fn pending(&self) -> bool {
        self.signals.delivered.load(Ordering::Acquire) < self.last_sequence
    }

    pub(crate) fn unwritable(&mut self, now: Instant) -> bool {
        let delivered = self.signals.delivered.load(Ordering::Acquire);
        if delivered != self.last_delivered {
            self.last_delivered = delivered;
            self.pending_since = if delivered < self.last_sequence {
                Some(now)
            } else {
                None
            };
        }
        self.pending_since
            .is_some_and(|at| now.saturating_duration_since(at) >= WRITE_DEADLINE)
    }

    pub(crate) fn publish(&mut self, batch: Batch, now: Instant) -> Result<(), InvalidContract> {
        if batch.bytes.len() > MAX_FRAME_BYTES
            || batch.last_sequence <= self.last_sequence
            || self.signals.closed.load(Ordering::Acquire)
            || self.unwritable(now)
        {
            self.close();
            return Err(InvalidContract);
        }
        if self.sender.receiver_count() == 0 {
            self.close();
            return Err(InvalidContract);
        }
        self.pending_since.get_or_insert(now);
        self.last_sequence = batch.last_sequence;
        self.sender.send_replace(Some(Arc::new(batch)));
        Ok(())
    }

    pub(crate) fn close(&mut self) {
        self.signals.closed.store(true, Ordering::Release);
        self.sender.send_replace(None); // erase queued data before EOF/retry
    }
}

impl Drop for LatestMailbox {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) struct DeliveryEncoder {
    stream_id: Uuid,
    sequence: u64,
    previous: Vec<RowDto>,
    initialized: bool,
}

impl DeliveryEncoder {
    pub(crate) fn last_sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn new(stream_id: Uuid) -> Self {
        Self {
            stream_id,
            sequence: 0,
            previous: Vec::new(),
            initialized: false,
        }
    }

    fn event<T: serde::Serialize>(
        &mut self,
        buffer: &mut String,
        name: &str,
        body: T,
        now: DateTime<Utc>,
    ) -> Result<(), InvalidContract> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(InvalidContract)?;
        let json = event_json(self.stream_id, self.sequence, now, &body)?;
        if buffer.len() + json.len() + 128 > MAX_FRAME_BYTES {
            return Err(InvalidContract);
        }
        write!(
            buffer,
            "event: {name}\nid: {}:{}\ndata: {json}\n\n",
            self.stream_id, self.sequence
        )
        .map_err(|_| InvalidContract)
    }

    pub(crate) fn encode(
        &mut self,
        lease_id: Uuid,
        lease_expires_at: DateTime<Utc>,
        rows: Vec<RowDto>,
        status: StatusBody,
        now: DateTime<Utc>,
        replace_pending: bool,
        heartbeat: bool,
    ) -> Result<Batch, InvalidContract> {
        if rows.is_empty() || rows.len() > MAX_IDENTITIES {
            return Err(InvalidContract);
        }
        let identities_changed = self
            .previous
            .iter()
            .map(|v| (v.membership_id, v.generation, v.row_generation))
            .ne(rows
                .iter()
                .map(|v| (v.membership_id, v.generation, v.row_generation)));
        let mut buffer = String::new();
        if !self.initialized || replace_pending || identities_changed {
            self.event(
                &mut buffer,
                "reset",
                ResetBody {
                    reason_code: "RESYNC_REQUIRED",
                },
                now,
            )?;
            self.event(
                &mut buffer,
                "snapshot",
                SnapshotBody {
                    lease_id,
                    lease_expires_at,
                    rows: &rows,
                },
                now,
            )?;
        } else {
            let changed = rows
                .iter()
                .filter(|row| !self.previous.contains(row))
                .cloned()
                .collect::<Vec<_>>();
            if !changed.is_empty() {
                self.event(&mut buffer, "delta", DeltaBody { rows: &changed }, now)?;
            }
        }
        self.event(&mut buffer, "status", status, now)?;
        if heartbeat {
            buffer.push_str(": heartbeat\n\n");
        }
        if buffer.len() > MAX_FRAME_BYTES {
            return Err(InvalidContract);
        }
        self.previous = rows;
        self.initialized = true;
        Ok(Batch {
            bytes: Bytes::from(buffer),
            last_sequence: self.sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(count: usize) -> Vec<RowDto> {
        (1..=count)
            .map(|i| RowDto {
                membership_id: Uuid::from_u128(i as u128),
                instrument_id: format!("{i:06}.KRX"),
                generation: 1,
                row_generation: Uuid::from_u128(100 + i as u128),
                venue: "KRX",
                currency: "KRW",
                source: "KIS_MARKET_WS",
                wire_version: "kis-h0stcnt0-20260914-v1",
                session: None,
                subscription: "DESIRED",
                connection: "DISCONNECTED",
                market_state: "UNKNOWN",
                freshness: "UNAVAILABLE",
                availability: "UNAVAILABLE",
                reason_code: Some("SESSION_WINDOW_UNAVAILABLE"),
                state_version: "0".into(),
                gap_open: false,
                session_has_gap: false,
                gap_generation: "0".into(),
                quote: None,
            })
            .collect()
    }

    fn status() -> StatusBody {
        StatusBody {
            connection: "DISCONNECTED",
            reason_code: Some("SESSION_WINDOW_UNAVAILABLE"),
            gap_open: false,
            session_has_gap: false,
            gap_generation: "0".into(),
        }
    }

    #[test]
    fn initial_delivery_and_backpressure_replacement_always_reset_to_all_thirty_rows() {
        let mut encoder = DeliveryEncoder::new(Uuid::from_u128(900));
        let now = Utc::now();
        let lease = Uuid::from_u128(901);
        let first = encoder
            .encode(lease, now, rows(30), status(), now, false, false)
            .unwrap();
        let first = std::str::from_utf8(&first.bytes).unwrap();
        assert!(first.starts_with("event: reset\n"));
        let snapshot = first
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .nth(1)
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(snapshot).unwrap();
        assert_eq!(json["body"]["rows"].as_array().unwrap().len(), 30);
        let replaced = encoder
            .encode(lease, now, rows(30), status(), now, true, false)
            .unwrap();
        assert!(
            std::str::from_utf8(&replaced.bytes)
                .unwrap()
                .starts_with("event: reset\n")
        );
        assert_eq!(replaced.last_sequence, 6);
        assert!(replaced.bytes.len() < MAX_FRAME_BYTES);
    }

    #[test]
    fn quiet_read_sends_status_and_changes_send_complete_replacement_rows() {
        let mut encoder = DeliveryEncoder::new(Uuid::from_u128(900));
        let now = Utc::now();
        let lease = Uuid::from_u128(901);
        encoder
            .encode(lease, now, rows(2), status(), now, false, false)
            .unwrap();
        let quiet = encoder
            .encode(lease, now, rows(2), status(), now, false, true)
            .unwrap();
        let quiet = std::str::from_utf8(&quiet.bytes).unwrap();
        assert!(quiet.starts_with("event: status\n"));
        assert!(quiet.ends_with(": heartbeat\n\n"));
        let mut next = rows(2);
        next[0].connection = "CONNECTING";
        let changed = encoder
            .encode(lease, now, next, status(), now, false, false)
            .unwrap();
        let changed = std::str::from_utf8(&changed.bytes).unwrap();
        assert!(changed.starts_with("event: delta\n"));
        let json: serde_json::Value = serde_json::from_str(
            changed
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(json["body"]["rows"].as_array().unwrap().len(), 1);
        assert_eq!(json["body"]["rows"][0]["instrument_id"], "000001.KRX");
        assert_eq!(
            json["body"]["rows"][0]["state_version"], "0",
            "read-time status never invents a cache version"
        );
    }

    #[test]
    fn membership_removal_resets_and_sequence_exhaustion_fails_closed() {
        let mut encoder = DeliveryEncoder::new(Uuid::from_u128(900));
        let now = Utc::now();
        let lease = Uuid::from_u128(901);
        encoder
            .encode(lease, now, rows(2), status(), now, false, false)
            .unwrap();
        let removed = encoder
            .encode(lease, now, rows(1), status(), now, false, false)
            .unwrap();
        assert!(
            std::str::from_utf8(&removed.bytes)
                .unwrap()
                .starts_with("event: reset\n")
        );
        encoder.sequence = i64::MAX as u64;
        assert!(
            encoder
                .encode(lease, now, rows(1), status(), now, false, false)
                .is_err()
        );
    }

    #[test]
    fn per_owner_capacity_is_bounded_and_released_with_the_response() {
        let registry = ConsumerRegistry::default();
        let owner = Uuid::from_u128(1);
        let mut permits = (0..20)
            .map(|_| registry.acquire(owner).unwrap())
            .collect::<Vec<_>>();
        assert!(registry.acquire(owner).is_err());
        assert!(registry.acquire(Uuid::from_u128(2)).is_ok());
        permits.pop();
        assert!(registry.acquire(owner).is_ok());
        drop(permits);
        assert!(registry.0.lock().unwrap().is_empty());
    }

    #[test]
    fn replacements_do_not_extend_an_unwritable_consumers_deadline() {
        let (mut mailbox, receiver) = LatestMailbox::new();
        let start = Instant::now();
        for second in 0..5 {
            mailbox
                .publish(
                    Batch {
                        bytes: Bytes::from_static(b"latest"),
                        last_sequence: second + 1,
                    },
                    start + Duration::from_secs(second),
                )
                .unwrap();
        }
        assert!(mailbox.pending());
        assert!(mailbox.unwritable(start + WRITE_DEADLINE));
        assert!(
            mailbox
                .publish(
                    Batch {
                        bytes: Bytes::from_static(b"late"),
                        last_sequence: 6
                    },
                    start + WRITE_DEADLINE
                )
                .is_err()
        );
        assert!(receiver.borrow().is_none());
        assert!(mailbox.signals.closed.load(Ordering::Acquire));
    }

    #[test]
    fn consumed_latest_batch_resets_backpressure_and_close_erases_pending_data() {
        let (mut mailbox, receiver) = LatestMailbox::new();
        let start = Instant::now();
        mailbox
            .publish(
                Batch {
                    bytes: Bytes::from_static(b"first"),
                    last_sequence: 1,
                },
                start,
            )
            .unwrap();
        mailbox.signals.delivered.store(1, Ordering::Release);
        assert!(!mailbox.unwritable(start + Duration::from_secs(4)));
        mailbox
            .publish(
                Batch {
                    bytes: Bytes::from_static(b"next"),
                    last_sequence: 2,
                },
                start + Duration::from_secs(4),
            )
            .unwrap();
        assert!(!mailbox.unwritable(start + Duration::from_secs(8)));
        assert!(mailbox.unwritable(start + Duration::from_secs(9)));
        mailbox.close();
        assert!(receiver.borrow().is_none());
    }

    #[test]
    fn oversized_batch_is_rejected_instead_of_partially_published() {
        let (mut mailbox, receiver) = LatestMailbox::new();
        assert!(
            mailbox
                .publish(
                    Batch {
                        bytes: Bytes::from(vec![0; MAX_FRAME_BYTES + 1]),
                        last_sequence: 1
                    },
                    Instant::now()
                )
                .is_err()
        );
        assert!(receiver.borrow().is_none());
        assert_eq!(MAX_FRAME_BYTES * 4, 256 * 1024);
    }
    #[test]
    fn cache_row_recreation_resets_namespace_without_membership_change() {
        let mut encoder = DeliveryEncoder::new(Uuid::from_u128(900));
        let now = Utc::now();
        let lease = Uuid::from_u128(901);
        let mut prior = rows(30);
        prior[0].state_version = "50".into();
        let old_namespace = prior[0].row_generation;
        let first = encoder
            .encode(lease, now, prior.clone(), status(), now, false, false)
            .unwrap();
        let mut recreated = prior.clone();
        recreated[0].row_generation = Uuid::from_u128(5_000);
        recreated[0].state_version = "0".into();
        assert_eq!(recreated[0].membership_id, prior[0].membership_id);
        assert_eq!(recreated[0].generation, prior[0].generation);
        let next = encoder
            .encode(lease, now, recreated.clone(), status(), now, false, false)
            .unwrap();
        let wire = std::str::from_utf8(&next.bytes).unwrap();
        assert!(wire.starts_with("event: reset\n"));
        assert!(!wire.contains("event: delta\n"));
        assert!(!wire.contains(&old_namespace.to_string()));
        assert_eq!(next.last_sequence, first.last_sequence + 3);
        let payloads = wire
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[0]["body"]["reason_code"], "RESYNC_REQUIRED");
        let fresh = payloads[1]["body"]["rows"].as_array().unwrap();
        assert_eq!(fresh.len(), 30);
        assert_eq!(
            fresh[0]["row_generation"],
            recreated[0].row_generation.to_string()
        );
        assert_eq!(fresh[0]["state_version"], "0");
        let quiet = encoder
            .encode(lease, now, recreated, status(), now, false, false)
            .unwrap();
        let quiet_wire = std::str::from_utf8(&quiet.bytes).unwrap();
        assert!(quiet_wire.starts_with("event: status\n"));
        assert!(!quiet_wire.contains("event: reset\n"));
        assert_eq!(quiet.last_sequence, next.last_sequence + 1);
    }

}
