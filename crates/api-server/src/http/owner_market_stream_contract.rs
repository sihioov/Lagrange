//! Strict schema-2 HTTP values. This module performs no authentication,
//! database access, demand renewal, or provider operation.
//! Callers authenticate before passing bounded request bytes to these parsers.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use job_queue::owner_equity_v2::{StreamLease, StreamLeaseIdentity, StreamLeaseRequest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_REQUEST_BYTES: usize = 16 * 1024;
pub const MAX_IDENTITIES: usize = 30;
pub const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
// Four simultaneously retained serialization/delivery buffers fit the
// 256 KiB budget. No buffer or event is allowed to truncate a 30-row set.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidContract;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityDto {
    pub membership_id: String,
    pub instrument_id: String,
    pub generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseBody {
    schema_version: u32,
    consumer_id: String,
    renewal_sequence: u64,
    identities: Vec<IdentityDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseBody {
    schema_version: u32,
    consumer_id: String,
    renewal_sequence: u64,
}

pub struct ValidatedLease {
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
    pub identities: Vec<StreamLeaseIdentity>,
}

impl ValidatedLease {
    pub fn into_storage(
        self,
        idempotency_key: String,
    ) -> Result<StreamLeaseRequest, InvalidContract> {
        StreamLeaseRequest::new(
            self.consumer_id,
            self.renewal_sequence,
            self.identities,
            idempotency_key,
        )
        .map_err(|_| InvalidContract)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedRelease {
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
}

pub fn canonical_uuid(value: &str) -> Result<Uuid, InvalidContract> {
    let uuid = Uuid::parse_str(value).map_err(|_| InvalidContract)?;
    if uuid.is_nil() || uuid.to_string() != value {
        return Err(InvalidContract);
    }
    Ok(uuid)
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, InvalidContract> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(InvalidContract);
    }
    // Never return serde's free-form message: it can contain untrusted input.
    serde_json::from_slice(bytes).map_err(|_| InvalidContract)
}

fn identities(values: &[IdentityDto]) -> Result<Vec<StreamLeaseIdentity>, InvalidContract> {
    if values.is_empty() || values.len() > MAX_IDENTITIES {
        return Err(InvalidContract);
    }
    let mut previous = "";
    let mut instruments = BTreeSet::new();
    values
        .iter()
        .map(|value| {
            let membership = canonical_uuid(&value.membership_id)?;
            if value.membership_id.as_str() <= previous
                || !instruments.insert(value.instrument_id.as_str())
                || value.generation == 0
                || value.generation > MAX_SAFE_INTEGER
            {
                return Err(InvalidContract);
            }
            previous = &value.membership_id;
            StreamLeaseIdentity::new(membership, value.instrument_id.clone(), value.generation)
                .map_err(|_| InvalidContract)
        })
        .collect()
}

pub fn parse_lease(bytes: &[u8]) -> Result<ValidatedLease, InvalidContract> {
    let body: LeaseBody = parse(bytes)?;
    if body.schema_version != 2 || body.renewal_sequence > MAX_SAFE_INTEGER {
        return Err(InvalidContract);
    }
    Ok(ValidatedLease {
        consumer_id: canonical_uuid(&body.consumer_id)?,
        renewal_sequence: body.renewal_sequence,
        identities: identities(&body.identities)?,
    })
}

pub fn parse_release(bytes: &[u8]) -> Result<ValidatedRelease, InvalidContract> {
    let body: ReleaseBody = parse(bytes)?;
    if body.schema_version != 2 || body.renewal_sequence > MAX_SAFE_INTEGER {
        return Err(InvalidContract);
    }
    Ok(ValidatedRelease {
        consumer_id: canonical_uuid(&body.consumer_id)?,
        renewal_sequence: body.renewal_sequence,
    })
}

#[derive(Debug, Serialize)]
pub struct LeaseDto {
    pub schema_version: u32,
    pub lease_id: Uuid,
    pub consumer_id: Uuid,
    pub renewal_sequence: u64,
    pub lease_expires_at: DateTime<Utc>,
    pub renew_after_ms: u32,
    pub identities: Vec<IdentityDto>,
}

impl TryFrom<StreamLease> for LeaseDto {
    type Error = InvalidContract;

    fn try_from(lease: StreamLease) -> Result<Self, Self::Error> {
        if lease.lease_id.is_nil()
            || lease.consumer_id.is_nil()
            || lease.renewal_sequence > MAX_SAFE_INTEGER
            || lease.renew_after_ms != 15_000
        {
            return Err(InvalidContract);
        }
        let mut values = lease
            .identities
            .into_iter()
            .map(|identity| IdentityDto {
                membership_id: identity.membership_id.to_string(),
                instrument_id: identity.instrument_id,
                generation: identity.generation,
            })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.membership_id.cmp(&b.membership_id));
        identities(&values)?;
        Ok(Self {
            schema_version: 2,
            lease_id: lease.lease_id,
            consumer_id: lease.consumer_id,
            renewal_sequence: lease.renewal_sequence,
            lease_expires_at: lease.lease_expires_at,
            renew_after_ms: 15_000,
            identities: values,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionDto {
    pub date: NaiveDate,
    pub timezone: &'static str,
    pub calendar_source: &'static str,
    pub calendar_source_version: &'static str,
    pub calendar_content_sha256: String,
    pub window_contract_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuoteDto {
    pub price: String,
    pub base_price: Option<String>,
    pub base_price_reason: &'static str,
    pub change_from_previous_day: String,
    pub change_percent_from_previous_day: String,
    pub direction: &'static str,
    pub trade_volume: String,
    pub cumulative_volume: String,
    pub halted: bool,
    pub business_date: NaiveDate,
    pub trade_time: String,
    pub provider_trade_at: String,
    pub received_at: DateTime<Utc>,
    pub committed_at: DateTime<Utc>,
    pub epoch: Uuid,
    pub quote_version: String,
    pub receive_ordinal: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowDto {
    pub membership_id: Uuid,
    pub instrument_id: String,
    pub generation: u64,
    pub row_generation: Uuid,
    pub venue: &'static str,
    pub currency: &'static str,
    pub source: &'static str,
    pub wire_version: &'static str,
    pub session: Option<SessionDto>,
    pub subscription: &'static str,
    pub connection: &'static str,
    pub market_state: &'static str,
    pub freshness: &'static str,
    pub availability: &'static str,
    pub reason_code: Option<&'static str>,
    pub state_version: String,
    pub gap_open: bool,
    pub session_has_gap: bool,
    pub gap_generation: String,
    pub quote: Option<QuoteDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusBody {
    pub connection: &'static str,
    pub reason_code: Option<&'static str>,
    pub gap_open: bool,
    pub session_has_gap: bool,
    pub gap_generation: String,
}

#[derive(Serialize)]
pub struct SnapshotBody<'a> {
    pub lease_id: Uuid,
    pub lease_expires_at: DateTime<Utc>,
    pub rows: &'a [RowDto],
}

#[derive(Serialize)]
pub struct DeltaBody<'a> {
    pub rows: &'a [RowDto],
}

#[derive(Serialize)]
pub struct ResetBody {
    pub reason_code: &'static str,
}

#[derive(Serialize)]
struct EventEnvelope<'a, T> {
    schema_version: u32,
    stream_id: Uuid,
    event_sequence: String,
    server_time: DateTime<Utc>,
    body: &'a T,
}

pub fn event_json<T: Serialize>(
    stream_id: Uuid,
    sequence: u64,
    server_time: DateTime<Utc>,
    body: &T,
) -> Result<String, InvalidContract> {
    if stream_id.is_nil() || sequence == 0 || sequence > i64::MAX as u64 {
        return Err(InvalidContract);
    }
    let json = serde_json::to_string(&EventEnvelope {
        schema_version: 2,
        stream_id,
        event_sequence: sequence.to_string(),
        server_time,
        body,
    })
    .map_err(|_| InvalidContract)?;
    if json.len() > MAX_FRAME_BYTES {
        return Err(InvalidContract);
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> serde_json::Value {
        json!({
            "schema_version": 2,
            "consumer_id": "00000000-0000-4000-8000-000000000099",
            "renewal_sequence": 0,
            "identities": [{"membership_id": "00000000-0000-4000-8000-000000000001",
                "instrument_id": "005930.KRX", "generation": 1}]
        })
    }

    fn accepted(value: serde_json::Value) -> bool {
        parse_lease(&serde_json::to_vec(&value).unwrap()).is_ok()
    }

    #[test]
    fn initial_zero_and_safe_integer_limits_are_exact() {
        assert!(accepted(request()));
        let mut value = request();
        value["renewal_sequence"] = json!(MAX_SAFE_INTEGER);
        value["identities"][0]["generation"] = json!(MAX_SAFE_INTEGER);
        assert!(accepted(value.clone()));
        value["renewal_sequence"] = json!(MAX_SAFE_INTEGER + 1);
        assert!(!accepted(value));
        for generation in [json!(0), json!(-1), json!(1.5), json!(MAX_SAFE_INTEGER + 1)] {
            let mut value = request();
            value["identities"][0]["generation"] = generation;
            assert!(!accepted(value));
        }
    }

    #[test]
    fn canonical_uuids_and_exact_identity_sets_are_required() {
        for id in [
            "00000000000040008000000000000001",
            "00000000-0000-0000-0000-000000000000",
            "AAAAAAAA-0000-4000-8000-000000000001",
        ] {
            let mut value = request();
            value["consumer_id"] = json!(id);
            assert!(!accepted(value));
        }
        let mut value = request();
        value["identities"] = json!([]);
        assert!(!accepted(value));
        let mut value = request();
        let first = value["identities"][0].clone();
        value["identities"] = json!([first, first]);
        assert!(!accepted(value.clone()));
        value["identities"][1]["membership_id"] = json!("00000000-0000-4000-8000-000000000002");
        assert!(!accepted(value.clone())); // duplicate instrument
        value["identities"][1]["instrument_id"] = json!("000660.KRX");
        assert!(accepted(value.clone()));
        value["identities"].as_array_mut().unwrap().swap(0, 1);
        assert!(!accepted(value));
    }

    #[test]
    fn unknown_fields_duplicate_keys_and_oversized_bodies_are_rejected() {
        for name in [
            "owner_user_id",
            "credential_slot_id",
            "app_key",
            "source",
            "request",
        ] {
            let mut value = request();
            value[name] = json!("sentinel");
            assert!(!accepted(value));
        }
        let mut value = request();
        value["identities"][0]["unexpected"] = json!(true);
        assert!(!accepted(value));
        let text = serde_json::to_string(&request()).unwrap().replace(
            "\"schema_version\":2",
            "\"schema_version\":2,\"schema_version\":2",
        );
        assert!(parse_lease(text.as_bytes()).is_err());
        assert!(parse_lease(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
    }

    #[test]
    fn thirty_identities_are_bounded_without_silent_truncation() {
        let mut value = request();
        value["identities"] = json!(
            (1..=30)
                .map(|n| json!({
                    "membership_id": Uuid::from_u128(n).to_string(),
                    "instrument_id": format!("{:06}.KRX", 100_000 + n),
                    "generation": 1,
                }))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            parse_lease(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .identities
                .len(),
            30
        );
        value["identities"].as_array_mut().unwrap().push(json!({
            "membership_id":Uuid::from_u128(31).to_string(),
            "instrument_id":"100031.KRX", "generation":1
        }));
        assert!(!accepted(value));
    }

    #[test]
    fn lease_response_is_canonical_and_excludes_private_identity_fields() {
        use job_queue::owner_equity_v2::StreamIdentity;
        let owner = Uuid::from_u128(90);
        let expires = DateTime::parse_from_rfc3339("2026-10-03T06:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let lease = StreamLease {
            lease_id: Uuid::from_u128(80),
            owner_user_id: owner,
            consumer_id: Uuid::from_u128(81),
            renewal_sequence: 0,
            lease_expires_at: expires,
            renew_after_ms: 15_000,
            identities: vec![
                StreamIdentity::new(
                    owner,
                    Uuid::from_u128(2),
                    Uuid::from_u128(92),
                    "000660.KRX".into(),
                    1,
                )
                .unwrap(),
                StreamIdentity::new(
                    owner,
                    Uuid::from_u128(1),
                    Uuid::from_u128(91),
                    "005930.KRX".into(),
                    1,
                )
                .unwrap(),
            ],
        };
        let response = serde_json::to_value(LeaseDto::try_from(lease).unwrap()).unwrap();
        assert_eq!(response.as_object().unwrap().len(), 7);
        assert_eq!(
            response["identities"][0]["membership_id"],
            Uuid::from_u128(1).to_string()
        );
        assert_eq!(response["lease_expires_at"], "2026-10-03T06:00:30Z");
        assert_eq!(response["identities"][0].as_object().unwrap().len(), 3);
        let serialized = response.to_string();
        for private in [
            "owner_user_id",
            "generation_id",
            "session_hash",
            "credential_slot_id",
            "fencing_token",
            "grant_id",
        ] {
            assert!(!serialized.contains(private));
        }
    }

    #[test]
    fn release_requires_the_same_strict_schema_and_last_sequence_shape() {
        let good = json!({"schema_version":2,"consumer_id":"00000000-0000-4000-8000-000000000099","renewal_sequence":0});
        assert_eq!(
            parse_release(&serde_json::to_vec(&good).unwrap())
                .unwrap()
                .renewal_sequence,
            0
        );
        let mut bad = good.clone();
        bad["schema_version"] = json!(1);
        assert!(parse_release(&serde_json::to_vec(&bad).unwrap()).is_err());
        let mut bad = good;
        bad["renewal_sequence"] = json!(MAX_SAFE_INTEGER + 1);
        assert!(parse_release(&serde_json::to_vec(&bad).unwrap()).is_err());
    }

    #[test]
    fn event_has_exact_nested_envelope_and_string_cursor() {
        let stream = Uuid::from_u128(1);
        let now = DateTime::parse_from_rfc3339("2026-10-03T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let body = ResetBody {
            reason_code: "RESYNC_REQUIRED",
        };
        let value: serde_json::Value =
            serde_json::from_str(&event_json(stream, i64::MAX as u64, now, &body).unwrap())
                .unwrap();
        assert_eq!(
            value,
            json!({"schema_version":2,"stream_id":stream.to_string(),"event_sequence":i64::MAX.to_string(),"server_time":"2026-10-03T06:00:00Z","body":{"reason_code":"RESYNC_REQUIRED"}})
        );
        assert!(event_json(stream, 0, now, &body).is_err());
        assert!(event_json(stream, i64::MAX as u64 + 1, now, &body).is_err());
        assert!(event_json(Uuid::nil(), 1, now, &body).is_err());
        assert!(event_json(stream, 1, now, &"x".repeat(MAX_FRAME_BYTES)).is_err());
    }
}
