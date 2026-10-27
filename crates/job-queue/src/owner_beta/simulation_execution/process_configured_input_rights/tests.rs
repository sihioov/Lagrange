// The fake transport keeps explicit failure controls and typed mutation tables.
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_noop_waker
)]

use super::*;

use chrono::Duration;
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

const TEST_REFERENCE: &str = "licensed-ref-한글";
const TEST_LICENSE_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct InputFixture {
    summary: SummaryFields,
    pins: PinFields,
    dates: Vec<TradingDate>,
    bars: Vec<PrimitiveInputBar>,
}

impl InputFixture {
    fn new() -> Self {
        let instruments = (0..EXPECTED_INSTRUMENTS)
            .map(|index| format!("ETF{:02}.KRX", index + 1))
            .collect::<Vec<_>>();
        let start = NaiveDate::from_ymd_opt(2016, 8, 29).expect("synthetic start date");
        let end = NaiveDate::from_ymd_opt(2026, 8, 28).expect("synthetic end date");
        let span = (end - start).num_days();
        let dates = (0..EXPECTED_SESSIONS)
            .map(|index| {
                let offset = (index as i64 * span) / (EXPECTED_SESSIONS as i64 - 1);
                let date = start + Duration::days(offset);
                TradingDate::parse(&date.format("%Y-%m-%d").to_string())
                    .expect("synthetic date is valid")
            })
            .collect::<Vec<_>>();
        let mut bars = Vec::with_capacity(EXPECTED_BARS);
        for date in &dates {
            for instrument in &instruments {
                bars.push(PrimitiveInputBar {
                    instrument: instrument.clone(),
                    session_date: *date,
                });
            }
        }

        let summary = SummaryFields {
            artifact_manifest_sha256: "sha256:artifact-manifest-sentinel".to_owned(),
            candidate_content_sha256: "sha256:candidate-sentinel".to_owned(),
            price_manifest_line_sha256: "sha256:price-line-sentinel".to_owned(),
            action_manifest_line_sha256: "sha256:action-line-sentinel".to_owned(),
            range_start: dates[0],
            range_end: *dates.last().expect("date list is nonempty"),
            instruments,
            instrument_count: EXPECTED_INSTRUMENTS,
            session_count: EXPECTED_SESSIONS,
            row_count: EXPECTED_BARS,
            bars_sha256: "sha256:artifact-bars-sentinel".to_owned(),
            bars_row_count: EXPECTED_BARS,
        };
        let pins = PinFields {
            candidate_content_sha256: summary.candidate_content_sha256.clone(),
            artifact_manifest_sha256: summary.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: "sha256:price-line-sentinel".to_owned(),
            action_manifest_sha256: "sha256:action-line-sentinel".to_owned(),
            approval_registry_sha256: "sha256:registry-sentinel".to_owned(),
        };
        Self {
            summary,
            pins,
            dates,
            bars,
        }
    }

    fn view(&self) -> ApprovedInputView<'_> {
        ApprovedInputView {
            summary: Some(self.summary.clone()),
            pins: self.pins.clone(),
            bars: InputBars::Primitive(&self.bars),
        }
    }
}

fn test_config() -> ConfigView {
    ConfigView {
        config_sha256: "config-secret-sentinel".to_owned(),
        owner: Uuid::from_u128(0x1234),
        entitlement_reference: TEST_REFERENCE.to_owned(),
        entitlement_sha256: format!("sha256:{TEST_LICENSE_HASH}"),
        startup_timeout_seconds: 45,
    }
}

fn ready<T>(result: Result<T, SimulationConfiguredInputRightsError>) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("expected successful private validation"),
    }
}

fn expect_primary<T>(
    result: Result<T, SimulationConfiguredInputRightsError>,
    primary: PrimaryFailure,
) -> SimulationConfiguredInputRightsError {
    match result {
        Ok(_) => panic!("expected a fail-closed result"),
        Err(error) => {
            assert_eq!(error.primary, primary);
            error
        }
    }
}

fn test_date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("test date is valid")
}

fn add_day(date: TradingDate, days: i64) -> TradingDate {
    let date = date.as_naive_date() + Duration::days(days);
    TradingDate::parse(&date.format("%Y-%m-%d").to_string()).expect("test date is valid")
}

fn validated_test_config() -> ValidatedConfig {
    ready(validate_config_view(test_config()))
}

fn raw_bytes(value: &str) -> RawCell {
    let bytes = value.as_bytes().to_vec();
    RawCell::Bytes {
        is_null: false,
        original_bytes: Some(bytes.len() as i64),
        prefix: bytes,
    }
}

fn raw_bytes_value(value: &[u8]) -> RawCell {
    RawCell::Bytes {
        is_null: false,
        original_bytes: Some(value.len() as i64),
        prefix: value.to_vec(),
    }
}

fn raw_null_bytes() -> RawCell {
    RawCell::Bytes {
        is_null: true,
        original_bytes: None,
        prefix: Vec::new(),
    }
}

fn setup_row(search_path: &str, statement_timeout: &str, lock_timeout: &str) -> RawRow {
    RawRow::new(
        RawCategory::Setup,
        vec![
            raw_bytes(search_path),
            raw_bytes(statement_timeout),
            raw_bytes(lock_timeout),
        ],
    )
}

fn context_evidence() -> ContextEvidence {
    ContextEvidence {
        current_user: "worker".to_owned(),
        session_user: "worker".to_owned(),
        database_name: "lagrange".to_owned(),
        database_oid: 16_384,
        worker_oid: 16_385,
        backend_pid: 2_468,
        row_security_on: true,
        replication_origin: true,
        transaction_read_only: true,
        repeatable_read: true,
        search_path_is_catalog: true,
        statement_timeout_ms: 30_000,
        lock_timeout_ms: 30_000,
        statement_unit_ms: true,
        lock_unit_ms: true,
    }
}

fn context_row(evidence: &ContextEvidence, category: RawCategory) -> RawRow {
    RawRow::new(
        category,
        vec![
            raw_bytes(&evidence.current_user),
            raw_bytes(&evidence.session_user),
            raw_bytes(&evidence.database_name),
            RawCell::I64(i64::from(evidence.database_oid)),
            RawCell::I64(i64::from(evidence.worker_oid)),
            RawCell::I32(evidence.backend_pid),
            RawCell::Bool(evidence.row_security_on),
            RawCell::Bool(evidence.replication_origin),
            RawCell::Bool(evidence.transaction_read_only),
            RawCell::Bool(evidence.repeatable_read),
            RawCell::Bool(evidence.search_path_is_catalog),
            RawCell::I64(evidence.statement_timeout_ms),
            RawCell::I64(evidence.lock_timeout_ms),
            RawCell::Bool(evidence.statement_unit_ms),
            RawCell::Bool(evidence.lock_unit_ms),
        ],
    )
}

fn owner_row(owner: Uuid, role_observed: bool) -> RawRow {
    RawRow::new(
        RawCategory::Owner,
        vec![RawCell::Uuid(owner), RawCell::Bool(role_observed)],
    )
}

fn entitlement_row(
    id: Uuid,
    reference: &str,
    license_hash: &str,
    status: &str,
    datasets_json: &str,
    uses_json: &str,
    effective_from: NaiveDate,
    effective_until: NaiveDate,
    covers_any_required: bool,
) -> RawRow {
    RawRow::new(
        RawCategory::Entitlements,
        vec![
            RawCell::Uuid(id),
            raw_bytes(reference),
            raw_bytes(license_hash),
            raw_bytes(status),
            raw_bytes(datasets_json),
            raw_bytes(uses_json),
            RawCell::Date(effective_from),
            RawCell::Date(effective_until),
            RawCell::Bool(covers_any_required),
        ],
    )
}

fn active_entitlement(
    id: u128,
    from: NaiveDate,
    until: NaiveDate,
    datasets_json: &str,
    uses_json: &str,
    covers_any_required: bool,
) -> RawRow {
    entitlement_row(
        Uuid::from_u128(id),
        TEST_REFERENCE,
        TEST_LICENSE_HASH,
        "ACTIVE",
        datasets_json,
        uses_json,
        from,
        until,
        covers_any_required,
    )
}

fn valid_entitlements_for(dates: &[TradingDate]) -> Vec<RawRow> {
    vec![active_entitlement(
        1,
        dates[0].as_naive_date(),
        dates[dates.len() - 1].as_naive_date(),
        "[\"krx_eod_bars\"]",
        "[\"backtest\"]",
        true,
    )]
}

fn validate_entitlement_rows(
    rows: Vec<RawRow>,
    dates: &[TradingDate],
    canceled: &AtomicBool,
) -> Result<Vec<Uuid>, SimulationConfiguredInputRightsError> {
    validate_entitlements(
        rows,
        &validated_test_config(),
        dates,
        canceled,
        &mut LogicalPayloadBudget::default(),
    )
}

#[test]
fn input_requires_v3_envelope_and_all_four_summary_pins() {
    let fixture = InputFixture::new();
    let missing = ApprovedInputView {
        summary: None,
        pins: fixture.pins.clone(),
        bars: InputBars::Primitive(&fixture.bars),
    };
    expect_primary(
        validate_input_view(missing, &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    for field in 0..4 {
        let mut changed = InputFixture::new();
        match field {
            0 => changed.summary.candidate_content_sha256.push('x'),
            1 => changed.summary.artifact_manifest_sha256.push('x'),
            2 => changed.summary.price_manifest_line_sha256.push('x'),
            _ => changed.summary.action_manifest_line_sha256.push('x'),
        }
        expect_primary(
            validate_input_view(changed.view(), &AtomicBool::new(false)),
            PrimaryFailure::InputInvalid,
        );
    }

    for field in 0..4 {
        let mut changed = InputFixture::new();
        match field {
            0 => changed.pins.candidate_content_sha256.push('x'),
            1 => changed.pins.artifact_manifest_sha256.push('x'),
            2 => changed.pins.stage5_manifest_sha256.push('x'),
            _ => changed.pins.action_manifest_sha256.push('x'),
        }
        expect_primary(
            validate_input_view(changed.view(), &AtomicBool::new(false)),
            PrimaryFailure::InputInvalid,
        );
    }

    let prepared = ready(validate_input_view(fixture.view(), &AtomicBool::new(false)));
    assert_eq!(
        prepared.artifact_bars_sha256,
        "sha256:artifact-bars-sentinel"
    );
    assert_eq!(
        prepared.pins.approval_registry_sha256,
        "sha256:registry-sentinel"
    );
    assert_eq!(prepared.dates, fixture.dates);
    assert_eq!(prepared.instrument_count, EXPECTED_INSTRUMENTS);
    assert_eq!(prepared.bar_count, EXPECTED_BARS);
}

#[test]
fn input_rejects_count_range_instrument_and_pair_mismatches() {
    let fixture = InputFixture::new();

    for mutation in 0..5 {
        let mut changed = InputFixture::new();
        match mutation {
            0 => changed.summary.row_count -= 1,
            1 => changed.summary.bars_row_count -= 1,
            2 => changed.summary.session_count -= 1,
            3 => changed.summary.instrument_count -= 1,
            _ => changed.summary.range_end = add_day(changed.summary.range_end, -1),
        }
        expect_primary(
            validate_input_view(changed.view(), &AtomicBool::new(false)),
            PrimaryFailure::InputInvalid,
        );
    }

    let mut wrong_start = InputFixture::new();
    wrong_start.summary.range_start = add_day(wrong_start.summary.range_start, 1);
    expect_primary(
        validate_input_view(wrong_start.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut empty_instrument = InputFixture::new();
    empty_instrument.summary.instruments[0].clear();
    expect_primary(
        validate_input_view(empty_instrument.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut long_instrument = InputFixture::new();
    long_instrument.summary.instruments[0] = "x".repeat(MAX_IDENTIFIER_BYTES + 1);
    expect_primary(
        validate_input_view(long_instrument.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut duplicate_instrument = InputFixture::new();
    duplicate_instrument.summary.instruments[1] =
        duplicate_instrument.summary.instruments[0].clone();
    expect_primary(
        validate_input_view(duplicate_instrument.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut empty_bar = InputFixture::new();
    empty_bar.bars[0].instrument.clear();
    expect_primary(
        validate_input_view(empty_bar.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut foreign_bar = InputFixture::new();
    foreign_bar.bars[0].instrument = "FOREIGN.KRX".to_owned();
    expect_primary(
        validate_input_view(foreign_bar.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut duplicate_pair = InputFixture::new();
    let first = duplicate_pair.bars[0].clone();
    let last = duplicate_pair.bars.len() - 1;
    duplicate_pair.bars[last] = first;
    expect_primary(
        validate_input_view(duplicate_pair.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut outside_range = InputFixture::new();
    outside_range.bars[0].session_date = add_day(outside_range.summary.range_start, -1);
    expect_primary(
        validate_input_view(outside_range.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut too_many_bars = InputFixture::new();
    too_many_bars.bars.push(too_many_bars.bars[0].clone());
    expect_primary(
        validate_input_view(too_many_bars.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    let mut too_few_bars = InputFixture::new();
    too_few_bars.bars.pop();
    expect_primary(
        validate_input_view(too_few_bars.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );

    assert_eq!(fixture.bars.len(), EXPECTED_BARS);
}

#[test]
fn input_rejects_more_than_2452_dates_even_with_fixed_bar_count() {
    let mut fixture = InputFixture::new();
    let used = fixture.dates.iter().copied().collect::<BTreeSet<_>>();
    let start = fixture.dates[0].as_naive_date();
    let candidate = (1..=5_000_i64)
        .map(|days| start + Duration::days(days))
        .find(|date| {
            let date = TradingDate::parse(&date.format("%Y-%m-%d").to_string()).expect("date");
            !used.contains(&date) && date < fixture.summary.range_end
        })
        .expect("synthetic range includes unselected dates");
    fixture.bars[0].session_date = TradingDate::parse(&candidate.format("%Y-%m-%d").to_string())
        .expect("candidate date is valid");
    expect_primary(
        validate_input_view(fixture.view(), &AtomicBool::new(false)),
        PrimaryFailure::InputInvalid,
    );
}

#[test]
fn actual_date_binding_uses_sorted_bar_dates_and_changes_with_the_input() {
    let mut first = InputFixture::new();
    let first_prepared = ready(validate_input_view(first.view(), &AtomicBool::new(false)));
    let first_dates = first_prepared
        .dates
        .iter()
        .map(TradingDate::as_naive_date)
        .collect::<Vec<_>>();
    let first_plan = statement_plan(Statement::Entitlements {
        reference: TEST_REFERENCE.to_owned(),
        bare_hash: TEST_LICENSE_HASH.to_owned(),
        dates: first_dates.clone(),
    });
    assert!(statement_plan_is_valid(&first_plan));
    match first_plan.binds.get(2) {
        Some(BindValue::DateArray(bound)) => assert_eq!(bound, &first_dates),
        _ => panic!("date input must use the typed DATE[] bind"),
    }
    assert_eq!(first_dates.len() * 4, DATE_INPUT_LOGICAL_BYTES);

    let original_middle = first.dates[1];
    let mut replacement = None;
    let mut day = original_middle.as_naive_date() + Duration::days(1);
    let used = first.dates.iter().copied().collect::<BTreeSet<_>>();
    while day < first.summary.range_end.as_naive_date() {
        let trading_date = TradingDate::parse(&day.format("%Y-%m-%d").to_string()).expect("date");
        if !used.contains(&trading_date) {
            replacement = Some(trading_date);
            break;
        }
        day += Duration::days(1);
    }
    let replacement = replacement.expect("synthetic range contains an unselected date");
    for bar in &mut first.bars {
        if bar.session_date == original_middle {
            bar.session_date = replacement;
        }
    }
    let second_prepared = ready(validate_input_view(first.view(), &AtomicBool::new(false)));
    let second_dates = second_prepared
        .dates
        .iter()
        .map(TradingDate::as_naive_date)
        .collect::<Vec<_>>();
    assert_ne!(first_dates, second_dates);
}

#[test]
fn config_hash_prefix_is_exact_and_all_zero_is_syntactically_allowed() {
    let zero = format!("sha256:{}", "0".repeat(64));
    let zero_bare = "0".repeat(64);
    assert_eq!(bare_license_hash(&zero), Some(zero_bare.as_str()));
    let mut zero_config = test_config();
    zero_config.entitlement_sha256 = zero.clone();
    assert_eq!(
        ready(validate_config_view(zero_config)).bare_license_hash,
        zero_bare
    );
    assert_eq!(
        bare_license_hash(&format!("sha256:{TEST_LICENSE_HASH}")),
        Some(TEST_LICENSE_HASH)
    );
    for invalid in [
        TEST_LICENSE_HASH.to_owned(),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{} ", TEST_LICENSE_HASH),
        format!("sha256:sha256:{TEST_LICENSE_HASH}"),
        "sha256:short".to_owned(),
    ] {
        assert!(bare_license_hash(&invalid).is_none());
    }

    let config = ready(validate_config_view(test_config()));
    assert_eq!(config.bare_license_hash, TEST_LICENSE_HASH);
    assert_eq!(config.entitlement_reference, TEST_REFERENCE);
}

#[test]
fn config_rejects_bad_reference_and_checked_timeout_arithmetic_caps_per_statement_value() {
    for reference in ["", " leading", "trailing ", "nul\0value"] {
        let mut config = test_config();
        config.entitlement_reference = reference.to_owned();
        expect_primary(validate_config_view(config), PrimaryFailure::InvalidBound);
    }
    assert_eq!(timeout_millis(1).ok(), Some(1_000));
    assert_eq!(timeout_millis(45).ok(), Some(30_000));
    assert_eq!(timeout_millis(i32::MAX as u64).ok(), Some(30_000));
    expect_primary(timeout_millis(u64::MAX), PrimaryFailure::InvalidBound);
}

#[test]
fn config_reference_preserves_unicode_and_enforces_exact_trimmed_value() {
    assert!(valid_reference("権利参照-한글-العربية"));
    assert!(valid_reference(&"가".repeat(512)));
    assert!(!valid_reference(&"가".repeat(513)));
    assert!(!valid_reference("ref\u{0085}control"));

    let mut changed = test_config();
    changed.entitlement_reference = format!("{} ", TEST_REFERENCE);
    expect_primary(validate_config_view(changed), PrimaryFailure::InvalidBound);
}

#[test]
fn setup_projection_requires_nonempty_values_and_context_checks_timeout_values() {
    let mut budget = LogicalPayloadBudget::default();
    ready(validate_setup(
        vec![setup_row("pg_catalog", "30000ms", "30000ms")],
        &mut budget,
    ));

    let mut budget = LogicalPayloadBudget::default();
    ready(validate_setup(
        vec![setup_row("pg_catalog", "10 seconds", "30000 ms")],
        &mut budget,
    ));

    let mut budget = LogicalPayloadBudget::default();
    expect_primary(
        validate_setup(vec![setup_row("pg_catalog", "", "30000ms")], &mut budget),
        PrimaryFailure::SetupResultInvalid,
    );
}

#[test]
fn owner_requires_the_exact_configured_user_and_owner_role() {
    let owner = Uuid::from_u128(42);
    for (rows, expected) in [
        (Vec::new(), owner),
        (vec![owner_row(Uuid::from_u128(43), true)], owner),
        (vec![owner_row(owner, false)], owner),
    ] {
        expect_primary(
            validate_owner(rows, expected, &mut LogicalPayloadBudget::default()),
            PrimaryFailure::PolicyRejected,
        );
    }
    ready(validate_owner(
        vec![owner_row(owner, true)],
        owner,
        &mut LogicalPayloadBudget::default(),
    ));
    for rows in [
        vec![owner_row(owner, true), owner_row(owner, true)],
        vec![
            owner_row(owner, true),
            owner_row(owner, true),
            owner_row(owner, true),
        ],
    ] {
        expect_primary(
            validate_owner(rows, owner, &mut LogicalPayloadBudget::default()),
            PrimaryFailure::BoundExceeded,
        );
    }
}

#[test]
fn context_requires_worker_identity_exact_settings_and_stable_snapshot() {
    let expected = context_evidence();
    ready(validate_context(
        vec![context_row(&expected, RawCategory::InitialContext)],
        RawCategory::InitialContext,
        30_000,
        &mut LogicalPayloadBudget::default(),
    ));

    expect_primary(
        validate_context(
            Vec::new(),
            RawCategory::InitialContext,
            30_000,
            &mut LogicalPayloadBudget::default(),
        ),
        PrimaryFailure::PolicyRejected,
    );
    expect_primary(
        validate_context(
            vec![
                context_row(&expected, RawCategory::InitialContext),
                context_row(&expected, RawCategory::InitialContext),
            ],
            RawCategory::InitialContext,
            30_000,
            &mut LogicalPayloadBudget::default(),
        ),
        PrimaryFailure::BoundExceeded,
    );

    let mut mutations: Vec<Box<dyn Fn(&mut ContextEvidence)>> = vec![
        Box::new(|row| row.current_user = "owner".to_owned()),
        Box::new(|row| row.session_user = "other".to_owned()),
        Box::new(|row| row.database_name.clear()),
        Box::new(|row| row.database_oid = 0),
        Box::new(|row| row.worker_oid = 0),
        Box::new(|row| row.backend_pid = -1),
        Box::new(|row| row.row_security_on = false),
        Box::new(|row| row.replication_origin = false),
        Box::new(|row| row.transaction_read_only = false),
        Box::new(|row| row.repeatable_read = false),
        Box::new(|row| row.search_path_is_catalog = false),
        Box::new(|row| row.statement_timeout_ms += 1),
        Box::new(|row| row.lock_timeout_ms += 1),
        Box::new(|row| row.statement_unit_ms = false),
        Box::new(|row| row.lock_unit_ms = false),
    ];
    for mutate in mutations.drain(..) {
        let mut changed = expected.clone();
        mutate(&mut changed);
        expect_primary(
            validate_context(
                vec![context_row(&changed, RawCategory::InitialContext)],
                RawCategory::InitialContext,
                30_000,
                &mut LogicalPayloadBudget::default(),
            ),
            PrimaryFailure::PolicyRejected,
        );
    }

    let mut drifted = expected.clone();
    drifted.database_oid += 1;
    expect_primary(
        compare_contexts(&expected, &drifted),
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn entitlement_row_limits_are_zero_to_sixty_four_with_a_sixty_five_row_sentinel() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let make_rows = |count: usize| {
        (1..=count)
            .map(|id| {
                active_entitlement(
                    id as u128,
                    first,
                    last,
                    "[\"krx_eod_bars\"]",
                    "[\"backtest\"]",
                    true,
                )
            })
            .collect::<Vec<_>>()
    };

    expect_primary(
        validate_entitlement_rows(Vec::new(), &dates, &AtomicBool::new(false)),
        PrimaryFailure::PolicyRejected,
    );
    ready(validate_entitlement_rows(
        make_rows(1),
        &dates,
        &AtomicBool::new(false),
    ));
    ready(validate_entitlement_rows(
        make_rows(64),
        &dates,
        &AtomicBool::new(false),
    ));
    expect_primary(
        validate_entitlement_rows(make_rows(65), &dates, &AtomicBool::new(false)),
        PrimaryFailure::BoundExceeded,
    );
    expect_primary(
        validate_entitlement_rows(make_rows(66), &dates, &AtomicBool::new(false)),
        PrimaryFailure::BoundExceeded,
    );
}

#[test]
fn entitlement_ids_must_be_strictly_ordered_and_unique() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let lower = active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    let higher = active_entitlement(2, first, last, "[]", "[]", true);

    ready(validate_entitlement_rows(
        vec![lower.clone(), higher.clone()],
        &dates,
        &AtomicBool::new(false),
    ));
    let mut duplicate_content = lower.clone();
    duplicate_content.cells[0] = RawCell::Uuid(Uuid::from_u128(3));
    ready(validate_entitlement_rows(
        vec![lower.clone(), duplicate_content],
        &dates,
        &AtomicBool::new(false),
    ));
    for rows in [vec![higher.clone(), lower], vec![higher.clone(), higher]] {
        expect_primary(
            validate_entitlement_rows(rows, &dates, &AtomicBool::new(false)),
            PrimaryFailure::PolicyRejected,
        );
    }
}

#[test]
fn known_inactive_statuses_are_decoded_but_unknown_status_is_rejected() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    for status in ["PENDING", "EXPIRED", "REVOKED"] {
        let inactive = entitlement_row(
            Uuid::from_u128(1),
            TEST_REFERENCE,
            TEST_LICENSE_HASH,
            status,
            "[]",
            "[]",
            first,
            last,
            true,
        );
        let mut rows = vec![
            inactive,
            active_entitlement(2, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true),
        ];
        rows.sort_by_key(|row| match row.cells.first() {
            Some(RawCell::Uuid(id)) => *id,
            _ => Uuid::nil(),
        });
        ready(validate_entitlement_rows(
            rows,
            &dates,
            &AtomicBool::new(false),
        ));
    }

    let unknown = entitlement_row(
        Uuid::from_u128(1),
        TEST_REFERENCE,
        TEST_LICENSE_HASH,
        "UNKNOWN",
        "[]",
        "[]",
        first,
        last,
        true,
    );
    expect_primary(
        validate_entitlement_rows(
            vec![
                unknown,
                active_entitlement(2, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true),
            ],
            &dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn entitlement_identity_is_exact_and_no_different_license_row_can_cover() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let mut wrong_reference =
        active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    if let Some(RawCell::Bytes {
        prefix,
        original_bytes,
        ..
    }) = wrong_reference.cells.get_mut(1)
    {
        *prefix = b"licensed-ref-\xe6\xbc\xa2\xe8\xaa\x9e".to_vec();
        *original_bytes = Some(prefix.len() as i64);
    }
    let active = active_entitlement(2, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    expect_primary(
        validate_entitlement_rows(
            vec![wrong_reference, active.clone()],
            &dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::PolicyRejected,
    );

    let mut wrong_hash = active_entitlement(1, first, last, "[]", "[]", true);
    if let Some(RawCell::Bytes {
        prefix,
        original_bytes,
        ..
    }) = wrong_hash.cells.get_mut(2)
    {
        *prefix = b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_vec();
        *original_bytes = Some(prefix.len() as i64);
    }
    expect_primary(
        validate_entitlement_rows(vec![wrong_hash, active], &dates, &AtomicBool::new(false)),
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn exact_license_windows_allow_partitions_and_overlaps_but_reject_gaps() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let midpoint = dates[dates.len() / 2].as_naive_date();
    let next_midpoint = dates[dates.len() / 2 + 1].as_naive_date();
    let eligibility = ("[\"krx_eod_bars\"]", "[\"backtest\"]");

    ready(validate_entitlement_rows(
        vec![
            active_entitlement(1, first, midpoint, eligibility.0, eligibility.1, true),
            active_entitlement(2, next_midpoint, last, eligibility.0, eligibility.1, true),
        ],
        &dates,
        &AtomicBool::new(false),
    ));
    ready(validate_entitlement_rows(
        vec![
            active_entitlement(1, first, last, eligibility.0, eligibility.1, true),
            active_entitlement(
                2,
                midpoint,
                next_midpoint,
                eligibility.0,
                eligibility.1,
                true,
            ),
        ],
        &dates,
        &AtomicBool::new(false),
    ));
    expect_primary(
        validate_entitlement_rows(
            vec![
                active_entitlement(
                    1,
                    first,
                    dates[dates.len() / 2 - 1].as_naive_date(),
                    eligibility.0,
                    eligibility.1,
                    true,
                ),
                active_entitlement(2, next_midpoint, last, eligibility.0, eligibility.1, true),
            ],
            &dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::PolicyRejected,
    );

    expect_primary(
        validate_entitlement_rows(
            vec![
                active_entitlement(1, first, first, eligibility.0, eligibility.1, true),
                active_entitlement(2, last, last, eligibility.0, eligibility.1, true),
            ],
            &dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::PolicyRejected,
    );
}

#[test]
fn covers_any_is_recomputed_for_active_and_inactive_rows() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let wrong_flag = active_entitlement(
        1,
        first,
        last,
        "[\"krx_eod_bars\"]",
        "[\"backtest\"]",
        false,
    );
    expect_primary(
        validate_entitlement_rows(vec![wrong_flag], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );

    let inactive = entitlement_row(
        Uuid::from_u128(1),
        TEST_REFERENCE,
        TEST_LICENSE_HASH,
        "REVOKED",
        "[]",
        "[]",
        first,
        last,
        false,
    );
    let active = active_entitlement(2, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    expect_primary(
        validate_entitlement_rows(vec![inactive, active], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );
}

#[test]
fn identifier_json_is_bounded_distinct_control_free_and_exactly_typed() {
    for invalid_json in [
        "{}",
        "null",
        "\"one\"",
        "[1]",
        "[\"\"]",
        "[\"a\",\"a\"]",
        "[\"x\\u0000y\"]",
    ] {
        assert!(
            decode_identifiers(invalid_json.to_owned()).is_err(),
            "accepted {invalid_json}"
        );
    }
    let many = serde_json::to_string(
        &(0..=MAX_IDENTIFIER_COUNT)
            .map(|n| format!("id-{n}"))
            .collect::<Vec<_>>(),
    )
    .expect("test JSON serializes");
    expect_primary(decode_identifiers(many), PrimaryFailure::BoundExceeded);
    assert!(decode_identifiers("[]".to_owned()).is_ok());
    assert!(decode_identifiers("[\"未知-dataset\"]".to_owned()).is_ok());

    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let overlong_identifier = serde_json::to_string(&vec!["x".repeat(MAX_IDENTIFIER_BYTES + 1)])
        .expect("test JSON serializes");
    expect_primary(
        validate_entitlement_rows(
            vec![active_entitlement(
                1,
                first,
                last,
                &overlong_identifier,
                "[\"backtest\"]",
                true,
            )],
            &dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::PolicyRejected,
    );

    let mut oversized_json =
        active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    oversized_json.cells[4] = raw_bytes_value(&vec![b' '; JSON_FETCH_BYTES]);
    expect_primary(
        validate_entitlement_rows(vec![oversized_json], &dates, &AtomicBool::new(false)),
        PrimaryFailure::BoundExceeded,
    );

    let mut null_json =
        active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    null_json.cells[4] = raw_null_bytes();
    expect_primary(
        validate_entitlement_rows(vec![null_json], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );
}

#[test]
fn entitlement_dates_must_be_present_ordered_and_raw_utf8_must_be_complete() {
    let dates = InputFixture::new().dates;
    let first = dates[0].as_naive_date();
    let last = dates[dates.len() - 1].as_naive_date();
    let mut reversed =
        active_entitlement(1, last, first, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    expect_primary(
        validate_entitlement_rows(vec![reversed.clone()], &dates, &AtomicBool::new(false)),
        PrimaryFailure::PolicyRejected,
    );

    reversed.cells[6] = RawCell::Null;
    expect_primary(
        validate_entitlement_rows(vec![reversed], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );

    let mut malformed =
        active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    malformed.cells[4] = raw_bytes_value(&[0xff, 0xfe]);
    expect_primary(
        validate_entitlement_rows(vec![malformed], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );

    let mut truncated =
        active_entitlement(1, first, last, "[\"krx_eod_bars\"]", "[\"backtest\"]", true);
    let mut partial_codepoint = vec![b'a'; JSON_ACCEPT_BYTES - 1];
    partial_codepoint.push(0xe2);
    truncated.cells[4] = RawCell::Bytes {
        is_null: false,
        original_bytes: Some(JSON_ACCEPT_BYTES as i64),
        prefix: partial_codepoint,
    };
    expect_primary(
        validate_entitlement_rows(vec![truncated], &dates, &AtomicBool::new(false)),
        PrimaryFailure::RawInvalid,
    );
}

#[test]
fn raw_decoder_rejects_null_contradictions_lying_lengths_caps_and_wrong_types() {
    let decode = |cell: RawCell| {
        let mut decoder = RawDecoder::new(
            RawRow::new(
                RawCategory::Setup,
                vec![cell, raw_bytes("30000ms"), raw_bytes("30000ms")],
            ),
            RawCategory::Setup,
            SETUP_ROW,
        )?;
        decoder.bytes(SHORT_TEXT_ACCEPT_BYTES, SHORT_TEXT_FETCH_BYTES)
    };

    assert_eq!(ready(decode(raw_bytes("pg_catalog"))), "pg_catalog");
    for malformed in [
        raw_null_bytes(),
        RawCell::Bytes {
            is_null: true,
            original_bytes: Some(1),
            prefix: vec![b'x'],
        },
        RawCell::Bytes {
            is_null: false,
            original_bytes: None,
            prefix: Vec::new(),
        },
        RawCell::Bytes {
            is_null: false,
            original_bytes: Some(-1),
            prefix: Vec::new(),
        },
        RawCell::Bytes {
            is_null: false,
            original_bytes: Some(3),
            prefix: vec![b'x', b'y'],
        },
        RawCell::WrongType,
        RawCell::Null,
    ] {
        expect_primary(decode(malformed), PrimaryFailure::RawInvalid);
    }
    let clipped = RawCell::Bytes {
        is_null: false,
        original_bytes: Some((SHORT_TEXT_FETCH_BYTES + 1) as i64),
        prefix: vec![b'x'; SHORT_TEXT_FETCH_BYTES],
    };
    expect_primary(decode(clipped), PrimaryFailure::BoundExceeded);
}

#[test]
fn raw_payload_is_charged_from_materialized_bytes_before_row_semantics() {
    let mut budget = LogicalPayloadBudget::default();
    let huge = RawCell::Bytes {
        is_null: true,
        original_bytes: Some(i64::MAX),
        prefix: vec![b'x'; MAX_LOGICAL_PAYLOAD_BYTES],
    };
    let rejected = RawRow::new(RawCategory::Owner, vec![huge]);
    expect_primary(
        prepare_raw_rows(
            vec![rejected],
            RawCategory::Setup,
            SETUP_ROW,
            1,
            SETUP_ACCEPT_ROWS,
            SETUP_FETCH_ROWS,
            PrimaryFailure::SetupResultInvalid,
            &mut budget,
        ),
        PrimaryFailure::PayloadExceeded,
    );
    assert_eq!(budget.used, 0);

    let mut budget = LogicalPayloadBudget::default();
    budget
        .charge_cell(&RawCell::Null)
        .expect("null scalar is one byte");
    budget
        .charge_cell(&RawCell::WrongType)
        .expect("wrong type is one byte");
    budget
        .charge_cell(&RawCell::Bytes {
            is_null: false,
            original_bytes: Some(i64::MAX),
            prefix: vec![b'a'; 3],
        })
        .expect("claimed length cannot inflate or reduce actual charge");
    assert_eq!(budget.used, 1 + 1 + 3 + RAW_TRIPLET_OVERHEAD_BYTES);
}

#[test]
fn raw_fake_shape_category_and_row_sentinel_checks_share_the_production_decoder() {
    let mut budget = LogicalPayloadBudget::default();
    ready(prepare_raw_rows(
        vec![setup_row("pg_catalog", "30000ms", "30000ms")],
        RawCategory::Setup,
        SETUP_ROW,
        1,
        SETUP_ACCEPT_ROWS,
        SETUP_FETCH_ROWS,
        PrimaryFailure::SetupResultInvalid,
        &mut budget,
    ));

    for (rows, expected) in [
        (Vec::new(), PrimaryFailure::SetupResultInvalid),
        (
            vec![
                setup_row("pg_catalog", "30000ms", "30000ms"),
                setup_row("pg_catalog", "30000ms", "30000ms"),
            ],
            PrimaryFailure::BoundExceeded,
        ),
        (
            vec![RawRow::new(
                RawCategory::Owner,
                vec![RawCell::Uuid(Uuid::nil())],
            )],
            PrimaryFailure::RawInvalid,
        ),
    ] {
        expect_primary(
            prepare_raw_rows(
                rows,
                RawCategory::Setup,
                SETUP_ROW,
                1,
                SETUP_ACCEPT_ROWS,
                SETUP_FETCH_ROWS,
                PrimaryFailure::SetupResultInvalid,
                &mut LogicalPayloadBudget::default(),
            ),
            expected,
        );
    }

    let mut wrong_type = setup_row("pg_catalog", "30000ms", "30000ms");
    wrong_type.cells[1] = RawCell::WrongType;
    expect_primary(
        validate_setup(vec![wrong_type], &mut LogicalPayloadBudget::default()),
        PrimaryFailure::RawInvalid,
    );

    let mut wrong_date = active_entitlement(
        1,
        test_date("2016-08-29"),
        test_date("2026-08-28"),
        "[\"krx_eod_bars\"]",
        "[\"backtest\"]",
        true,
    );
    wrong_date.cells[7] = RawCell::WrongType;
    expect_primary(
        validate_entitlement_rows(
            vec![wrong_date],
            &InputFixture::new().dates,
            &AtomicBool::new(false),
        ),
        PrimaryFailure::RawInvalid,
    );
}

#[test]
fn fetched_and_date_logical_payload_maxima_remain_inside_frozen_allowances() {
    let fetched = maximum_fetched_payloads().expect("static bounds do not overflow");
    assert_eq!(
        maximum_shape_payload(SETUP_ROW, SETUP_FETCH_ROWS),
        Some(828)
    );
    assert_eq!(
        maximum_shape_payload(CONTEXT_ROW, CONTEXT_FETCH_ROWS),
        Some(914)
    );
    assert_eq!(maximum_shape_payload(OWNER_ROW, OWNER_FETCH_ROWS), Some(34));
    assert_eq!(
        maximum_shape_payload(ENTITLEMENTS_ROW, ENTITLEMENT_FETCH_ROWS),
        Some(808_210)
    );
    assert_eq!(
        maximum_shape_payload(ENTITLEMENTS_ROW, ENTITLEMENT_FETCH_ROWS),
        Some(LICENSE_MAX_FETCHED_PAYLOAD)
    );
    assert_eq!(fetched, ALL_MAX_FETCHED_PAYLOAD);
    assert_eq!(fetched, 810_900);
    assert_eq!(DATE_INPUT_LOGICAL_BYTES, 9_808);
    assert_eq!(fetched + DATE_INPUT_LOGICAL_BYTES, 820_708);
    assert_eq!(MAX_LOGICAL_PAYLOAD_BYTES, 1_048_576);
}

#[test]
fn all_statement_plans_pin_sql_category_projection_bounds_and_typed_binds() {
    let input = InputFixture::new();
    let prepared = ready(validate_input_view(input.view(), &AtomicBool::new(false)));
    let dates = prepared
        .dates
        .iter()
        .map(TradingDate::as_naive_date)
        .collect::<Vec<_>>();
    let owner = Uuid::from_u128(7);
    let plans = [
        statement_plan(Statement::Begin),
        statement_plan(Statement::Configure { timeout_ms: 30_000 }),
        statement_plan(Statement::InitialContext),
        statement_plan(Statement::Owner { owner }),
        statement_plan(Statement::Entitlements {
            reference: TEST_REFERENCE.to_owned(),
            bare_hash: TEST_LICENSE_HASH.to_owned(),
            dates: dates.clone(),
        }),
        statement_plan(Statement::FinalContext),
        statement_plan(Statement::Commit),
        statement_plan(Statement::Rollback),
    ];
    assert!(plans.iter().all(statement_plan_is_valid));
    for plan in [&plans[0], &plans[6], &plans[7]] {
        assert_eq!(plan.category, Some(RawCategory::Command));
        assert_eq!(plan.accept_rows, 0);
        assert_eq!(plan.fetch_rows, 1);
    }

    assert!(matches!(
        plans[1].binds.as_slice(),
        [BindValue::I64(30_000)]
    ));
    assert!(matches!(plans[3].binds.as_slice(), [BindValue::Uuid(actual)] if *actual == owner));
    match plans[4].binds.as_slice() {
        [
            BindValue::Text(reference),
            BindValue::Text(hash),
            BindValue::DateArray(bound),
        ] => {
            assert_eq!(reference.as_str(), TEST_REFERENCE);
            assert_eq!(hash.as_str(), TEST_LICENSE_HASH);
            assert_eq!(bound, &dates);
        }
        _ => panic!("entitlement query must bind exact TEXT/TEXT/DATE[] values"),
    }

    let mut wrong_category = statement_plan(Statement::Configure { timeout_ms: 30_000 });
    wrong_category.category = Some(RawCategory::Owner);
    assert!(!statement_plan_is_valid(&wrong_category));

    let mut wrong_sentinel = statement_plan(Statement::Owner { owner });
    wrong_sentinel.fetch_rows = OWNER_ACCEPT_ROWS;
    assert!(!statement_plan_is_valid(&wrong_sentinel));

    let mut wrong_bind = statement_plan(Statement::Owner { owner });
    wrong_bind.binds = vec![BindValue::Text("not-a-uuid".to_owned())];
    assert!(!statement_plan_is_valid(&wrong_bind));

    let mut wrong_date_bind = statement_plan(Statement::Entitlements {
        reference: TEST_REFERENCE.to_owned(),
        bare_hash: TEST_LICENSE_HASH.to_owned(),
        dates,
    });
    if let Some(BindValue::DateArray(bound)) = wrong_date_bind.binds.get_mut(2) {
        bound.swap(100, 101);
    }
    assert!(!statement_plan_is_valid(&wrong_date_bind));
}

#[test]
fn projection_guard_rejects_mutations_in_each_real_sql_projection() {
    let fixture = InputFixture::new();
    let prepared = ready(validate_input_view(fixture.view(), &AtomicBool::new(false)));
    let dates = prepared
        .dates
        .iter()
        .map(TradingDate::as_naive_date)
        .collect::<Vec<_>>();
    let projections = [
        (Statement::Configure { timeout_ms: 30_000 }, 9),
        (Statement::InitialContext, 21),
        (Statement::FinalContext, 21),
        (
            Statement::Owner {
                owner: Uuid::from_u128(42),
            },
            2,
        ),
        (
            Statement::Entitlements {
                reference: TEST_REFERENCE.to_owned(),
                bare_hash: TEST_LICENSE_HASH.to_owned(),
                dates,
            },
            19,
        ),
    ];
    for (statement, expected_columns) in projections {
        let plan = statement_plan(statement);
        assert!(statement_plan_is_valid(&plan));
        let sql = plan.sql;
        let shape = plan.shape;
        assert_eq!(pg_column_count(shape), expected_columns);
        assert_eq!(
            projection_signature(sql).map(|columns| columns.len()),
            Some(expected_columns)
        );
        assert!(projection_matches(sql, shape));

        let (start, end) = outer_projection_range(sql).expect("SQL has an outer projection");
        let original = split_top_level_projection(&sql[start..end])
            .expect("outer projection splits at top level")
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let reconstruct = |columns: &[String]| {
            format!("{} {} {}", &sql[..start], columns.join(", "), &sql[end..])
        };

        let first_alias = projection_columns(shape)[0].alias;
        let mut aliased = original.clone();
        let expected_alias = format!(" AS {first_alias}");
        let changed_alias = format!(" AS altered_{first_alias}");
        assert!(aliased[0].contains(&expected_alias));
        aliased[0] = aliased[0].replace(&expected_alias, &changed_alias);
        assert!(!projection_matches(&reconstruct(&aliased), shape));

        let mut reordered = original.clone();
        reordered.swap(0, 1);
        assert!(!projection_matches(&reconstruct(&reordered), shape));

        let mut shortened = original.clone();
        shortened.pop();
        assert!(!projection_matches(&reconstruct(&shortened), shape));

        let mut cast = original;
        let marker = "::pg_catalog.";
        let cast_start = cast[0]
            .find(marker)
            .map(|offset| offset + marker.len())
            .expect("first projection column has an explicit pg_catalog cast");
        let cast_end = cast[0][cast_start..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map(|offset| cast_start + offset)
            .unwrap_or(cast[0].len());
        let cast_type = &cast[0][cast_start..cast_end];
        let changed_type = if cast_type.eq_ignore_ascii_case("bool") {
            "int8"
        } else {
            "bool"
        };
        cast[0].replace_range(cast_start..cast_end, changed_type);
        assert!(!projection_matches(&reconstruct(&cast), shape));
    }
}

#[test]
fn command_result_classifier_projects_only_optional_row_presence() {
    assert!(classify_command_result::<()>(None).is_empty());
    let rows = classify_command_result(Some(()));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].category, RawCategory::Command);
    assert!(rows[0].cells.is_empty());
}

#[test]
fn actual_date_vector_controls_interval_coverage_and_a_changed_vector_changes_identity() {
    let fixture = InputFixture::new();
    let prepared = ready(validate_input_view(fixture.view(), &AtomicBool::new(false)));
    let config = validated_test_config();
    let full = valid_entitlements_for(&prepared.dates);
    ready(validate_entitlements(
        full,
        &config,
        &prepared.dates,
        &AtomicBool::new(false),
        &mut LogicalPayloadBudget::default(),
    ));

    let used = prepared.dates.iter().copied().collect::<BTreeSet<_>>();
    let start = prepared.dates[1].as_naive_date();
    let changed_middle = (1..5_000_i64)
        .map(|offset| start + Duration::days(offset))
        .map(|date| TradingDate::parse(&date.format("%Y-%m-%d").to_string()).expect("valid date"))
        .find(|date| {
            !used.contains(date) && *date < *prepared.dates.last().expect("dates are nonempty")
        })
        .expect("unselected date within the frozen range exists");
    let mut changed_dates = prepared.dates.clone();
    changed_dates[1] = changed_middle;
    changed_dates.sort();
    let still_bounded = changed_dates.len() == EXPECTED_SESSIONS;
    assert!(still_bounded);
    assert_ne!(prepared.dates, changed_dates);
    let from = changed_dates[0].as_naive_date();
    let until = changed_dates[changed_dates.len() - 1].as_naive_date();
    let rows = vec![active_entitlement(
        1,
        from,
        until,
        "[\"krx_eod_bars\"]",
        "[\"backtest\"]",
        true,
    )];
    ready(validate_entitlements(
        rows,
        &config,
        &changed_dates,
        &AtomicBool::new(false),
        &mut LogicalPayloadBudget::default(),
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeAwait {
    Acquire,
    Execute(StatementKind),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeEvent {
    Await(FakeAwait),
    ArmDropGuard,
    DropGuardDisposed,
    Pending(FakeAwait),
}

struct FakeTransport {
    config: ValidatedConfig,
    cancel: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<FakeEvent>>>,
    fail_on: Option<FakeAwait>,
    failure: TransportFailure,
    cancel_on: Option<FakeAwait>,
    pending_on: Option<FakeAwait>,
    nonempty_on: Option<StatementKind>,
    fail_rollback: bool,
    fail_close: bool,
    custom_entitlements: Option<Vec<RawRow>>,
    guarded: bool,
    closed: bool,
}

impl FakeTransport {
    fn new(config: ValidatedConfig, cancel: Arc<AtomicBool>) -> Self {
        Self {
            config,
            cancel,
            events: Arc::new(Mutex::new(Vec::new())),
            fail_on: None,
            failure: TransportFailure::Database,
            cancel_on: None,
            pending_on: None,
            nonempty_on: None,
            fail_rollback: false,
            fail_close: false,
            custom_entitlements: None,
            guarded: false,
            closed: false,
        }
    }

    fn record(&self, event: FakeEvent) {
        self.events
            .lock()
            .expect("fake event lock is healthy")
            .push(event);
    }

    fn events(&self) -> Vec<FakeEvent> {
        self.events
            .lock()
            .expect("fake event lock is healthy")
            .clone()
    }

    async fn at(&mut self, await_point: FakeAwait) -> Result<(), TransportFailure> {
        self.record(FakeEvent::Await(await_point));
        if self.pending_on == Some(await_point) {
            self.record(FakeEvent::Pending(await_point));
            pending::<()>().await;
        }
        if self.cancel_on == Some(await_point) {
            self.cancel.store(true, Ordering::SeqCst);
        }
        let rollback_failure =
            await_point == FakeAwait::Execute(StatementKind::Rollback) && self.fail_rollback;
        let close_failure = await_point == FakeAwait::Close && self.fail_close;
        if self.fail_on == Some(await_point) || rollback_failure || close_failure {
            Err(self.failure)
        } else {
            Ok(())
        }
    }

    fn default_rows(&self, plan: &StatementPlan) -> Result<Vec<RawRow>, TransportFailure> {
        let category = plan.category.ok_or(TransportFailure::State)?;
        let rows = match plan.kind {
            StatementKind::Begin | StatementKind::Commit | StatementKind::Rollback => {
                if !plan.binds.is_empty() {
                    return Err(TransportFailure::State);
                }
                classify_command_result::<()>(None)
            }
            StatementKind::Configure => {
                let [BindValue::I64(timeout_ms)] = plan.binds.as_slice() else {
                    return Err(TransportFailure::State);
                };
                let timeout = format!("{timeout_ms}ms");
                vec![setup_row("pg_catalog", &timeout, &timeout)]
            }
            StatementKind::InitialContext | StatementKind::FinalContext => {
                if !plan.binds.is_empty() {
                    return Err(TransportFailure::State);
                }
                let mut evidence = context_evidence();
                evidence.statement_timeout_ms = self.config.timeout_ms;
                evidence.lock_timeout_ms = self.config.timeout_ms;
                vec![context_row(&evidence, category)]
            }
            StatementKind::Owner => {
                let [BindValue::Uuid(owner)] = plan.binds.as_slice() else {
                    return Err(TransportFailure::State);
                };
                vec![owner_row(*owner, true)]
            }
            StatementKind::Entitlements => {
                let [
                    BindValue::Text(reference),
                    BindValue::Text(bare_hash),
                    BindValue::DateArray(dates),
                ] = plan.binds.as_slice()
                else {
                    return Err(TransportFailure::State);
                };
                if let Some(rows) = &self.custom_entitlements {
                    rows.clone()
                } else {
                    let Some(first) = dates.first().copied() else {
                        return Err(TransportFailure::State);
                    };
                    let Some(last) = dates.last().copied() else {
                        return Err(TransportFailure::State);
                    };
                    vec![entitlement_row(
                        Uuid::from_u128(1),
                        reference,
                        bare_hash,
                        "ACTIVE",
                        "[\"krx_eod_bars\"]",
                        "[\"backtest\"]",
                        first,
                        last,
                        true,
                    )]
                }
            }
        };
        if rows.len() > plan.fetch_rows
            || rows
                .iter()
                .any(|row| row.category != category || row.cells.len() != plan.shape.len())
        {
            return Err(TransportFailure::State);
        }
        Ok(rows)
    }
}

impl Drop for FakeTransport {
    fn drop(&mut self) {
        if self.guarded && !self.closed {
            self.record(FakeEvent::DropGuardDisposed);
        }
    }
}

#[async_trait::async_trait]
impl ObserverTransport for FakeTransport {
    async fn acquire(&mut self) -> Result<(), TransportFailure> {
        self.at(FakeAwait::Acquire).await
    }

    fn arm_close_on_drop(&mut self) {
        self.guarded = true;
        self.record(FakeEvent::ArmDropGuard);
    }

    async fn execute(&mut self, statement: Statement) -> Result<Vec<RawRow>, TransportFailure> {
        let plan = statement_plan(statement);
        if !statement_plan_is_valid(&plan) {
            return Err(TransportFailure::State);
        }
        let kind = plan.kind;
        let await_point = FakeAwait::Execute(kind);
        self.at(await_point).await?;
        if self.nonempty_on == Some(kind) {
            return if plan.category == Some(RawCategory::Command) {
                Ok(classify_command_result(Some(())))
            } else {
                Err(TransportFailure::State)
            };
        }
        self.default_rows(&plan)
    }

    async fn close(&mut self) -> Result<(), TransportFailure> {
        self.at(FakeAwait::Close).await?;
        self.closed = true;
        Ok(())
    }
}

async fn observe_fake(
    transport: &mut FakeTransport,
    canceled: &AtomicBool,
) -> Result<SimulationConfiguredInputRightsObservation, SimulationConfiguredInputRightsError> {
    let fixture = InputFixture::new();
    observe_with_transport(transport, test_config(), fixture.view(), canceled).await
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn poll_pending_once<F: Future>(future: F) -> Pin<Box<F>> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    future
}

fn default_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn run_failed_transport(
    transport: &mut FakeTransport,
    canceled: &AtomicBool,
) -> SimulationConfiguredInputRightsError {
    match block_on(observe_fake(transport, canceled)) {
        Ok(_) => panic!("expected observer to fail closed"),
        Err(error) => error,
    }
}

#[test]
fn successful_fake_observation_uses_guarded_readonly_order_and_hides_values() {
    let canceled = default_cancel();
    let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
    let result = block_on(observe_fake(&mut transport, &canceled))
        .expect("the synthetic fixture and fake observer satisfy the private source contract");
    assert_eq!(result.config_sha256(), result.config_sha256);
    assert_eq!(result.database_oid(), result.database_oid);
    assert_eq!(result.worker_oid(), result.worker_oid);
    assert_eq!(
        transport.events(),
        vec![
            FakeEvent::Await(FakeAwait::Acquire),
            FakeEvent::ArmDropGuard,
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Begin)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Configure)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::InitialContext)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Owner)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Entitlements)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::FinalContext)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Commit)),
            FakeEvent::Await(FakeAwait::Close),
        ]
    );
    let debug = format!("{result:?}");
    assert_eq!(debug, "SimulationConfiguredInputRightsObservation");
    for sentinel in [
        "config-secret-sentinel",
        TEST_REFERENCE,
        TEST_LICENSE_HASH,
        "sha256:candidate-sentinel",
        "sha256:artifact-bars-sentinel",
    ] {
        assert!(!debug.contains(sentinel));
    }
}

#[test]
fn acquisition_error_and_same_await_cancel_preserve_acquisition_failure() {
    let canceled = default_cancel();
    let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
    transport.fail_on = Some(FakeAwait::Acquire);
    transport.cancel_on = Some(FakeAwait::Acquire);
    let error = run_failed_transport(&mut transport, &canceled);
    assert_eq!(error.primary, PrimaryFailure::AcquisitionFailed);
    assert_eq!(error.begin, BeginState::NotAttempted);
    assert!(error.canceled);
    assert_eq!(
        transport.events(),
        vec![FakeEvent::Await(FakeAwait::Acquire)]
    );
}

#[test]
fn same_await_acquire_success_cancellation_closes_without_starting_sql() {
    let canceled = default_cancel();
    let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
    transport.cancel_on = Some(FakeAwait::Acquire);
    let error = run_failed_transport(&mut transport, &canceled);
    assert_eq!(error.primary, PrimaryFailure::Canceled);
    assert_eq!(error.begin, BeginState::NotAttempted);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(error.canceled);
    assert_eq!(
        transport.events(),
        vec![
            FakeEvent::Await(FakeAwait::Acquire),
            FakeEvent::ArmDropGuard,
            FakeEvent::Await(FakeAwait::Close),
        ]
    );
}

#[test]
fn same_await_begin_success_cancel_and_begin_error_cancel_keep_uncertainty() {
    let canceled = default_cancel();
    let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
    transport.cancel_on = Some(FakeAwait::Execute(StatementKind::Begin));
    let error = run_failed_transport(&mut transport, &canceled);
    assert_eq!(error.primary, PrimaryFailure::Canceled);
    assert_eq!(error.begin, BeginState::Started);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(error.canceled);
    assert_eq!(
        &transport.events()[2..],
        &[
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Begin)),
            FakeEvent::Await(FakeAwait::Execute(StatementKind::Rollback)),
            FakeEvent::Await(FakeAwait::Close),
        ]
    );

    for failure in [
        TransportFailure::Database,
        TransportFailure::State,
        TransportFailure::FetchBound,
    ] {
        let canceled = default_cancel();
        let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
        let begin = FakeAwait::Execute(StatementKind::Begin);
        transport.fail_on = Some(begin);
        transport.cancel_on = Some(begin);
        transport.failure = failure;
        let error = run_failed_transport(&mut transport, &canceled);
        assert_eq!(error.primary, PrimaryFailure::BeginFailed);
        assert_eq!(error.begin, BeginState::Uncertain);
        assert_eq!(error.rollback, CleanupState::Confirmed);
        assert_eq!(error.close, CleanupState::Confirmed);
        assert!(error.canceled);
    }
}

#[test]
fn successful_statement_same_await_cancellation_matrix_rolls_back_and_closes() {
    for kind in [
        StatementKind::Configure,
        StatementKind::InitialContext,
        StatementKind::Owner,
        StatementKind::Entitlements,
        StatementKind::FinalContext,
    ] {
        let canceled = default_cancel();
        let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
        transport.cancel_on = Some(FakeAwait::Execute(kind));
        let error = run_failed_transport(&mut transport, &canceled);
        assert_eq!(
            error.primary,
            PrimaryFailure::Canceled,
            "same-await cancellation after {kind:?}"
        );
        assert_eq!(error.begin, BeginState::Started);
        assert_eq!(error.rollback, CleanupState::Confirmed);
        assert_eq!(error.close, CleanupState::Confirmed);
        assert!(error.canceled);
    }
}

#[test]
fn failed_statement_same_await_cancellation_matrix_preserves_primary_failure() {
    for kind in [
        StatementKind::Configure,
        StatementKind::InitialContext,
        StatementKind::Owner,
        StatementKind::Entitlements,
        StatementKind::FinalContext,
    ] {
        for failure in [
            TransportFailure::Database,
            TransportFailure::State,
            TransportFailure::FetchBound,
        ] {
            let canceled = default_cancel();
            let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
            let point = FakeAwait::Execute(kind);
            transport.fail_on = Some(point);
            transport.failure = failure;
            transport.cancel_on = Some(point);
            let error = run_failed_transport(&mut transport, &canceled);
            let fallback = if kind == StatementKind::Configure {
                PrimaryFailure::SetupFailed
            } else {
                PrimaryFailure::QueryUnavailable
            };
            assert_eq!(
                error.primary,
                if failure == TransportFailure::FetchBound {
                    PrimaryFailure::BoundExceeded
                } else {
                    fallback
                },
                "transport failure at {kind:?}"
            );
            assert_eq!(error.begin, BeginState::Started);
            assert_eq!(error.rollback, CleanupState::Confirmed);
            assert_eq!(error.close, CleanupState::Confirmed);
            assert!(error.canceled);
        }
    }
}

#[test]
fn begin_nonempty_and_rollback_nonempty_are_uncertain_without_observation() {
    let canceled = default_cancel();
    let mut begin_nonempty = FakeTransport::new(validated_test_config(), canceled.clone());
    begin_nonempty.nonempty_on = Some(StatementKind::Begin);
    let error = run_failed_transport(&mut begin_nonempty, &canceled);
    assert_eq!(error.primary, PrimaryFailure::BeginResultInvalid);
    assert_eq!(error.begin, BeginState::Uncertain);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(
        !begin_nonempty
            .events()
            .contains(&FakeEvent::Await(FakeAwait::Execute(StatementKind::Commit)))
    );

    let canceled = default_cancel();
    let mut rollback_nonempty = FakeTransport::new(validated_test_config(), canceled.clone());
    rollback_nonempty.fail_on = Some(FakeAwait::Execute(StatementKind::FinalContext));
    rollback_nonempty.nonempty_on = Some(StatementKind::Rollback);
    let error = run_failed_transport(&mut rollback_nonempty, &canceled);
    assert_eq!(error.primary, PrimaryFailure::QueryUnavailable);
    assert_eq!(error.rollback, CleanupState::Unconfirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(
        !rollback_nonempty
            .events()
            .contains(&FakeEvent::Await(FakeAwait::Execute(StatementKind::Commit)))
    );
}

#[test]
fn commit_state_and_fetch_bound_same_await_cancel_keep_completion_uncertainty() {
    for failure in [TransportFailure::State, TransportFailure::FetchBound] {
        for (fail_rollback, fail_close) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let canceled = default_cancel();
            let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
            let commit = FakeAwait::Execute(StatementKind::Commit);
            transport.fail_on = Some(commit);
            transport.failure = failure;
            transport.cancel_on = Some(commit);
            transport.fail_rollback = fail_rollback;
            transport.fail_close = fail_close;
            let error = run_failed_transport(&mut transport, &canceled);
            assert_eq!(error.primary, PrimaryFailure::CompletionUnconfirmed);
            assert_eq!(error.begin, BeginState::Started);
            assert_eq!(error.completion, CompletionState::Unconfirmed);
            assert_eq!(
                error.rollback,
                if fail_rollback {
                    CleanupState::Unconfirmed
                } else {
                    CleanupState::Confirmed
                }
            );
            assert_eq!(
                error.close,
                if fail_close {
                    CleanupState::Unconfirmed
                } else {
                    CleanupState::Confirmed
                }
            );
            assert!(error.canceled);
            let events = transport.events();
            let commit_at = events
                .iter()
                .position(|event| *event == FakeEvent::Await(commit))
                .expect("commit was attempted");
            assert_eq!(
                &events[commit_at..],
                &[
                    FakeEvent::Await(commit),
                    FakeEvent::Await(FakeAwait::Execute(StatementKind::Rollback)),
                    FakeEvent::Await(FakeAwait::Close),
                ]
            );
        }
    }
}

#[test]
fn commit_database_and_nonempty_commit_also_roll_back_then_close() {
    let cleanup_outcomes = [(false, false), (false, true), (true, false), (true, true)];
    for nonempty in [false, true] {
        for cancel_during_commit in [false, true] {
            for (fail_rollback, fail_close) in cleanup_outcomes {
                let canceled = default_cancel();
                let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
                let commit = FakeAwait::Execute(StatementKind::Commit);
                if nonempty {
                    transport.nonempty_on = Some(StatementKind::Commit);
                } else {
                    transport.fail_on = Some(commit);
                    transport.failure = TransportFailure::Database;
                }
                if cancel_during_commit {
                    transport.cancel_on = Some(commit);
                }
                transport.fail_rollback = fail_rollback;
                transport.fail_close = fail_close;

                let error = run_failed_transport(&mut transport, &canceled);
                assert_eq!(error.primary, PrimaryFailure::CompletionUnconfirmed);
                assert_eq!(error.begin, BeginState::Started);
                assert_eq!(error.completion, CompletionState::Unconfirmed);
                assert_eq!(
                    error.rollback,
                    if fail_rollback {
                        CleanupState::Unconfirmed
                    } else {
                        CleanupState::Confirmed
                    }
                );
                assert_eq!(
                    error.close,
                    if fail_close {
                        CleanupState::Unconfirmed
                    } else {
                        CleanupState::Confirmed
                    }
                );
                assert_eq!(error.canceled, cancel_during_commit);
                let events = transport.events();
                let commit_at = events
                    .iter()
                    .position(|event| *event == FakeEvent::Await(commit))
                    .expect("commit was attempted");
                assert_eq!(
                    &events[commit_at..],
                    &[
                        FakeEvent::Await(commit),
                        FakeEvent::Await(FakeAwait::Execute(StatementKind::Rollback)),
                        FakeEvent::Await(FakeAwait::Close),
                    ]
                );
            }
        }
    }
}

#[test]
fn acknowledged_commit_cancel_and_close_failure_never_create_observation() {
    let canceled = default_cancel();
    let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
    transport.cancel_on = Some(FakeAwait::Execute(StatementKind::Commit));
    let error = run_failed_transport(&mut transport, &canceled);
    assert_eq!(error.primary, PrimaryFailure::Canceled);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.rollback, CleanupState::NotApplicable);
    assert_eq!(error.close, CleanupState::Confirmed);

    let canceled = default_cancel();
    let mut close_failure = FakeTransport::new(validated_test_config(), canceled.clone());
    close_failure.fail_close = true;
    close_failure.cancel_on = Some(FakeAwait::Close);
    let error = run_failed_transport(&mut close_failure, &canceled);
    assert_eq!(error.primary, PrimaryFailure::CleanupUnconfirmed);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.rollback, CleanupState::NotApplicable);
    assert_eq!(error.close, CleanupState::Unconfirmed);
    assert!(error.canceled);

    let canceled = default_cancel();
    let mut close_cancel = FakeTransport::new(validated_test_config(), canceled.clone());
    close_cancel.cancel_on = Some(FakeAwait::Close);
    let error = run_failed_transport(&mut close_cancel, &canceled);
    assert_eq!(error.primary, PrimaryFailure::Canceled);
    assert_eq!(error.completion, CompletionState::Acknowledged);
    assert_eq!(error.rollback, CleanupState::NotApplicable);
    assert_eq!(error.close, CleanupState::Confirmed);
}

#[test]
fn rollback_error_and_cancel_are_sticky_and_close_is_independent() {
    let canceled = default_cancel();
    let mut rollback_success_cancel = FakeTransport::new(validated_test_config(), canceled.clone());
    rollback_success_cancel.fail_on = Some(FakeAwait::Execute(StatementKind::FinalContext));
    rollback_success_cancel.cancel_on = Some(FakeAwait::Execute(StatementKind::Rollback));
    let error = run_failed_transport(&mut rollback_success_cancel, &canceled);
    assert_eq!(error.primary, PrimaryFailure::QueryUnavailable);
    assert_eq!(error.rollback, CleanupState::Confirmed);
    assert_eq!(error.close, CleanupState::Confirmed);
    assert!(error.canceled);

    for failure in [
        TransportFailure::Database,
        TransportFailure::State,
        TransportFailure::FetchBound,
    ] {
        let canceled = default_cancel();
        let mut transport = FakeTransport::new(validated_test_config(), canceled.clone());
        transport.fail_on = Some(FakeAwait::Execute(StatementKind::FinalContext));
        transport.cancel_on = Some(FakeAwait::Execute(StatementKind::Rollback));
        transport.fail_rollback = true;
        transport.failure = failure;
        let error = run_failed_transport(&mut transport, &canceled);
        assert_eq!(
            error.primary,
            if failure == TransportFailure::FetchBound {
                PrimaryFailure::BoundExceeded
            } else {
                PrimaryFailure::QueryUnavailable
            }
        );
        assert_eq!(error.rollback, CleanupState::Unconfirmed);
        assert_eq!(error.close, CleanupState::Confirmed);
        assert!(error.canceled);
    }
}

#[test]
fn dropped_pending_acquire_commit_rollback_and_close_do_not_claim_acknowledgement() {
    let canceled = default_cancel();
    let mut acquire = FakeTransport::new(validated_test_config(), canceled.clone());
    acquire.pending_on = Some(FakeAwait::Acquire);
    let pending = poll_pending_once(observe_fake(&mut acquire, &canceled));
    drop(pending);
    let acquire_events = acquire.events();
    assert_eq!(
        acquire_events,
        vec![
            FakeEvent::Await(FakeAwait::Acquire),
            FakeEvent::Pending(FakeAwait::Acquire),
        ]
    );
    drop(acquire);

    let canceled = default_cancel();
    let mut commit = FakeTransport::new(validated_test_config(), canceled.clone());
    commit.pending_on = Some(FakeAwait::Execute(StatementKind::Commit));
    let shared_events = commit.events.clone();
    let pending = poll_pending_once(observe_fake(&mut commit, &canceled));
    drop(pending);
    drop(commit);
    let commit_events = shared_events.lock().expect("events").clone();
    assert!(commit_events.contains(&FakeEvent::DropGuardDisposed));
    assert!(
        !commit_events.contains(&FakeEvent::Await(FakeAwait::Execute(
            StatementKind::Rollback
        )))
    );
    assert!(!commit_events.contains(&FakeEvent::Await(FakeAwait::Close)));

    let canceled = default_cancel();
    let mut rollback = FakeTransport::new(validated_test_config(), canceled.clone());
    rollback.fail_on = Some(FakeAwait::Execute(StatementKind::FinalContext));
    rollback.pending_on = Some(FakeAwait::Execute(StatementKind::Rollback));
    let shared_events = rollback.events.clone();
    let pending = poll_pending_once(observe_fake(&mut rollback, &canceled));
    drop(pending);
    drop(rollback);
    let rollback_events = shared_events.lock().expect("events").clone();
    assert!(rollback_events.contains(&FakeEvent::DropGuardDisposed));
    assert!(!rollback_events.contains(&FakeEvent::Await(FakeAwait::Close)));

    let canceled = default_cancel();
    let mut close = FakeTransport::new(validated_test_config(), canceled.clone());
    close.pending_on = Some(FakeAwait::Close);
    let shared_events = close.events.clone();
    let pending = poll_pending_once(observe_fake(&mut close, &canceled));
    drop(pending);
    drop(close);
    let close_events = shared_events.lock().expect("events").clone();
    assert!(close_events.contains(&FakeEvent::DropGuardDisposed));
    assert!(!close_events.contains(&FakeEvent::Await(FakeAwait::Execute(
        StatementKind::Rollback
    ))));
}

#[test]
fn private_errors_and_success_debug_never_include_sentinels_or_provider_text() {
    let error = SimulationConfiguredInputRightsError::primary(PrimaryFailure::QueryUnavailable);
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert_eq!(debug, "SimulationConfiguredInputRightsError");
    for value in [
        "config-secret-sentinel",
        TEST_REFERENCE,
        TEST_LICENSE_HASH,
        "provider-private-diagnostic",
        "SELECT",
    ] {
        assert!(!debug.contains(value));
        assert!(!display.contains(value));
    }
}
