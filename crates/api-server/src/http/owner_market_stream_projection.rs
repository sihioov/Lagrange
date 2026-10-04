//! Projection of one actor-authorized repository read. No provider I/O and
//! no reconstruction of calendar or publication proofs at the HTTP boundary.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use job_queue::owner_equity_v2::{
    StreamConnectionState as Connection, StreamDeliveryRow, StreamSnapshot,
    StreamStatusCode as Code, StreamSubscriptionDeliveryState as Subscription,
};
use std::collections::BTreeSet;

use super::owner_market_stream_contract::{
    InvalidContract, MAX_IDENTITIES, MAX_SAFE_INTEGER, QuoteDto, RowDto, SessionDto, StatusBody,
};

pub(crate) fn connection(value: Connection) -> &'static str {
    match value {
        Connection::Disconnected => "DISCONNECTED",
        Connection::Connecting => "CONNECTING",
        Connection::Connected => "CONNECTED",
        Connection::Backoff => "BACKOFF",
        Connection::Stopped => "STOPPED",
    }
}

pub(crate) fn reason(value: Code) -> &'static str {
    match value {
        Code::FeatureDisabled => "FEATURE_DISABLED",
        Code::QuoteStale => "QUOTE_STALE",
        Code::ProducerUnavailable => "PRODUCER_UNAVAILABLE",
        Code::NoActiveDemand => "NO_ACTIVE_DEMAND",
        Code::CalendarUnavailable => "CALENDAR_UNAVAILABLE",
        Code::SessionWindowUnavailable => "SESSION_WINDOW_UNAVAILABLE",
        Code::SessionClosed => "SESSION_CLOSED",
        Code::AwaitingFirstTrade => "AWAITING_FIRST_TRADE",
        Code::ConnectionLost => "CONNECTION_LOST",
        Code::ReconnectGap => "RECONNECT_GAP",
        Code::SubscriptionPending => "SUBSCRIPTION_PENDING",
        Code::SubscriptionRejected => "SUBSCRIPTION_REJECTED",
        Code::SubscriptionAmbiguous => "SUBSCRIPTION_AMBIGUOUS",
        Code::ApprovalUnavailable => "APPROVAL_UNAVAILABLE",
        Code::BudgetExhausted => "BUDGET_EXHAUSTED",
        Code::PipelineLag => "PIPELINE_LAG",
        Code::WireSchemaMismatch => "WIRE_SCHEMA_MISMATCH",
        Code::ProviderResponseInvalid => "PROVIDER_RESPONSE_INVALID",
        Code::QuoteValueInvalid => "QUOTE_VALUE_INVALID",
        Code::MarketClassUnsupported => "MARKET_CLASS_UNSUPPORTED",
        Code::LocalIngressLimit => "LOCAL_INGRESS_LIMIT",
        Code::ResyncRequired => "RESYNC_REQUIRED",
        Code::AccessRevoked => "ACCESS_REVOKED",
    }
}

fn recent(time: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    time <= now && now.signed_duration_since(time) <= Duration::seconds(30)
}

pub(crate) fn row(value: &StreamDeliveryRow, has_window: bool) -> Result<RowDto, InvalidContract> {
    if value.identity.generation == 0
        || value.identity.generation > MAX_SAFE_INTEGER
        || value.row_generation.is_nil()
        || value.state_version > i64::MAX as u64
        || value
            .producer
            .as_ref()
            .is_some_and(|p| p.gap_generation > i64::MAX as u64)
    {
        return Err(InvalidContract);
    }
    let subscription = match value.subscription.as_ref().map(|v| v.state) {
        Some(Subscription::Desired) => "DESIRED",
        Some(
            Subscription::PendingSubscribe
            | Subscription::PendingUnsubscribe
            | Subscription::Ambiguous,
        ) => "PENDING",
        Some(Subscription::Acked) => "ACKED",
        Some(Subscription::Rejected) => "REJECTED",
        Some(Subscription::Absent) | None => "ABSENT",
    };
    let producer = value.producer.as_ref();
    let mut output = RowDto {
        membership_id: value.identity.membership_id,
        instrument_id: value.identity.instrument_id.clone(),
        generation: value.identity.generation,
        row_generation: value.row_generation,
        venue: "KRX",
        currency: "KRW",
        source: "KIS_MARKET_WS",
        wire_version: "kis-h0stcnt0-20260914-v1",
        session: None,
        subscription,
        connection: producer
            .map(|p| connection(p.connection))
            .unwrap_or("DISCONNECTED"),
        market_state: "UNKNOWN",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reason_code: Some(if has_window {
            "CALENDAR_UNAVAILABLE"
        } else {
            "SESSION_WINDOW_UNAVAILABLE"
        }),
        state_version: value.state_version.to_string(),
        gap_open: producer.is_some_and(|p| p.gap_since.is_some()),
        session_has_gap: producer.is_some_and(|p| p.session_has_gap || p.gap_since.is_some()),
        gap_generation: producer.map(|p| p.gap_generation).unwrap_or(0).to_string(),
        quote: None,
    };
    let Some(evidence) = value.read_evidence.as_ref() else {
        return Ok(output);
    };
    if producer.and_then(|p| p.reason).is_some_and(|v| {
        matches!(
            v,
            Code::FeatureDisabled
                | Code::CalendarUnavailable
                | Code::SessionWindowUnavailable
                | Code::AccessRevoked
        )
    }) {
        output.reason_code = producer.and_then(|p| p.reason).map(reason);
        return Ok(output);
    }
    let now = evidence.observed_at;
    output.session = Some(SessionDto {
        date: evidence.session_date,
        timezone: "Asia/Seoul",
        calendar_source: "kis",
        calendar_source_version: "kis-chk-holiday-v1:schema-1",
        calendar_content_sha256: evidence.calendar_content_sha256.clone(),
        window_contract_sha256: evidence.window_contract_sha256.clone(),
    });
    let open = evidence.open_at <= now && now < evidence.close_at;
    output.market_state = if open { "OPEN" } else { "CLOSED" };
    let healthy = producer.is_some_and(|p| {
        p.heartbeat_at <= now
            && now.signed_duration_since(p.heartbeat_at) <= Duration::seconds(10)
            && p.lease_expires_at > now + Duration::seconds(5)
    });
    if !healthy {
        output.connection = "DISCONNECTED";
    }
    if evidence.quote_eligible {
        let cache = value.cache.as_ref().ok_or(InvalidContract)?;
        let quote = cache.quote.as_ref().ok_or(InvalidContract)?;
        let received_at = cache.received_at.ok_or(InvalidContract)?;
        let committed_at = cache.committed_at.ok_or(InvalidContract)?;
        let epoch = cache.epoch.filter(|v| !v.is_nil()).ok_or(InvalidContract)?;
        let ordinal = cache
            .receive_ordinal
            .filter(|v| *v > 0)
            .ok_or(InvalidContract)?;
        if cache.quote_version == 0 || quote.base_price.is_some() {
            return Err(InvalidContract);
        }
        output.freshness = if recent(received_at, now)
            && recent(quote.provider_trade_at.with_timezone(&Utc), now)
        {
            "RECENT"
        } else {
            "STALE"
        };
        output.availability = "LAST_KNOWN";
        output.quote = Some(QuoteDto {
            price: quote.price.clone(),
            base_price: None,
            base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
            change_from_previous_day: quote.change_from_previous_day.clone(),
            change_percent_from_previous_day: quote.change_percent_from_previous_day.clone(),
            direction: quote.direction.as_str(),
            trade_volume: quote.trade_volume.to_string(),
            cumulative_volume: quote.cumulative_volume.to_string(),
            halted: quote.halted,
            business_date: quote.business_date,
            trade_time: quote.trade_time.format("%H:%M:%S").to_string(),
            provider_trade_at: quote
                .provider_trade_at
                .to_rfc3339_opts(SecondsFormat::Secs, false),
            received_at,
            committed_at,
            epoch,
            quote_version: cache.quote_version.to_string(),
            receive_ordinal: ordinal.to_string(),
        });
    }
    // Conditions are derived from the current read, rather than blindly
    // replaying a cache status that was committed before a quiet interval.
    output.reason_code = if !open {
        Some("SESSION_CLOSED")
    } else if !healthy {
        Some("PRODUCER_UNAVAILABLE")
    } else if let Some(code) = producer.and_then(|p| p.reason) {
        Some(reason(code))
    } else if output.gap_open {
        Some("RECONNECT_GAP")
    } else if output.connection != "CONNECTED" {
        Some("CONNECTION_LOST")
    } else if subscription == "REJECTED" {
        Some("SUBSCRIPTION_REJECTED")
    } else if subscription != "ACKED" {
        Some("SUBSCRIPTION_PENDING")
    } else if output.quote.is_none() {
        Some("AWAITING_FIRST_TRADE")
    } else if output.freshness == "STALE" {
        Some("QUOTE_STALE")
    } else if !value.live {
        Some("RESYNC_REQUIRED")
    } else {
        None
    };
    if value.live
        && output.quote.is_some()
        && output.reason_code.is_none()
        && output.freshness == "RECENT"
    {
        output.availability = "LIVE";
    } else if output.quote.is_none() && output.reason_code == Some("AWAITING_FIRST_TRADE") {
        output.availability = "AWAITING_FIRST_TRADE";
    }
    Ok(output)
}

pub(crate) fn rows(
    snapshot: &StreamSnapshot,
    has_window: bool,
) -> Result<Vec<RowDto>, InvalidContract> {
    if snapshot.delivery_rows.is_empty() || snapshot.delivery_rows.len() > MAX_IDENTITIES {
        return Err(InvalidContract);
    }
    let mut result = snapshot
        .delivery_rows
        .iter()
        .map(|v| row(v, has_window))
        .collect::<Result<Vec<_>, _>>()?;
    result.sort_by_key(|v| v.membership_id);
    let mut instruments = BTreeSet::new();
    if result
        .windows(2)
        .any(|w| w[0].membership_id == w[1].membership_id)
        || result
            .iter()
            .any(|row| !instruments.insert(&row.instrument_id))
    {
        return Err(InvalidContract);
    }
    Ok(result)
}

pub(crate) fn status(rows: &[RowDto]) -> Result<StatusBody, InvalidContract> {
    // One credential slot owns the connection. Per-symbol ACK/quote reasons
    // remain in each row; never promote one symbol's stale quote to all rows.
    let first = rows.first().ok_or(InvalidContract)?;
    let connection_reason = first.reason_code.filter(|v| {
        !matches!(
            *v,
            "AWAITING_FIRST_TRADE"
                | "QUOTE_STALE"
                | "SUBSCRIPTION_PENDING"
                | "SUBSCRIPTION_REJECTED"
        )
    });
    Ok(StatusBody {
        connection: first.connection,
        reason_code: connection_reason,
        gap_open: first.gap_open,
        session_has_gap: first.session_has_gap,
        gap_generation: first.gap_generation.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use job_queue::owner_equity_v2::{
        StreamCacheRow, StreamDeliveryEvidence, StreamIdentity, StreamProducerDelivery,
        StreamSubscriptionDelivery,
    };
    use market_data::market_stream::{StreamBasePriceReason, StreamQuote, StreamQuoteDirection};
    use uuid::Uuid;

    // Projection inputs only. This fixture does not fabricate a transport
    // receipt, publication capability, durable grant or DB acceptance result.
    fn fixture() -> StreamDeliveryRow {
        let now = DateTime::parse_from_rfc3339("2026-10-03T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let provider = now.with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap());
        let identity = StreamIdentity::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            "005930.KRX".into(),
            1,
        )
        .unwrap();
        let epoch = Uuid::from_u128(4);
        let row_generation = Uuid::from_u128(5);
        let date = provider.date_naive();
        StreamDeliveryRow {
            identity: identity.clone(),
            row_generation,
            state_version: 7,
            live: true,
            cache: Some(StreamCacheRow {
                identity,
                row_generation,
                quote_version: 8,
                state_version: 7,
                status: None,
                epoch: Some(epoch),
                receive_ordinal: Some(9),
                received_at: Some(now),
                committed_at: Some(now),
                gap_open: false,
                gap_generation: 0,
                quote: Some(StreamQuote {
                    symbol: "005930".into(),
                    business_date: date,
                    trade_time: provider.time(),
                    provider_trade_at: provider,
                    price: "70000".into(),
                    change_from_previous_day: "100".into(),
                    change_percent_from_previous_day: "0.14".into(),
                    direction: StreamQuoteDirection::Up,
                    trade_volume: 1,
                    cumulative_volume: 1000,
                    base_price: None,
                    base_price_reason: StreamBasePriceReason::NotProvidedByChannel,
                    halted: true,
                    opening_class: "0".into(),
                    hour_class: "0".into(),
                    market_class: "J".into(),
                    transaction_class: "0".into(),
                }),
            }),
            subscription: Some(StreamSubscriptionDelivery {
                state: Subscription::Acked,
                epoch: Some(epoch),
                revision: Uuid::from_u128(6),
                updated_at: now,
            }),
            producer: Some(StreamProducerDelivery {
                connection: Connection::Connected,
                reason: None,
                status_at: None,
                heartbeat_at: now,
                lease_expires_at: now + Duration::seconds(30),
                current_epoch: Some(epoch),
                session_date: Some(date),
                session_proof_id: Some(Uuid::from_u128(10)),
                session_proof_sha256: Some("a".repeat(64)),
                calendar_source_batch_id: Some(Uuid::from_u128(11)),
                calendar_content_sha256: Some("b".repeat(64)),
                window_contract_sha256: Some("c".repeat(64)),
                gap_since: None,
                session_has_gap: false,
                gap_generation: 0,
                state_version: 7,
            }),
            read_evidence: Some(StreamDeliveryEvidence {
                observed_at: now,
                session_date: date,
                calendar_content_sha256: "b".repeat(64),
                window_contract_sha256: "c".repeat(64),
                open_at: now - Duration::hours(1),
                close_at: now + Duration::hours(5),
                quote_eligible: true,
            }),
        }
    }

    #[test]
    fn valid_recent_projection_preserves_capture_and_halt_semantics() {
        let input = fixture();
        let output = row(&input, true).unwrap();
        assert_eq!(
            (output.availability, output.freshness, output.market_state),
            ("LIVE", "RECENT", "OPEN")
        );
        assert_eq!(output.reason_code, None);
        let quote = output.quote.unwrap();
        assert!(quote.halted);
        assert_eq!(quote.base_price, None);
        assert_eq!(
            quote.received_at,
            input.cache.as_ref().unwrap().received_at.unwrap()
        );
        assert_eq!(
            (quote.quote_version.as_str(), quote.receive_ordinal.as_str()),
            ("8", "9")
        );
        assert_eq!(quote.provider_trade_at, "2026-10-03T10:00:00+09:00");
        let encoded = serde_json::to_string(&row(&input, true).unwrap()).unwrap();
        for private in [
            "owner_user_id",
            "session_hash",
            "credential_slot_id",
            "fencing_token",
            "session_proof_id",
        ] {
            assert!(!encoded.contains(private));
        }
    }

    #[test]
    fn quiet_age_and_reconnect_keep_original_last_known_namespace() {
        let mut input = fixture();
        let original = row(&input, true).unwrap().quote.unwrap();
        let now = input.read_evidence.as_ref().unwrap().observed_at + Duration::seconds(31);
        input.read_evidence.as_mut().unwrap().observed_at = now;
        let producer = input.producer.as_mut().unwrap();
        producer.heartbeat_at = now;
        producer.lease_expires_at = now + Duration::seconds(30);
        input.live = false;
        let aged = row(&input, true).unwrap();
        assert_eq!(
            (aged.availability, aged.freshness, aged.reason_code),
            ("LAST_KNOWN", "STALE", Some("QUOTE_STALE"))
        );
        assert_eq!(aged.quote.unwrap(), original);
        let producer = input.producer.as_mut().unwrap();
        producer.current_epoch = Some(Uuid::from_u128(100));
        producer.gap_since = Some(now);
        producer.gap_generation = 1;
        let gap = row(&input, true).unwrap();
        assert_eq!(gap.reason_code, Some("RECONNECT_GAP"));
        assert!(gap.gap_open && gap.session_has_gap);
        assert_eq!(gap.quote.unwrap(), original);
    }

    #[test]
    fn half_open_close_keeps_last_known_and_invalid_proof_clears_it() {
        let mut input = fixture();
        let now = input.read_evidence.as_ref().unwrap().observed_at;
        input.read_evidence.as_mut().unwrap().close_at = now;
        input.live = false;
        let closed = row(&input, true).unwrap();
        assert_eq!(
            (closed.market_state, closed.availability, closed.reason_code),
            ("CLOSED", "LAST_KNOWN", Some("SESSION_CLOSED"))
        );
        assert!(closed.quote.is_some());
        input.read_evidence = None;
        for (window, expected) in [
            (false, "SESSION_WINDOW_UNAVAILABLE"),
            (true, "CALENDAR_UNAVAILABLE"),
        ] {
            let missing = row(&input, window).unwrap();
            assert_eq!(missing.reason_code, Some(expected));
            assert!(missing.quote.is_none() && missing.session.is_none());
            assert_eq!(
                (missing.availability, missing.market_state),
                ("UNAVAILABLE", "UNKNOWN")
            );
        }
    }

    #[test]
    fn unhealthy_or_unacked_producer_never_promotes_live() {
        let mut input = fixture();
        input.producer.as_mut().unwrap().heartbeat_at -= Duration::seconds(11);
        let output = row(&input, true).unwrap();
        assert_eq!(
            (output.availability, output.connection, output.reason_code),
            ("LAST_KNOWN", "DISCONNECTED", Some("PRODUCER_UNAVAILABLE"))
        );
        let mut input = fixture();
        input.subscription.as_mut().unwrap().state = Subscription::PendingSubscribe;
        let output = row(&input, true).unwrap();
        assert_eq!(
            (output.availability, output.reason_code),
            ("LAST_KNOWN", Some("SUBSCRIPTION_PENDING"))
        );
        input.read_evidence.as_mut().unwrap().quote_eligible = false;
        input.cache = None;
        input.subscription.as_mut().unwrap().state = Subscription::Acked;
        let output = row(&input, true).unwrap();
        assert_eq!(output.availability, "AWAITING_FIRST_TRADE");
        assert!(output.quote.is_none());
    }

    #[test]
    fn unavailable_authority_and_incomplete_capture_never_expose_prices() {
        for reason in [
            Code::FeatureDisabled,
            Code::CalendarUnavailable,
            Code::SessionWindowUnavailable,
            Code::AccessRevoked,
        ] {
            let mut input = fixture();
            input.producer.as_mut().unwrap().reason = Some(reason);
            let output = row(&input, true).unwrap();
            assert!(output.quote.is_none() && output.session.is_none());
            assert_eq!(output.availability, "UNAVAILABLE");
        }
        let mut input = fixture();
        input.cache.as_mut().unwrap().committed_at = None;
        assert!(row(&input, true).is_err());
        let mut input = fixture();
        input
            .cache
            .as_mut()
            .unwrap()
            .quote
            .as_mut()
            .unwrap()
            .base_price = Some("1".into());
        assert!(row(&input, true).is_err());
        let mut input = fixture();
        input.producer.as_mut().unwrap().gap_generation = u64::MAX;
        assert!(row(&input, true).is_err());
    }

    #[test]
    fn snapshots_reject_duplicate_membership_or_instrument_and_sort_membership() {
        let first = fixture();
        let mut second = first.clone();
        second.identity.membership_id = Uuid::from_u128(20);
        let mut snapshot = StreamSnapshot {
            lease_id: Uuid::from_u128(30),
            lease_expires_at: first.read_evidence.as_ref().unwrap().observed_at
                + Duration::seconds(30),
            rows: vec![],
            delivery_rows: vec![first.clone(), first.clone()],
        };
        assert!(rows(&snapshot, true).is_err());
        snapshot.delivery_rows = vec![second.clone(), first.clone()];
        assert!(
            rows(&snapshot, true).is_err(),
            "duplicate instrument is forbidden"
        );
        second.identity.instrument_id = "000660.KRX".into();
        snapshot.delivery_rows = vec![second, first];
        let result = rows(&snapshot, true).unwrap();
        assert!(result[0].membership_id < result[1].membership_id);
        snapshot.delivery_rows.clear();
        assert!(rows(&snapshot, true).is_err());
    }
}
