use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use domain::{BatchId, CodeCommit, ContentHash, InstrumentId, TradingDate};
use market_data::daily_price_only::*;

const SYMBOLS: [&str; 11] = [
    "069500", "102110", "114260", "132030", "133690", "143850", "148070", "153130", "192090",
    "195930", "229200",
];
const INSTRUMENTS: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "114260.KRX",
    "132030.KRX",
    "133690.KRX",
    "143850.KRX",
    "148070.KRX",
    "153130.KRX",
    "192090.KRX",
    "195930.KRX",
    "229200.KRX",
];
const CASH_CODE_FIELDS: [&str; 3] = ["dividend_kind", "stock_kind", "high_dividend_flag"];
const PRESENT_EMPTY_CASH_ROWS_JSON: &str = r#"[{"symbol":"069500","record_date":"2026-08-31","dividend_kind":"","face_value":"5000","cash_dividend_amount":"100","cash_dividend_rate":"2","stock_dividend_rate":"0","cash_payment_date":"2026-09-14","stock_payment_date":null,"odd_lot_payment_date":null,"stock_kind":"","high_dividend_flag":"","source_file":{"provider":"kis","market":"kr","batch_id":"00000000-0000-4000-8000-000000000001","file_name":"dividend-page-01.json","wire_sha256":"sha256:5b0f4798540250232a5d5321f61a1c0ba5263adde7400078a7b06d0904c61c39"}}]"#;

#[derive(Clone)]
struct Fixture {
    policy: DailyPricePolicyV1Wire,
    draft: DailyPriceManifestDraftV1Wire,
    manifest: DailyPriceManifestV1Wire,
    proof: DailyNormalizationProofV1Wire,
    admission: DailyPriceAdmissionV1Wire,
    pins: DailyPricePinsV1Wire,
}

type ActionSpec = (
    DailyActionClassV1,
    &'static str,
    &'static str,
    BTreeMap<String, String>,
);
type FileKeySortParts = (String, String, String, String, String);

struct FileFixture<'a> {
    kind: DailyEvidenceKindV1,
    name: &'a str,
    provider: &'a str,
    batch_id: BatchId,
    batch_date: &'a str,
    batch_json_hash: ContentHash,
    manifest_line_hash: ContentHash,
    request: DailyRequestMetadataV1Wire,
    retrieved_at: &'a str,
    response_continuation: Option<&'a str>,
}

struct SessionFixture<'a> {
    session_date: &'a str,
    previous_frontier: &'a str,
    calendar_dates: &'a [(&'a str, DailySessionTypeV1)],
    batch_id: BatchId,
    batch_date: &'a str,
    batch_hash: ContentHash,
    line_hash: ContentHash,
    retrieved_at: &'a str,
}

fn hash(bytes: &[u8]) -> ContentHash {
    ContentHash::from_bytes(bytes)
}

fn sh(n: u64) -> ContentHash {
    ContentHash::parse(&format!("sha256:{n:064x}")).unwrap()
}

fn date(value: &str) -> TradingDate {
    TradingDate::parse(value).unwrap()
}

fn ts(value: &str) -> DailyWireTimestampV1 {
    DailyWireTimestampV1::parse(value).unwrap()
}

fn batch(value: &str) -> BatchId {
    BatchId::from_str(value).unwrap()
}

fn ids() -> Vec<InstrumentId> {
    INSTRUMENTS
        .iter()
        .map(|value| InstrumentId::from_str(value).unwrap())
        .collect()
}

fn base() -> DailyBaseV1Wire {
    DailyBaseV1Wire {
        kind: "historical_price_only_v3".to_owned(),
        pins: DailyBasePinsV1Wire {
            candidate_content_sha256: sh(1),
            artifact_manifest_sha256: sh(2),
            stage5_manifest_sha256: sh(3),
            action_manifest_sha256: sh(4),
            approval_registry_sha256: sh(5),
        },
    }
}

fn policy() -> DailyPricePolicyV1Wire {
    DailyPricePolicyV1Wire {
        schema: "owner-beta-daily-etf11-policy-v1".to_owned(),
        capability: "PRICE_RETURN_ONLY".to_owned(),
        audience: "OWNER_ONLY".to_owned(),
        vendor_snapshot: true,
        strict_pit: false,
        base: base(),
        instruments: ids(),
        price_semantics: "KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC".to_owned(),
        cash_action_policy: "CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1".to_owned(),
        normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
        materializer_version: "daily-price-only-v1".to_owned(),
        source_kinds: vec![
            source_kind("kis", DailyEvidenceKindV1::Bars),
            source_kind("kis", DailyEvidenceKindV1::Calendar),
            source_kind("kis", DailyEvidenceKindV1::CorporateActions),
            source_kind("kis", DailyEvidenceKindV1::Reference),
            source_kind("kis-calendar", DailyEvidenceKindV1::Calendar),
        ],
        limits: DailyResourceLimitsV1Wire {
            max_policy_bytes: 262_144,
            max_manifest_bytes: 16_777_216,
            max_admission_bytes: 1_048_576,
            max_normalization_bytes: 16_777_216,
            max_bars_bytes: 134_217_728,
            max_sessions: 20_000,
            max_civil_dates_per_addition: 370,
            max_action_chains_per_session: 32_768,
            max_file_evidence_per_session: 262_144,
            max_source_file_bytes: 67_108_864,
            max_normalized_document_bytes: 67_108_864,
            max_copy_depth: 16,
            max_pages_per_action_chain: 10,
            max_cash_rows_per_chain: 100_000,
            max_json_depth: 32,
        },
    }
}

fn source_kind(provider: &str, kind: DailyEvidenceKindV1) -> DailySourceKindV1Wire {
    DailySourceKindV1Wire {
        provider: provider.to_owned(),
        market: "kr".to_owned(),
        kind,
    }
}

fn kind_name(kind: DailyEvidenceKindV1) -> &'static str {
    match kind {
        DailyEvidenceKindV1::Bars => "bars",
        DailyEvidenceKindV1::Reference => "reference",
        DailyEvidenceKindV1::Calendar => "calendar",
        DailyEvidenceKindV1::CorporateActions => "corporate_actions",
    }
}

fn pairs(map: BTreeMap<String, String>) -> Vec<(String, String)> {
    map.into_iter().collect()
}

fn request(
    path: &str,
    tr_id: &str,
    query: BTreeMap<String, String>,
    continuation: &str,
) -> DailyRequestMetadataV1Wire {
    DailyRequestMetadataV1Wire {
        endpoint: format!("/uapi/domestic-stock/v1/{path}"),
        query: pairs(query),
        headers: pairs(BTreeMap::from([
            ("appkey".to_owned(), "[REDACTED]".to_owned()),
            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
            ("authorization".to_owned(), "[REDACTED]".to_owned()),
            ("tr_cont".to_owned(), continuation.to_owned()),
            ("tr_id".to_owned(), tr_id.to_owned()),
        ])),
        mode: "credentialed".to_owned(),
    }
}

fn file(seed: FileFixture<'_>) -> DailyFileEvidenceV1Wire {
    let wire = format!("synthetic-wire:{}", seed.name);
    let retrieved = ts(seed.retrieved_at);
    DailyFileEvidenceV1Wire {
        provider: seed.provider.to_owned(),
        market: "kr".to_owned(),
        batch_id: seed.batch_id,
        batch_date: date(seed.batch_date),
        batch_json_sha256: seed.batch_json_hash,
        manifest_line_sha256: seed.manifest_line_hash,
        file_name: seed.name.to_owned(),
        kind: seed.kind,
        wire_sha256: hash(wire.as_bytes()),
        size_bytes: wire.len() as u64,
        request: seed.request,
        response_continuation: seed.response_continuation.map(str::to_owned),
        retrieved_at: retrieved,
        retrieval_precision: "batch".to_owned(),
        original_retrieved_at: retrieved,
        original_retrieval_precision: "batch".to_owned(),
        copied_from: Vec::new(),
    }
}

fn file_key(file: &DailyFileEvidenceV1Wire) -> DailyFileKeyV1Wire {
    DailyFileKeyV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        file_name: file.file_name.clone(),
        wire_sha256: file.wire_sha256.clone(),
    }
}

fn file_origin(
    file: &DailyFileEvidenceV1Wire,
    batch_number: u64,
    retrieved_at: DailyWireTimestampV1,
) -> DailyFileOriginV1Wire {
    DailyFileOriginV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: batch(&format!("00000000-0000-4000-8000-{batch_number:012x}")),
        batch_date: file.batch_date,
        batch_json_sha256: sh(batch_number + 1),
        manifest_line_sha256: sh(batch_number + 2),
        file_name: file.file_name.clone(),
        kind: file.kind,
        wire_sha256: file.wire_sha256.clone(),
        size_bytes: file.size_bytes,
        request: file.request.clone(),
        response_continuation: file.response_continuation.clone(),
        retrieved_at,
        retrieval_precision: file.retrieval_precision.clone(),
    }
}

fn set_copy_depth(file: &mut DailyFileEvidenceV1Wire, depth: usize, first_batch: u64) {
    let mut copied_from = Vec::with_capacity(depth);
    for hop in 0..depth {
        let nanos = 123_456_788u32 - u32::try_from(hop).unwrap();
        let retrieved_at = ts(&format!("2026-08-31T09:00:00.{nanos:09}Z"));
        copied_from.push(file_origin(
            file,
            first_batch + u64::try_from(hop).unwrap(),
            retrieved_at,
        ));
    }
    file.original_retrieved_at = copied_from
        .last()
        .map(|origin| origin.retrieved_at)
        .unwrap_or(file.retrieved_at);
    file.copied_from = copied_from;
}

fn key_sort(file: &DailyFileKeyV1Wire) -> FileKeySortParts {
    (
        file.provider.clone(),
        file.market.clone(),
        file.batch_id.to_string(),
        file.file_name.clone(),
        file.wire_sha256.to_string(),
    )
}

fn document(
    kind: DailyEvidenceKindV1,
    session_date: &str,
    mut source_files: Vec<DailyFileKeyV1Wire>,
    row_count: u64,
) -> DailyNormalizedDocumentV1Wire {
    source_files.sort_by_key(key_sort);
    let commitment = DailyNormalizedSourceCommitmentV1Wire {
        schema: "owner-beta-daily-normalized-source-v1".to_owned(),
        kind,
        session_date: date(session_date),
        source_files: source_files.clone(),
    };
    let evidence_preimage = daily_normalized_source_preimage_v1(&commitment).unwrap();
    let body = format!("synthetic-normalized:{}:{session_date}", kind_name(kind));
    DailyNormalizedDocumentV1Wire {
        kind,
        session_date: date(session_date),
        normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
        sha256: hash(body.as_bytes()),
        size_bytes: body.len() as u64,
        row_count,
        source_files,
        source_evidence_sha256: hash(&evidence_preimage),
    }
}

fn action_spec(name: &str) -> ActionSpec {
    let mut extra = BTreeMap::new();
    let (class, path, tr_id) = match name {
        "bonus-issue" => (
            DailyActionClassV1::BonusIssue,
            "bonus-issue",
            "HHKDB669101C0",
        ),
        "capital-decrease" => (
            DailyActionClassV1::CapitalDecrease,
            "cap-dcrs",
            "HHKDB669106C0",
        ),
        "dividend" => (DailyActionClassV1::Dividend, "dividend", "HHKDB669102C0"),
        "merger-split" => (
            DailyActionClassV1::MergerSplit,
            "merger-split",
            "HHKDB669104C0",
        ),
        "paidin-record" => {
            extra.insert("GB1".to_owned(), "2".to_owned());
            (
                DailyActionClassV1::PaidinRecord,
                "paidin-capin",
                "HHKDB669100C0",
            )
        }
        "paidin-subscription" => {
            extra.insert("GB1".to_owned(), "1".to_owned());
            (
                DailyActionClassV1::PaidinSubscription,
                "paidin-capin",
                "HHKDB669100C0",
            )
        }
        "reverse-split" => {
            extra.insert("MARKET_GB".to_owned(), "0".to_owned());
            (
                DailyActionClassV1::ReverseSplit,
                "rev-split",
                "HHKDB669105C0",
            )
        }
        _ => unreachable!(),
    };
    if name == "dividend" {
        extra.insert("GB1".to_owned(), "0".to_owned());
        extra.insert("HIGH_GB".to_owned(), String::new());
    }
    (class, path, tr_id, extra)
}

fn make_session(seed: SessionFixture<'_>) -> DailySessionEvidenceV1Wire {
    let SessionFixture {
        session_date,
        previous_frontier,
        calendar_dates,
        batch_id,
        batch_date,
        batch_hash,
        line_hash,
        retrieved_at,
    } = seed;
    let mut bars = Vec::new();
    let mut reference = Vec::new();
    for symbol in SYMBOLS {
        let ymd = session_date.replace('-', "");
        bars.push(file(FileFixture {
            kind: DailyEvidenceKindV1::Bars,
            name: &format!("bars-{symbol}.json"),
            provider: "kis",
            batch_id,
            batch_date,
            batch_json_hash: batch_hash.clone(),
            manifest_line_hash: line_hash.clone(),
            request: request(
                "quotations/inquire-daily-itemchartprice",
                "FHKST03010100",
                BTreeMap::from([
                    ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                    ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
                    ("FID_INPUT_DATE_1".to_owned(), ymd.clone()),
                    ("FID_INPUT_DATE_2".to_owned(), ymd),
                    ("FID_PERIOD_DIV_CODE".to_owned(), "D".to_owned()),
                    ("FID_ORG_ADJ_PRC".to_owned(), "1".to_owned()),
                ]),
                "",
            ),
            retrieved_at,
            response_continuation: Some(""),
        }));
        reference.push(file(FileFixture {
            kind: DailyEvidenceKindV1::Reference,
            name: &format!("reference-{symbol}.json"),
            provider: "kis",
            batch_id,
            batch_date,
            batch_json_hash: batch_hash.clone(),
            manifest_line_hash: line_hash.clone(),
            request: request(
                "quotations/inquire-price",
                "FHKST01010100",
                BTreeMap::from([
                    ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                    ("FID_INPUT_ISCD".to_owned(), symbol.to_owned()),
                ]),
                "",
            ),
            retrieved_at,
            response_continuation: Some(""),
        }));
    }

    let calendar_request = request(
        "quotations/chk-holiday",
        "CTCA0903R",
        BTreeMap::from([
            ("BASS_DT".to_owned(), batch_date.replace('-', "")),
            ("CTX_AREA_FK".to_owned(), String::new()),
            ("CTX_AREA_NK".to_owned(), String::new()),
        ]),
        "",
    );
    let calendar_file = file(FileFixture {
        kind: DailyEvidenceKindV1::Calendar,
        name: "calendar-page-01.json",
        provider: "kis",
        batch_id,
        batch_date,
        batch_json_hash: batch_hash.clone(),
        manifest_line_hash: line_hash.clone(),
        request: calendar_request,
        retrieved_at,
        response_continuation: Some(""),
    });

    let (action_start, action_end) = if session_date == "2026-08-31" {
        ("2026-08-29", "2026-08-31")
    } else {
        (session_date, session_date)
    };
    let mut actions = Vec::new();
    for name in [
        "bonus-issue",
        "capital-decrease",
        "dividend",
        "merger-split",
        "paidin-record",
        "paidin-subscription",
        "reverse-split",
    ] {
        let (class, path, tr_id, extra) = action_spec(name);
        let mut query = BTreeMap::from([
            ("CTS".to_owned(), String::new()),
            ("F_DT".to_owned(), action_start.replace('-', "")),
            ("T_DT".to_owned(), action_end.replace('-', "")),
            ("SHT_CD".to_owned(), String::new()),
        ]);
        query.extend(extra);
        let page = file(FileFixture {
            kind: DailyEvidenceKindV1::CorporateActions,
            name: &format!("{name}-page-01.json"),
            provider: "kis",
            batch_id,
            batch_date,
            batch_json_hash: batch_hash.clone(),
            manifest_line_hash: line_hash.clone(),
            request: request(&format!("ksdinfo/{path}"), tr_id, query, ""),
            retrieved_at,
            response_continuation: Some(""),
        });
        actions.push(DailyActionChainV1Wire {
            logical_class: class,
            scope: DailyActionScopeV1::WholeMarket,
            symbol: None,
            range_start: date(action_start),
            range_end: date(action_end),
            pages: vec![page],
            cash_dividends: Vec::new(),
            cash_dividend_rows_sha256: hash(b"[]"),
        });
    }

    let mut calendar = Vec::new();
    let mut documents = Vec::new();
    let bars_document = document(
        DailyEvidenceKindV1::Bars,
        session_date,
        bars.iter().map(file_key).collect(),
        11,
    );
    let reference_document = document(
        DailyEvidenceKindV1::Reference,
        session_date,
        reference.iter().map(file_key).collect(),
        11,
    );
    let corporate_document = document(
        DailyEvidenceKindV1::CorporateActions,
        session_date,
        actions
            .iter()
            .flat_map(|action| action.pages.iter().map(file_key))
            .collect(),
        0,
    );
    documents.push(bars_document.clone());
    documents.push(reference_document.clone());
    documents.push(corporate_document);
    for (civil_date, session_type) in calendar_dates {
        let calendar_document = document(
            DailyEvidenceKindV1::Calendar,
            civil_date,
            vec![file_key(&calendar_file)],
            1,
        );
        calendar.push(DailyCalendarBindingV1Wire {
            civil_date: date(civil_date),
            session_type: *session_type,
            exchange: "KRX".to_owned(),
            timezone: "Asia/Seoul".to_owned(),
            source_file: calendar_file.clone(),
            normalized_document_sha256: calendar_document.sha256.clone(),
            legacy_projection: None,
        });
        documents.push(calendar_document);
    }
    documents.sort_by_key(|document| (document.session_date, kind_name(document.kind)));
    DailySessionEvidenceV1Wire {
        session_date: date(session_date),
        previous_frontier: date(previous_frontier),
        instruments: ids(),
        bars,
        reference,
        calendar,
        actions,
        normalized_documents: documents,
    }
}

fn make_sessions(two: bool) -> Vec<DailySessionEvidenceV1Wire> {
    let mut sessions = vec![make_session(SessionFixture {
        session_date: "2026-08-31",
        previous_frontier: "2026-08-28",
        calendar_dates: &[
            ("2026-08-29", DailySessionTypeV1::Closed),
            ("2026-08-30", DailySessionTypeV1::Closed),
            ("2026-08-31", DailySessionTypeV1::Trading),
        ],
        batch_id: batch("00000000-0000-4000-8000-000000000001"),
        batch_date: "2026-08-31",
        batch_hash: sh(20),
        line_hash: sh(21),
        retrieved_at: "2026-08-31T09:00:00.123456789Z",
    })];
    if two {
        sessions.push(make_session(SessionFixture {
            session_date: "2026-09-01",
            previous_frontier: "2026-08-31",
            calendar_dates: &[("2026-09-01", DailySessionTypeV1::Trading)],
            batch_id: batch("00000000-0000-4000-8000-000000000002"),
            batch_date: "2026-09-01",
            batch_hash: sh(22),
            line_hash: sh(23),
            retrieved_at: "2026-09-01T09:00:00.123456789Z",
        }));
    }
    sessions
}

fn make_draft(
    sessions: Vec<DailySessionEvidenceV1Wire>,
    parent: Option<ContentHash>,
) -> DailyPriceManifestDraftV1Wire {
    let end = sessions.last().unwrap().session_date;
    let has_parent = parent.is_some();
    let sessions_list = if sessions.len() == 1 {
        vec![date("2016-08-29"), date("2026-08-28"), date("2026-08-31")]
    } else {
        vec![
            date("2016-08-29"),
            date("2026-08-28"),
            date("2026-08-31"),
            date("2026-09-01"),
        ]
    };
    DailyPriceManifestDraftV1Wire {
        schema: "owner-beta-daily-etf11-input-v1".to_owned(),
        capability: "PRICE_RETURN_ONLY".to_owned(),
        audience: "OWNER_ONLY".to_owned(),
        vendor_snapshot: true,
        strict_pit: false,
        base: base(),
        parent_admission_sha256: parent,
        range_start: date("2016-08-29"),
        range_end: end,
        instruments: ids(),
        sessions: sessions_list,
        daily_sources: sessions,
        bars: DailyBarsDescriptorV1Wire {
            file: "bars.ndjson".to_owned(),
            sha256: hash(b"synthetic-bars-not-approved"),
            size_bytes: b"synthetic-bars-not-approved".len() as u64,
            row_count: if has_parent { 44 } else { 33 },
        },
        normalization: DailyNormalizationDescriptorV1Wire {
            file: "normalization.json".to_owned(),
            sha256: sh(24),
            size_bytes: 1,
            row_count: if has_parent { 10 } else { 6 },
        },
        policy_sha256: sh(25),
        materializer_version: "daily-price-only-v1".to_owned(),
        code_commit: CodeCommit::parse(&"1".repeat(40)).unwrap(),
    }
}

fn make_proof(sessions: &[DailySessionEvidenceV1Wire]) -> DailyNormalizationProofV1Wire {
    let evidence = daily_price_evidence_preimage_v1(sessions).unwrap();
    let mut documents: Vec<_> = sessions
        .iter()
        .flat_map(|session| session.normalized_documents.iter().cloned())
        .collect();
    documents.sort_by_key(|document| (document.session_date, kind_name(document.kind)));
    DailyNormalizationProofV1Wire {
        schema: "owner-beta-daily-etf11-normalization-v1".to_owned(),
        normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
        evidence_sha256: hash(&evidence),
        documents,
    }
}

fn proof_for_calendar_year(year: i32) -> DailyNormalizationProofV1Wire {
    let mut proof = fixture(false).proof;
    let mut document = proof.documents.remove(0);
    document.session_date = TradingDate::new(year, 1, 1).unwrap();
    let commitment = DailyNormalizedSourceCommitmentV1Wire {
        schema: "owner-beta-daily-normalized-source-v1".to_owned(),
        kind: document.kind,
        session_date: document.session_date,
        source_files: document.source_files.clone(),
    };
    document.source_evidence_sha256 = hash(&serde_json::to_vec(&commitment).unwrap());
    proof.documents = vec![document];
    proof
}

fn counts(
    sessions: &[DailySessionEvidenceV1Wire],
    total_session_count: usize,
) -> DailyAdmissionCountsV1Wire {
    let mut files = BTreeSet::new();
    let mut calendar_rows = 0u64;
    let mut chains = 0u64;
    let mut pages = 0u64;
    let mut cash = 0u64;
    let mut docs = BTreeSet::new();
    for session in sessions {
        calendar_rows += session.calendar.len() as u64;
        chains += session.actions.len() as u64;
        for action in &session.actions {
            pages += action.pages.len() as u64;
            cash += action.cash_dividends.len() as u64;
        }
        for file in session
            .bars
            .iter()
            .chain(session.reference.iter())
            .chain(session.calendar.iter().map(|binding| &binding.source_file))
            .chain(
                session
                    .actions
                    .iter()
                    .flat_map(|action| action.pages.iter()),
            )
        {
            files.insert((
                file.provider.clone(),
                file.market.clone(),
                file.batch_id.to_string(),
                file.file_name.clone(),
            ));
        }
        for document in &session.normalized_documents {
            docs.insert((document.session_date, kind_name(document.kind)));
        }
    }
    let added = sessions.len() as u64;
    let session_count = total_session_count as u64;
    DailyAdmissionCountsV1Wire {
        instrument_count: 11,
        session_count,
        bar_count: session_count * 11,
        added_session_count: added,
        source_file_count: files.len() as u64,
        calendar_row_count: calendar_rows,
        action_chain_count: chains,
        action_page_count: pages,
        cash_dividend_row_count: cash,
        normalized_document_count: docs.len() as u64,
    }
}

fn fixture(two: bool) -> Fixture {
    let policy = policy();
    let policy_hash = hash_daily_price_policy_v1(&policy).unwrap();
    let sessions = make_sessions(two);
    let parent = if two { Some(sh(27)) } else { None };
    let mut draft = make_draft(sessions, parent.clone());
    draft.policy_sha256 = policy_hash.clone();
    let canonical_manifest = seal_daily_price_manifest_v1(&draft).unwrap();
    draft.daily_sources = canonical_manifest.daily_sources.clone();
    let proof = make_proof(&draft.daily_sources);
    let proof_bytes = encode_daily_normalization_proof_v1(&proof).unwrap();
    draft.normalization = DailyNormalizationDescriptorV1Wire {
        file: "normalization.json".to_owned(),
        sha256: hash(&proof_bytes),
        size_bytes: proof_bytes.len() as u64,
        row_count: proof.documents.len() as u64,
    };
    let manifest = seal_daily_price_manifest_v1(&draft).unwrap();
    let admission = DailyPriceAdmissionV1Wire {
        schema: "owner-beta-daily-etf11-admission-v1".to_owned(),
        content_sha256: manifest.content_sha256.clone(),
        manifest_sha256: hash(&encode_daily_price_manifest_v1(&manifest).unwrap()),
        policy_sha256: policy_hash.clone(),
        parent_admission_sha256: parent,
        base: base(),
        verifier_code_commit: CodeCommit::parse(&"2".repeat(40)).unwrap(),
        normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
        evidence_sha256: proof.evidence_sha256.clone(),
        counts: counts(&manifest.daily_sources, manifest.sessions.len()),
    };
    let pins = DailyPricePinsV1Wire {
        content_sha256: manifest.content_sha256.clone(),
        manifest_sha256: hash(&encode_daily_price_manifest_v1(&manifest).unwrap()),
        admission_sha256: hash(&encode_daily_price_admission_v1(&admission).unwrap()),
        policy_sha256: policy_hash,
    };
    Fixture {
        policy,
        draft,
        manifest,
        proof,
        admission,
        pins,
    }
}

fn rebuild(fixture: &mut Fixture) {
    let policy_hash = hash_daily_price_policy_v1(&fixture.policy).unwrap();
    fixture.draft.policy_sha256 = policy_hash.clone();
    let canonical = seal_daily_price_manifest_v1(&fixture.draft).unwrap();
    fixture.draft.daily_sources = canonical.daily_sources;
    fixture.proof = make_proof(&fixture.draft.daily_sources);
    let proof_bytes = encode_daily_normalization_proof_v1(&fixture.proof).unwrap();
    fixture.draft.normalization.sha256 = hash(&proof_bytes);
    fixture.draft.normalization.size_bytes = proof_bytes.len() as u64;
    fixture.draft.normalization.row_count = fixture.proof.documents.len() as u64;
    fixture.manifest = seal_daily_price_manifest_v1(&fixture.draft).unwrap();
    fixture.admission.content_sha256 = fixture.manifest.content_sha256.clone();
    fixture.admission.manifest_sha256 =
        hash(&encode_daily_price_manifest_v1(&fixture.manifest).unwrap());
    fixture.admission.policy_sha256 = policy_hash.clone();
    fixture.admission.parent_admission_sha256 = fixture.manifest.parent_admission_sha256.clone();
    fixture.admission.base = fixture.manifest.base.clone();
    fixture.admission.evidence_sha256 = fixture.proof.evidence_sha256.clone();
    fixture.admission.counts = counts(
        &fixture.manifest.daily_sources,
        fixture.draft.sessions.len(),
    );
    fixture.pins = DailyPricePinsV1Wire {
        content_sha256: fixture.manifest.content_sha256.clone(),
        manifest_sha256: hash(&encode_daily_price_manifest_v1(&fixture.manifest).unwrap()),
        admission_sha256: hash(&encode_daily_price_admission_v1(&fixture.admission).unwrap()),
        policy_sha256: policy_hash,
    };
}

fn first_dividend_page_key(session: &DailySessionEvidenceV1Wire) -> DailyFileKeyV1Wire {
    let action = session
        .actions
        .iter()
        .find(|action| action.logical_class == DailyActionClassV1::Dividend)
        .unwrap();
    file_key(action.pages.first().unwrap())
}

fn cash_row(session: &DailySessionEvidenceV1Wire, codes: [&str; 3]) -> DailyCashDividendRowV1Wire {
    DailyCashDividendRowV1Wire {
        symbol: "069500".to_owned(),
        record_date: date("2026-08-31"),
        dividend_kind: codes[0].to_owned(),
        face_value: "5000".to_owned(),
        cash_dividend_amount: "100".to_owned(),
        cash_dividend_rate: "2".to_owned(),
        stock_dividend_rate: "0".to_owned(),
        cash_payment_date: Some(date("2026-09-14")),
        stock_payment_date: None,
        odd_lot_payment_date: None,
        stock_kind: codes[1].to_owned(),
        high_dividend_flag: codes[2].to_owned(),
        source_file: first_dividend_page_key(session),
    }
}

fn replace_cash_row_and_rebuild(fixture: &mut Fixture, row: DailyCashDividendRowV1Wire) {
    {
        let session = &mut fixture.draft.daily_sources[0];
        let action = session
            .actions
            .iter_mut()
            .find(|action| action.logical_class == DailyActionClassV1::Dividend)
            .unwrap();
        action.cash_dividend_rows_sha256 =
            hash(&daily_cash_dividend_rows_preimage_v1(std::slice::from_ref(&row)).unwrap());
        action.cash_dividends = vec![row];

        let mut source_files: Vec<_> = session
            .actions
            .iter()
            .flat_map(|action| action.pages.iter().map(file_key))
            .collect();
        source_files.sort_by_key(key_sort);
        let corporate = session
            .normalized_documents
            .iter_mut()
            .find(|document| document.kind == DailyEvidenceKindV1::CorporateActions)
            .unwrap();
        corporate.row_count = 1;
        corporate.source_files = source_files;
        let commitment = DailyNormalizedSourceCommitmentV1Wire {
            schema: "owner-beta-daily-normalized-source-v1".to_owned(),
            kind: corporate.kind,
            session_date: corporate.session_date,
            source_files: corporate.source_files.clone(),
        };
        corporate.source_evidence_sha256 =
            hash(&daily_normalized_source_preimage_v1(&commitment).unwrap());
    }
    rebuild(fixture);
}

fn present_empty_cash_fixture() -> Fixture {
    let mut fixture = fixture(false);
    let row = cash_row(&fixture.draft.daily_sources[0], ["", "", ""]);
    replace_cash_row_and_rebuild(&mut fixture, row);
    fixture
}

fn set_cash_row_code(row: &mut DailyCashDividendRowV1Wire, field: &str, value: &str) {
    match field {
        "dividend_kind" => row.dividend_kind = value.to_owned(),
        "stock_kind" => row.stock_kind = value.to_owned(),
        "high_dividend_flag" => row.high_dividend_flag = value.to_owned(),
        _ => unreachable!(),
    }
}

fn first_manifest_cash_row_mut(
    manifest: &mut DailyPriceManifestV1Wire,
) -> &mut DailyCashDividendRowV1Wire {
    manifest.daily_sources[0]
        .actions
        .iter_mut()
        .find(|action| action.logical_class == DailyActionClassV1::Dividend)
        .unwrap()
        .cash_dividends
        .first_mut()
        .unwrap()
}

fn replace_json_cash_code(bytes: &[u8], field: &str, raw_value: Option<&str>) -> Vec<u8> {
    let source = std::str::from_utf8(bytes).unwrap();
    if let Some(raw_value) = raw_value {
        let needle = format!("\"{field}\":\"\"");
        assert_eq!(source.matches(needle.as_str()).count(), 1);
        let replacement = format!("\"{field}\":{raw_value}");
        source
            .replacen(needle.as_str(), replacement.as_str(), 1)
            .into_bytes()
    } else {
        let needle = format!("\"{field}\":\"\",");
        assert_eq!(source.matches(needle.as_str()).count(), 1);
        source.replacen(needle.as_str(), "", 1).into_bytes()
    }
}

fn assert_hash(actual: &ContentHash, expected: &str) {
    assert_eq!(actual.as_str(), expected);
}

fn assert_json_input_boundary<T>(
    bytes: Vec<u8>,
    limit: usize,
    parse: impl Fn(&[u8]) -> Result<T, DailyWireErrorV1>,
) where
    T: std::fmt::Debug + PartialEq,
{
    assert!(bytes.len() <= limit);
    let mut at_limit = bytes;
    at_limit.resize(limit, b' ');
    assert!(parse(&at_limit).is_ok());
    at_limit.push(b' ');
    assert_eq!(parse(&at_limit), Err(DailyWireErrorV1::LimitExceeded));
}

fn reverse_canonical_sets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if matches!(
                    key.as_str(),
                    "instruments"
                        | "sessions"
                        | "daily_sources"
                        | "source_kinds"
                        | "bars"
                        | "reference"
                        | "calendar"
                        | "actions"
                        | "cash_dividends"
                        | "normalized_documents"
                        | "source_files"
                        | "documents"
                        | "query"
                        | "headers"
                ) && let Some(array) = child.as_array_mut()
                {
                    array.reverse();
                }
                reverse_canonical_sets(child);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                reverse_canonical_sets(child);
            }
        }
        _ => {}
    }
}

fn refresh_corporate_document(session: &mut DailySessionEvidenceV1Wire) {
    let source_files: Vec<_> = session
        .actions
        .iter()
        .flat_map(|action| action.pages.iter().map(file_key))
        .collect();
    let document = session
        .normalized_documents
        .iter_mut()
        .find(|document| document.kind == DailyEvidenceKindV1::CorporateActions)
        .unwrap();
    document.source_files = source_files;
    document.source_files.sort_by_key(key_sort);
    let commitment = DailyNormalizedSourceCommitmentV1Wire {
        schema: "owner-beta-daily-normalized-source-v1".to_owned(),
        kind: document.kind,
        session_date: document.session_date,
        source_files: document.source_files.clone(),
    };
    document.source_evidence_sha256 =
        hash(&daily_normalized_source_preimage_v1(&commitment).unwrap());
}

fn refresh_calendar_documents(session: &mut DailySessionEvidenceV1Wire) {
    for binding in &session.calendar {
        let document = session
            .normalized_documents
            .iter_mut()
            .find(|document| {
                document.kind == DailyEvidenceKindV1::Calendar
                    && document.session_date == binding.civil_date
            })
            .unwrap();
        document.source_files = vec![file_key(&binding.source_file)];
        let commitment = DailyNormalizedSourceCommitmentV1Wire {
            schema: "owner-beta-daily-normalized-source-v1".to_owned(),
            kind: document.kind,
            session_date: document.session_date,
            source_files: document.source_files.clone(),
        };
        document.source_evidence_sha256 =
            hash(&daily_normalized_source_preimage_v1(&commitment).unwrap());
    }
}

fn near_limit_action(index: usize, file_name: String) -> DailyActionChainV1Wire {
    let batch_number = 10_000 + u64::try_from(index).unwrap() * 17;
    let (class, path, tr_id, extra) = action_spec("dividend");
    let mut query = BTreeMap::from([
        ("CTS".to_owned(), String::new()),
        ("F_DT".to_owned(), "20260829".to_owned()),
        ("T_DT".to_owned(), "20260831".to_owned()),
        ("SHT_CD".to_owned(), "069500".to_owned()),
    ]);
    query.extend(extra);
    let mut page = file(FileFixture {
        kind: DailyEvidenceKindV1::CorporateActions,
        name: &file_name,
        provider: "kis",
        batch_id: batch(&format!("00000000-0000-4000-8000-{batch_number:012x}")),
        batch_date: "2026-08-31",
        batch_json_hash: sh(batch_number + 1),
        manifest_line_hash: sh(batch_number + 2),
        request: request(&format!("ksdinfo/{path}"), tr_id, query, ""),
        retrieved_at: "2026-08-31T09:00:00.123456789Z",
        response_continuation: Some(""),
    });
    set_copy_depth(&mut page, 16, batch_number + 1);
    DailyActionChainV1Wire {
        logical_class: class,
        scope: DailyActionScopeV1::Etf11Symbol,
        symbol: Some("069500".to_owned()),
        range_start: date("2026-08-29"),
        range_end: date("2026-08-31"),
        pages: vec![page],
        cash_dividends: Vec::new(),
        cash_dividend_rows_sha256: hash(b"[]"),
    }
}

fn resize_action_filename(action: &mut DailyActionChainV1Wire, length: usize) {
    let file_name = "x".repeat(length);
    let wire = format!("synthetic-wire:{file_name}");
    let wire_sha256 = hash(wire.as_bytes());
    let size_bytes = wire.len() as u64;
    let page = &mut action.pages[0];
    page.file_name = file_name.clone();
    page.wire_sha256 = wire_sha256.clone();
    page.size_bytes = size_bytes;
    for origin in &mut page.copied_from {
        origin.file_name = file_name.clone();
        origin.wire_sha256 = wire_sha256.clone();
        origin.size_bytes = size_bytes;
    }
}

#[test]
fn synthetic_fixture_matches_pinned_bytes_hashes_and_bundle_links() {
    let fixture = fixture(false);
    let policy_bytes = encode_daily_price_policy_v1(&fixture.policy).unwrap();
    let evidence_bytes = daily_price_evidence_preimage_v1(&fixture.manifest.daily_sources).unwrap();
    let proof_bytes = encode_daily_normalization_proof_v1(&fixture.proof).unwrap();
    let content_bytes = daily_price_content_preimage_v1(&fixture.draft).unwrap();
    let manifest_bytes = encode_daily_price_manifest_v1(&fixture.manifest).unwrap();
    let admission_bytes = encode_daily_price_admission_v1(&fixture.admission).unwrap();

    assert_eq!(policy_bytes.len(), 1_827);
    assert_hash(
        &hash(&policy_bytes),
        "sha256:b9189f229d2863922a6fcc3d839c7e12b25521bc0b1ba0c0a9534b3fd565f474",
    );
    assert_eq!(evidence_bytes.len(), 44_176);
    assert_hash(
        &hash(&evidence_bytes),
        "sha256:9bf59d097b59b70fd0978f3ae9afe27d93243af78f251b8ca50d8eb1b10d7b34",
    );
    assert_eq!(proof_bytes.len(), 8_790);
    assert_hash(
        &hash(&proof_bytes),
        "sha256:f1baaf8e73ada15381d59b6936f95463e31dcd8810063024b0fecd09d5e05f59",
    );
    assert_eq!(content_bytes.len(), 45_616);
    assert_hash(
        &hash(&content_bytes),
        "sha256:e8d61163bd89509d012d7a1f84d47748e866c5c69119a1d97afd73755bc4efdf",
    );
    assert_eq!(manifest_bytes.len(), 45_707);
    assert_hash(
        &hash(&manifest_bytes),
        "sha256:2e1e5763e443c5ff7ca2d44e39c15bcd9fd7a8477b4e3d3bbff76a067a392daf",
    );
    assert_eq!(admission_bytes.len(), 1_353);
    assert_hash(
        &hash(&admission_bytes),
        "sha256:3409c138a55d73bd9d354c7e9f08101e4613d96047ced5075d2e54bc826b2654",
    );
    assert_eq!(daily_cash_dividend_rows_preimage_v1(&[]).unwrap(), b"[]");
    assert_eq!(
        hash(b"[]").as_str(),
        "sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
    );
    validate_daily_price_wire_bundle_v1(
        &fixture.policy,
        &fixture.manifest,
        &fixture.proof,
        &fixture.admission,
        &fixture.pins,
    )
    .unwrap();
}

#[test]
fn every_json_input_byte_limit_accepts_exact_size_and_rejects_one_more() {
    let fixture = fixture(false);
    assert_json_input_boundary(
        encode_daily_price_policy_v1(&fixture.policy).unwrap(),
        usize::try_from(fixture.policy.limits.max_policy_bytes).unwrap(),
        parse_daily_price_policy_v1,
    );
    assert_json_input_boundary(
        encode_daily_price_manifest_v1(&fixture.manifest).unwrap(),
        usize::try_from(fixture.policy.limits.max_manifest_bytes).unwrap(),
        parse_daily_price_manifest_v1,
    );
    assert_json_input_boundary(
        encode_daily_price_admission_v1(&fixture.admission).unwrap(),
        usize::try_from(fixture.policy.limits.max_admission_bytes).unwrap(),
        parse_daily_price_admission_v1,
    );
    assert_json_input_boundary(
        encode_daily_normalization_proof_v1(&fixture.proof).unwrap(),
        usize::try_from(fixture.policy.limits.max_normalization_bytes).unwrap(),
        parse_daily_normalization_proof_v1,
    );
}

#[test]
fn legal_proof_canonical_output_crosses_only_the_normalization_byte_bound() {
    fn source_key(index: usize) -> DailyFileKeyV1Wire {
        assert!(index < 1_000_000);
        let prefix = format!("proof-source-{index:06}-");
        let name = format!("{prefix}{}.json", "x".repeat(128 - prefix.len() - 5));
        assert_eq!(name.len(), 128);
        DailyFileKeyV1Wire {
            provider: "kis".to_owned(),
            market: "kr".to_owned(),
            batch_id: batch("00000000-0000-4000-8000-000000000001"),
            file_name: name,
            wire_sha256: sh(1),
        }
    }

    fn refresh_source_commitment(proof: &mut DailyNormalizationProofV1Wire) -> usize {
        let document = &mut proof.documents[0];
        let commitment = DailyNormalizedSourceCommitmentV1Wire {
            schema: "owner-beta-daily-normalized-source-v1".to_owned(),
            kind: document.kind,
            session_date: document.session_date,
            source_files: document.source_files.clone(),
        };
        let bytes = daily_normalized_source_preimage_v1(&commitment).unwrap();
        document.source_evidence_sha256 = hash(&bytes);
        bytes.len()
    }

    let limits = policy().limits;
    let output_limit = usize::try_from(limits.max_normalization_bytes).unwrap();
    assert_eq!(output_limit, 16_777_216);
    let mut proof = DailyNormalizationProofV1Wire {
        schema: "owner-beta-daily-etf11-normalization-v1".to_owned(),
        normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
        evidence_sha256: sh(3),
        documents: vec![DailyNormalizedDocumentV1Wire {
            kind: DailyEvidenceKindV1::Bars,
            session_date: date("2026-08-31"),
            normalizer_version: "daily-raw-reconstruction-v1".to_owned(),
            sha256: sh(1),
            size_bytes: 1,
            row_count: 0,
            source_files: vec![source_key(0)],
            source_evidence_sha256: sh(2),
        }],
    };
    refresh_source_commitment(&mut proof);

    // All identities are distinct by fixed-length ASCII filename. Sorting and
    // refreshed hashes cannot change lengths. Measure the one-key overhead once:
    // bytes(n) = overhead + n * (key_bytes + comma) - final_comma.
    let key_bytes = serde_json::to_vec(&proof.documents[0].source_files[0])
        .unwrap()
        .len();
    let overhead = serde_json::to_vec(&proof).unwrap().len() - key_bytes;
    let stride = key_bytes.checked_add(1).unwrap();
    let count = output_limit
        .checked_sub(overhead)
        .unwrap()
        .checked_add(1)
        .unwrap()
        / stride;
    assert_eq!(
        (key_bytes, overhead, stride, count),
        (313, 533, 314, 53_428)
    );
    assert!(count + 1 < usize::try_from(limits.max_file_evidence_per_session).unwrap());
    assert!(proof.documents.len() < usize::try_from(limits.max_file_evidence_per_session).unwrap());
    assert!(proof.documents[0].size_bytes <= limits.max_normalized_document_bytes);
    proof.documents[0].source_files = (0..count).map(source_key).collect();
    let inner_bytes = refresh_source_commitment(&mut proof);
    assert_eq!(inner_bytes, 16_776_501);
    assert!(inner_bytes < usize::try_from(limits.max_normalized_document_bytes).unwrap());

    let expected_bytes = overhead + count.checked_mul(stride).unwrap() - 1;
    let encoded = encode_daily_normalization_proof_v1(&proof).unwrap();
    assert_eq!(encoded.len(), expected_bytes);
    assert_eq!(encoded.len(), 16_776_924); // 292 bytes below the output limit.
    assert!(encoded.len() < output_limit);
    drop(encoded);

    proof.documents[0].source_files.push(source_key(count));
    let inner_bytes = refresh_source_commitment(&mut proof);
    assert_eq!(inner_bytes, 16_776_815);
    assert!(inner_bytes < usize::try_from(limits.max_normalized_document_bytes).unwrap());
    // This unrestricted serialization is size evidence only. The public encoder
    // must reject this otherwise legal proof at its 16 MiB output writer.
    let raw_bytes = serde_json::to_vec(&proof).unwrap();
    assert_eq!(raw_bytes.len(), expected_bytes + stride);
    assert_eq!(raw_bytes.len(), 16_777_238); // 22 bytes above the output limit.
    assert!(raw_bytes.len() > output_limit);
    drop(raw_bytes);
    assert_eq!(
        encode_daily_normalization_proof_v1(&proof),
        Err(DailyWireErrorV1::LimitExceeded),
    );
}

#[test]
fn bundle_counts_follow_all_manifest_sessions_with_more_than_two_base_dates() {
    let mut fixture = fixture(true);
    fixture.draft.sessions.insert(1, date("2020-01-02"));
    fixture.draft.bars.row_count = 55;
    rebuild(&mut fixture);

    assert_eq!(fixture.manifest.sessions.len(), 5);
    assert_eq!(fixture.admission.counts.session_count, 5);
    assert_eq!(fixture.admission.counts.bar_count, 55);
    assert_eq!(fixture.admission.counts.added_session_count, 2);
    validate_daily_price_wire_bundle_v1(
        &fixture.policy,
        &fixture.manifest,
        &fixture.proof,
        &fixture.admission,
        &fixture.pins,
    )
    .unwrap();

    let mut wrong_counts = fixture.admission.clone();
    wrong_counts.counts.session_count = 4;
    wrong_counts.counts.bar_count = 44;
    assert_eq!(
        validate_daily_price_wire_bundle_v1(
            &fixture.policy,
            &fixture.manifest,
            &fixture.proof,
            &wrong_counts,
            &fixture.pins,
        ),
        Err(DailyWireErrorV1::InconsistentFields),
    );
}

#[test]
fn declared_source_document_bars_and_normalization_sizes_hit_exact_limits() {
    let fixture = fixture(false);
    let limits = &fixture.policy.limits;

    let mut draft = fixture.draft.clone();
    draft.daily_sources[0].bars[0].size_bytes = limits.max_source_file_bytes;
    assert!(daily_price_content_preimage_v1(&draft).is_ok());
    draft.daily_sources[0].bars[0].size_bytes += 1;
    assert_eq!(
        daily_price_content_preimage_v1(&draft),
        Err(DailyWireErrorV1::LimitExceeded),
    );

    let mut draft = fixture.draft.clone();
    draft.daily_sources[0].normalized_documents[0].size_bytes =
        limits.max_normalized_document_bytes;
    assert!(daily_price_content_preimage_v1(&draft).is_ok());
    draft.daily_sources[0].normalized_documents[0].size_bytes += 1;
    assert_eq!(
        daily_price_content_preimage_v1(&draft),
        Err(DailyWireErrorV1::LimitExceeded),
    );

    let mut draft = fixture.draft.clone();
    draft.bars.size_bytes = limits.max_bars_bytes;
    assert!(daily_price_content_preimage_v1(&draft).is_ok());
    draft.bars.size_bytes += 1;
    assert_eq!(
        daily_price_content_preimage_v1(&draft),
        Err(DailyWireErrorV1::LimitExceeded),
    );

    let mut draft = fixture.draft;
    draft.normalization.size_bytes = limits.max_normalization_bytes;
    assert!(daily_price_content_preimage_v1(&draft).is_ok());
    draft.normalization.size_bytes += 1;
    assert_eq!(
        daily_price_content_preimage_v1(&draft),
        Err(DailyWireErrorV1::LimitExceeded),
    );
}

#[test]
fn seal_rejects_a_valid_draft_whose_final_manifest_exceeds_the_byte_limit() {
    let fixture = fixture(false);
    let mut draft = fixture.draft;
    let maximum = usize::try_from(fixture.policy.limits.max_manifest_bytes).unwrap();
    let target_draft_size = maximum - 90;
    let mut estimated_size = serde_json::to_vec(&draft).unwrap().len();
    let maximum_actions = usize::try_from(fixture.policy.limits.max_action_chains_per_session)
        .unwrap()
        - draft.daily_sources[0].actions.len();
    let mut extra_actions = Vec::new();

    while estimated_size <= target_draft_size && extra_actions.len() < maximum_actions {
        let action = near_limit_action(extra_actions.len(), "x".repeat(128));
        let action_size = serde_json::to_vec(&action).unwrap().len();
        let key_size = serde_json::to_vec(&file_key(&action.pages[0]))
            .unwrap()
            .len();
        estimated_size = estimated_size
            .checked_add(action_size + key_size + 2)
            .unwrap();
        extra_actions.push(action);
    }
    assert!(estimated_size > target_draft_size);

    let mut characters_to_remove = (estimated_size - target_draft_size) / 18;
    assert!(characters_to_remove <= extra_actions.len() * 28);
    for action in &mut extra_actions {
        if characters_to_remove == 0 {
            break;
        }
        let reduction = characters_to_remove.min(28);
        resize_action_filename(action, 128 - reduction);
        characters_to_remove -= reduction;
    }
    assert_eq!(characters_to_remove, 0);

    draft.daily_sources[0].actions.extend(extra_actions);
    refresh_corporate_document(&mut draft.daily_sources[0]);
    let preimage = daily_price_content_preimage_v1(&draft).unwrap();
    assert!(preimage.len() <= maximum);
    assert!(preimage.len() > maximum - 91);

    let content_hash = hash(&preimage);
    let mut final_manifest_bytes = preimage[..preimage.len() - 1].to_vec();
    final_manifest_bytes
        .extend_from_slice(format!(",\"content_sha256\":\"{content_hash}\"}}").as_bytes());
    assert_eq!(final_manifest_bytes.len(), preimage.len() + 91);
    assert!(final_manifest_bytes.len() > maximum);
    assert_eq!(
        seal_daily_price_manifest_v1(&draft),
        Err(DailyWireErrorV1::LimitExceeded),
    );
    drop(final_manifest_bytes);
    drop(preimage);

    let mut encodable_draft = draft;
    let action = encodable_draft.daily_sources[0]
        .actions
        .iter_mut()
        .find(|action| {
            action.scope == DailyActionScopeV1::Etf11Symbol
                && action.pages[0].file_name.starts_with("x")
        })
        .unwrap();
    let file_name_length = action.pages[0].file_name.len();
    resize_action_filename(action, file_name_length - 1);
    refresh_corporate_document(&mut encodable_draft.daily_sources[0]);
    let encodable_preimage = daily_price_content_preimage_v1(&encodable_draft).unwrap();
    assert!(encodable_preimage.len() <= maximum - 91);
    let sealed = seal_daily_price_manifest_v1(&encodable_draft).unwrap();
    let final_bytes = encode_daily_price_manifest_v1(&sealed).unwrap();
    assert!(final_bytes.len() <= maximum);
    assert!(final_bytes.len() > maximum - 128);
}

#[test]
fn present_empty_cash_codes_match_separate_golden_and_reseal_bundle() {
    let fixture = present_empty_cash_fixture();
    let session = &fixture.manifest.daily_sources[0];
    let row = &session
        .actions
        .iter()
        .find(|action| action.logical_class == DailyActionClassV1::Dividend)
        .unwrap()
        .cash_dividends[0];
    assert_eq!(row.dividend_kind, "");
    assert_eq!(row.stock_kind, "");
    assert_eq!(row.high_dividend_flag, "");

    let rows = daily_cash_dividend_rows_preimage_v1(std::slice::from_ref(row)).unwrap();
    assert_eq!(rows.len(), 515);
    assert_eq!(rows.as_slice(), PRESENT_EMPTY_CASH_ROWS_JSON.as_bytes());
    assert_eq!(
        hash(&rows).as_str(),
        "sha256:0f87c41ba19ba820ee79a2a05f54c27b9a4af5f185a98b080bc429b56eaca864"
    );
    assert_eq!(fixture.admission.counts.cash_dividend_row_count, 1);
    assert_eq!(
        session
            .normalized_documents
            .iter()
            .find(|document| document.kind == DailyEvidenceKindV1::CorporateActions)
            .unwrap()
            .row_count,
        1
    );

    let manifest_bytes = encode_daily_price_manifest_v1(&fixture.manifest).unwrap();
    assert_eq!(
        parse_daily_price_manifest_v1(&manifest_bytes).unwrap(),
        fixture.manifest
    );
    let proof_bytes = encode_daily_normalization_proof_v1(&fixture.proof).unwrap();
    assert_eq!(
        parse_daily_normalization_proof_v1(&proof_bytes).unwrap(),
        fixture.proof
    );
    let admission_bytes = encode_daily_price_admission_v1(&fixture.admission).unwrap();
    assert_eq!(
        parse_daily_price_admission_v1(&admission_bytes).unwrap(),
        fixture.admission
    );
    let policy_bytes = encode_daily_price_policy_v1(&fixture.policy).unwrap();
    assert_eq!(
        parse_daily_price_policy_v1(&policy_bytes).unwrap(),
        fixture.policy
    );
    validate_daily_price_wire_bundle_v1(
        &fixture.policy,
        &fixture.manifest,
        &fixture.proof,
        &fixture.admission,
        &fixture.pins,
    )
    .unwrap();
}

#[test]
fn each_cash_code_accepts_empty_and_valid_32_byte_values() {
    let empty = present_empty_cash_fixture();
    let empty_manifest = encode_daily_price_manifest_v1(&empty.manifest).unwrap();
    assert_eq!(
        parse_daily_price_manifest_v1(&empty_manifest).unwrap(),
        empty.manifest
    );
    for field in CASH_CODE_FIELDS {
        let mut candidate = fixture(false);
        let mut row = cash_row(&candidate.draft.daily_sources[0], ["", "", ""]);
        let code = "A".repeat(32);
        set_cash_row_code(&mut row, field, &code);
        assert!(daily_cash_dividend_rows_preimage_v1(std::slice::from_ref(&row)).is_ok());
        replace_cash_row_and_rebuild(&mut candidate, row);

        let bytes = encode_daily_price_manifest_v1(&candidate.manifest).unwrap();
        let parsed = parse_daily_price_manifest_v1(&bytes).unwrap();
        let parsed_row = parsed.daily_sources[0]
            .actions
            .iter()
            .find(|action| action.logical_class == DailyActionClassV1::Dividend)
            .unwrap()
            .cash_dividends
            .first()
            .unwrap();
        let parsed_value = match field {
            "dividend_kind" => &parsed_row.dividend_kind,
            "stock_kind" => &parsed_row.stock_kind,
            "high_dividend_flag" => &parsed_row.high_dividend_flag,
            _ => unreachable!(),
        };
        assert_eq!(parsed_value, &code);
        assert_eq!(parsed, candidate.manifest);
        validate_daily_price_wire_bundle_v1(
            &candidate.policy,
            &parsed,
            &candidate.proof,
            &candidate.admission,
            &candidate.pins,
        )
        .unwrap();
    }
}

#[test]
fn cash_code_json_and_typed_paths_return_exact_errors_before_hash_checks() {
    let valid = present_empty_cash_fixture();
    let valid_manifest_bytes = encode_daily_price_manifest_v1(&valid.manifest).unwrap();
    let invalid_values = [
        "A".repeat(33),
        "A B".to_owned(),
        "\n".to_owned(),
        "é".to_owned(),
        ".".to_owned(),
    ];

    for field in CASH_CODE_FIELDS {
        assert_eq!(
            parse_daily_price_manifest_v1(&replace_json_cash_code(
                &valid_manifest_bytes,
                field,
                None,
            )),
            Err(DailyWireErrorV1::MissingField),
            "missing {field}"
        );
        for token in ["null", "false", "1"] {
            assert_eq!(
                parse_daily_price_manifest_v1(&replace_json_cash_code(
                    &valid_manifest_bytes,
                    field,
                    Some(token),
                )),
                Err(DailyWireErrorV1::InvalidType),
                "non-string {field}: {token}"
            );
        }

        for invalid in &invalid_values {
            let json_value = serde_json::to_string(invalid).unwrap();
            assert_eq!(
                parse_daily_price_manifest_v1(&replace_json_cash_code(
                    &valid_manifest_bytes,
                    field,
                    Some(&json_value),
                )),
                Err(DailyWireErrorV1::InvalidLiteral),
                "JSON {field}: {invalid:?}"
            );

            let session = &valid.draft.daily_sources[0];
            let mut row = cash_row(session, ["", "", ""]);
            set_cash_row_code(&mut row, field, invalid);
            assert_eq!(
                daily_cash_dividend_rows_preimage_v1(&[row]),
                Err(DailyWireErrorV1::InvalidLiteral),
                "typed preimage {field}: {invalid:?}"
            );

            let mut manifest = valid.manifest.clone();
            set_cash_row_code(first_manifest_cash_row_mut(&mut manifest), field, invalid);
            assert_eq!(
                encode_daily_price_manifest_v1(&manifest),
                Err(DailyWireErrorV1::InvalidLiteral),
                "typed encoder {field}: {invalid:?}"
            );
        }
    }
}

#[test]
fn policy_parser_canonicalizes_set_arrays_and_keeps_struct_order() {
    let fixture = fixture(false);
    let mut value = serde_json::to_value(&fixture.policy).unwrap();
    value["instruments"].as_array_mut().unwrap().reverse();
    value["source_kinds"].as_array_mut().unwrap().reverse();
    let bytes = serde_json::to_vec(&value).unwrap();
    let parsed = parse_daily_price_policy_v1(&bytes).unwrap();
    assert_eq!(
        encode_daily_price_policy_v1(&parsed).unwrap(),
        encode_daily_price_policy_v1(&fixture.policy).unwrap()
    );

    let mut manifest_value = serde_json::to_value(&fixture.manifest).unwrap();
    reverse_canonical_sets(&mut manifest_value);
    let reordered = serde_json::to_vec(&manifest_value).unwrap();
    let parsed = parse_daily_price_manifest_v1(&reordered).unwrap();
    assert_eq!(
        encode_daily_price_manifest_v1(&parsed).unwrap(),
        encode_daily_price_manifest_v1(&fixture.manifest).unwrap()
    );
}

#[test]
fn parser_rejects_duplicate_unknown_missing_nullable_and_wrong_types() {
    let fixture = fixture(false);
    let bytes = encode_daily_price_manifest_v1(&fixture.manifest).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let duplicate = text.replacen('{', "{\"schema\":\"owner-beta-daily-etf11-input-v1\",", 1);
    assert_eq!(
        parse_daily_price_manifest_v1(duplicate.as_bytes()),
        Err(DailyWireErrorV1::DuplicateKey)
    );
    let escaped_duplicate = text.replacen(
        '{',
        "{\"\\u0073chema\":\"owner-beta-daily-etf11-input-v1\",",
        1,
    );
    assert_eq!(
        parse_daily_price_manifest_v1(escaped_duplicate.as_bytes()),
        Err(DailyWireErrorV1::DuplicateKey)
    );
    let unknown = text.replacen('{', "{\"extra\":0,", 1);
    assert_eq!(
        parse_daily_price_manifest_v1(unknown.as_bytes()),
        Err(DailyWireErrorV1::UnknownField)
    );
    let nested_duplicate = text.replacen(
        "\"market\":\"kr\"",
        "\"market\":\"kr\",\"market\":\"kr\"",
        1,
    );
    assert_eq!(
        parse_daily_price_manifest_v1(nested_duplicate.as_bytes()),
        Err(DailyWireErrorV1::DuplicateKey)
    );
    let nested_unknown =
        text.replacen("\"market\":\"kr\"", "\"market\":\"kr\",\"unexpected\":1", 1);
    assert_eq!(
        parse_daily_price_manifest_v1(nested_unknown.as_bytes()),
        Err(DailyWireErrorV1::UnknownField)
    );
    let missing_nullable = text.replace("\"parent_admission_sha256\":null,", "");
    assert_eq!(
        parse_daily_price_manifest_v1(missing_nullable.as_bytes()),
        Err(DailyWireErrorV1::MissingField)
    );
    let missing_nested_nullable = text.replace(",\"legacy_projection\":null", "");
    assert_eq!(
        parse_daily_price_manifest_v1(missing_nested_nullable.as_bytes()),
        Err(DailyWireErrorV1::MissingField)
    );
    let wrong_type = text.replace("\"strict_pit\":false", "\"strict_pit\":0");
    assert_eq!(
        parse_daily_price_manifest_v1(wrong_type.as_bytes()),
        Err(DailyWireErrorV1::InvalidType)
    );
    let wrong_pair_type = text.replace(
        "[\"FID_COND_MRKT_DIV_CODE\",\"J\"]",
        "[\"FID_COND_MRKT_DIV_CODE\",null]",
    );
    assert_eq!(
        parse_daily_price_manifest_v1(wrong_pair_type.as_bytes()),
        Err(DailyWireErrorV1::InvalidType)
    );
    let bad_hash = text.replace(
        fixture.manifest.content_sha256.as_str(),
        &fixture.manifest.content_sha256.to_string().to_uppercase(),
    );
    assert_eq!(
        parse_daily_price_manifest_v1(bad_hash.as_bytes()),
        Err(DailyWireErrorV1::InvalidHash)
    );
    let bad_uuid = text.replace(
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-0000-0000-000000000000",
    );
    assert_eq!(
        parse_daily_price_manifest_v1(bad_uuid.as_bytes()),
        Err(DailyWireErrorV1::InvalidUuid)
    );
    let bad_date = text.replace(
        "\"range_end\":\"2026-08-31\"",
        "\"range_end\":\"2026-02-30\"",
    );
    assert_eq!(
        parse_daily_price_manifest_v1(bad_date.as_bytes()),
        Err(DailyWireErrorV1::InvalidDate)
    );
    let bad_commit = text.replace(&"1".repeat(40), &"A".repeat(40));
    assert_eq!(
        parse_daily_price_manifest_v1(bad_commit.as_bytes()),
        Err(DailyWireErrorV1::InvalidCommit)
    );
}

#[test]
fn lexical_bounds_timestamp_precision_and_json_limits_are_fail_closed() {
    assert_eq!(
        DailyWireTimestampV1::parse("2026-08-31T09:00:00.123456789Z").and_then(|value| {
            serde_json::to_string(&value).map_err(|_| DailyWireErrorV1::InvalidJson)
        }),
        Ok("\"2026-08-31T09:00:00.123456789Z\"".to_owned())
    );
    assert!(DailyWireTimestampV1::parse("2026-08-31T09:00:00.000000000Z").is_ok());
    for bad in [
        "2026-08-31T09:00:00.123Z",
        "2026-08-31T09:00:00.123456789+00:00",
        "2026-02-30T09:00:00.000000000Z",
        "2026-08-31T09:00:60.000000000Z",
    ] {
        assert_eq!(
            DailyWireTimestampV1::parse(bad),
            Err(DailyWireErrorV1::InvalidTimestamp)
        );
    }

    let fixture = fixture(false);
    let policy_bytes = encode_daily_price_policy_v1(&fixture.policy).unwrap();
    let mut oversized = policy_bytes.clone();
    oversized.extend(std::iter::repeat_n(b' ', 262_145));
    assert_eq!(
        parse_daily_price_policy_v1(&oversized),
        Err(DailyWireErrorV1::LimitExceeded)
    );
    let deep = format!("{}null{}", "[".repeat(33), "]".repeat(33));
    assert_eq!(
        parse_daily_price_policy_v1(deep.as_bytes()),
        Err(DailyWireErrorV1::LimitExceeded)
    );
    let overflow = String::from_utf8(policy_bytes).unwrap().replace(
        "\"max_sessions\":20000",
        "\"max_sessions\":18446744073709551616",
    );
    assert_eq!(
        parse_daily_price_policy_v1(overflow.as_bytes()),
        Err(DailyWireErrorV1::InvalidType)
    );
    let negative_zero = String::from_utf8(encode_daily_price_policy_v1(&fixture.policy).unwrap())
        .unwrap()
        .replace("\"max_sessions\":20000", "\"max_sessions\":-0");
    assert_eq!(
        parse_daily_price_policy_v1(negative_zero.as_bytes()),
        Err(DailyWireErrorV1::InvalidType)
    );
}

#[test]
fn proof_date_typed_and_json_paths_match_domain_year_bounds() {
    for (year, valid) in [(0, false), (1, true), (9999, true), (10_000, false)] {
        let proof = proof_for_calendar_year(year);
        let raw = serde_json::to_vec(&proof).unwrap();
        if valid {
            let encoded = encode_daily_normalization_proof_v1(&proof).unwrap();
            assert_eq!(parse_daily_normalization_proof_v1(&raw).unwrap(), proof);
            assert_eq!(parse_daily_normalization_proof_v1(&encoded).unwrap(), proof);
        } else {
            assert_eq!(
                encode_daily_normalization_proof_v1(&proof),
                Err(DailyWireErrorV1::InvalidDate),
            );
            assert_eq!(
                parse_daily_normalization_proof_v1(&raw),
                Err(DailyWireErrorV1::InvalidDate),
            );
        }
    }
}

#[test]
fn single_page_markers_are_retained_and_legacy_hash_is_independent() {
    let fixture = fixture(false);
    let mut session = fixture.manifest.daily_sources[0].clone();
    assert_eq!(session.bars[0].response_continuation.as_deref(), Some(""));
    session.reference[0].response_continuation = Some("F".to_owned());
    for binding in &mut session.calendar {
        binding.source_file.response_continuation = Some("M".to_owned());
    }
    assert_eq!(
        session.reference[0].response_continuation.as_deref(),
        Some("F")
    );
    assert_eq!(
        session.calendar[0]
            .source_file
            .response_continuation
            .as_deref(),
        Some("M")
    );
    let mut followed_single_page = session.clone();
    for (name, value) in &mut followed_single_page.bars[0].request.headers {
        if name == "tr_cont" {
            *value = "N".to_owned();
        }
    }
    assert!(daily_price_evidence_preimage_v1(&[followed_single_page]).is_err());

    let binding = session
        .calendar
        .iter_mut()
        .find(|binding| binding.civil_date == date("2026-08-31"))
        .unwrap();
    binding.legacy_projection = Some(DailyLegacyCalendarProjectionV1Wire {
        source: "kis".to_owned(),
        source_version: "kis-chk-holiday-v1:schema-1".to_owned(),
        source_batch_id: binding.source_file.batch_id,
        source_file_name: "calendar.json".to_owned(),
        document_sha256: sh(42),
        retrieved_at: binding.source_file.original_retrieved_at,
    });
    assert_ne!(
        binding.normalized_document_sha256,
        binding.legacy_projection.as_ref().unwrap().document_sha256
    );
    assert!(daily_price_evidence_preimage_v1(&[session]).is_ok());
}

#[test]
fn request_interval_etf11_and_normalization_conflicts_are_rejected() {
    let fixture = fixture(false);
    let mut missing_instrument = fixture.manifest.daily_sources[0].clone();
    missing_instrument.bars.pop();
    assert!(daily_price_evidence_preimage_v1(&[missing_instrument]).is_err());

    let mut bad_request = fixture.manifest.daily_sources[0].clone();
    let value = &mut bad_request.bars[0].request.query[0].1;
    *value = "wrong".to_owned();
    assert!(daily_price_evidence_preimage_v1(&[bad_request]).is_err());

    let mut gap = fixture.manifest.daily_sources[0].clone();
    let dividend = gap
        .actions
        .iter_mut()
        .find(|action| action.logical_class == DailyActionClassV1::Dividend)
        .unwrap();
    dividend.range_start = date("2026-08-30");
    for (name, value) in &mut dividend.pages[0].request.query {
        if name == "F_DT" {
            *value = "20260830".to_owned();
        }
    }
    assert!(daily_price_evidence_preimage_v1(&[gap]).is_err());

    let mut bad_commitment = fixture.manifest.daily_sources[0].clone();
    bad_commitment.normalized_documents[0].source_evidence_sha256 = sh(999);
    assert!(daily_price_evidence_preimage_v1(&[bad_commitment]).is_err());
}

#[test]
fn ordered_action_pages_and_copy_hops_are_never_sorted() {
    let fixture = fixture(false);
    let mut session = fixture.manifest.daily_sources[0].clone();
    let action = &mut session.actions[0];
    let mut second = action.pages[0].clone();
    second.file_name = "bonus-issue-page-02.json".to_owned();
    let wire = b"synthetic-wire:bonus-issue-page-02.json";
    second.wire_sha256 = hash(wire);
    second.size_bytes = wire.len() as u64;
    second.response_continuation = Some("F".to_owned());
    for (name, value) in &mut action.pages[0].request.headers {
        if name == "tr_cont" {
            *value = String::new();
        }
    }
    action.pages[0].response_continuation = Some("M".to_owned());
    for (name, value) in &mut second.request.headers {
        if name == "tr_cont" {
            *value = "N".to_owned();
        }
    }
    action.pages.push(second);
    refresh_corporate_document(&mut session);
    assert!(daily_price_evidence_preimage_v1(std::slice::from_ref(&session)).is_ok());
    let mut terminal_more = session.clone();
    terminal_more.actions[0].pages[1].response_continuation = Some("M".to_owned());
    assert!(daily_price_evidence_preimage_v1(&[terminal_more]).is_err());
    session.actions[0].pages.swap(0, 1);
    assert!(daily_price_evidence_preimage_v1(&[session]).is_err());

    let mut copied = fixture.manifest.daily_sources[0].clone();
    let file = &mut copied.calendar[0].source_file;
    let origin = DailyFileOriginV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        batch_date: file.batch_date,
        batch_json_sha256: file.batch_json_sha256.clone(),
        manifest_line_sha256: file.manifest_line_sha256.clone(),
        file_name: file.file_name.clone(),
        kind: file.kind,
        wire_sha256: file.wire_sha256.clone(),
        size_bytes: file.size_bytes,
        request: file.request.clone(),
        response_continuation: file.response_continuation.clone(),
        retrieved_at: file.retrieved_at,
        retrieval_precision: file.retrieval_precision.clone(),
    };
    file.copied_from = vec![origin.clone(), origin];
    assert_eq!(
        daily_price_evidence_preimage_v1(&[copied]),
        Err(DailyWireErrorV1::DuplicateEntry)
    );
}

#[test]
fn ten_ordered_pages_and_sixteen_distinct_copy_hops_hit_their_bounds() {
    let fixture = fixture(false);
    let mut ten_pages = fixture.manifest.daily_sources[0].clone();
    let action = &mut ten_pages.actions[0];
    let first_page = action.pages[0].clone();
    action.pages[0].response_continuation = Some("M".to_owned());
    for page_index in 1..10 {
        let name = format!("bonus-issue-page-{:02}.json", page_index + 1);
        let wire = format!("synthetic-wire:{name}");
        let mut page = first_page.clone();
        page.file_name = name;
        page.wire_sha256 = hash(wire.as_bytes());
        page.size_bytes = wire.len() as u64;
        page.response_continuation = Some(if page_index == 9 { "F" } else { "M" }.to_owned());
        for (header, value) in &mut page.request.headers {
            if header == "tr_cont" {
                *value = "N".to_owned();
            }
        }
        action.pages.push(page);
    }
    refresh_corporate_document(&mut ten_pages);
    assert!(daily_price_evidence_preimage_v1(std::slice::from_ref(&ten_pages)).is_ok());

    let mut eleven_pages = ten_pages;
    let action = &mut eleven_pages.actions[0];
    action.pages[9].response_continuation = Some("M".to_owned());
    let name = "bonus-issue-page-11.json";
    let wire = format!("synthetic-wire:{name}");
    let mut last_page = action.pages[0].clone();
    last_page.file_name = name.to_owned();
    last_page.wire_sha256 = hash(wire.as_bytes());
    last_page.size_bytes = wire.len() as u64;
    last_page.response_continuation = Some("F".to_owned());
    for (header, value) in &mut last_page.request.headers {
        if header == "tr_cont" {
            *value = "N".to_owned();
        }
    }
    action.pages.push(last_page);
    refresh_corporate_document(&mut eleven_pages);
    assert_eq!(
        daily_price_evidence_preimage_v1(&[eleven_pages]),
        Err(DailyWireErrorV1::LimitExceeded),
    );

    let mut sixteen_hops = fixture.manifest.daily_sources[0].clone();
    for binding in &mut sixteen_hops.calendar {
        set_copy_depth(&mut binding.source_file, 16, 3);
    }
    assert!(daily_price_evidence_preimage_v1(std::slice::from_ref(&sixteen_hops)).is_ok());

    let mut seventeen_hops = sixteen_hops;
    for binding in &mut seventeen_hops.calendar {
        set_copy_depth(&mut binding.source_file, 17, 3);
    }
    assert_eq!(
        daily_price_evidence_preimage_v1(&[seventeen_hops]),
        Err(DailyWireErrorV1::LimitExceeded),
    );
}

#[test]
fn copied_calendar_keeps_terminal_request_target_and_changes_source_commitment() {
    let fixture = fixture(false);
    let original_evidence =
        daily_price_evidence_preimage_v1(&fixture.manifest.daily_sources).unwrap();
    let mut session = fixture.manifest.daily_sources[0].clone();
    let original = session.calendar[0].source_file.clone();
    let origin = DailyFileOriginV1Wire {
        provider: original.provider.clone(),
        market: original.market.clone(),
        batch_id: original.batch_id,
        batch_date: original.batch_date,
        batch_json_sha256: original.batch_json_sha256.clone(),
        manifest_line_sha256: original.manifest_line_sha256.clone(),
        file_name: original.file_name.clone(),
        kind: original.kind,
        wire_sha256: original.wire_sha256.clone(),
        size_bytes: original.size_bytes,
        request: original.request.clone(),
        response_continuation: original.response_continuation.clone(),
        retrieved_at: original.retrieved_at,
        retrieval_precision: original.retrieval_precision.clone(),
    };
    let mut copied = original.clone();
    copied.batch_id = batch("00000000-0000-4000-8000-000000000003");
    copied.batch_date = date("2026-09-01");
    copied.batch_json_sha256 = sh(20);
    copied.manifest_line_sha256 = sh(21);
    copied.retrieved_at = ts("2026-09-01T09:00:00.123456789Z");
    copied.copied_from = vec![origin];
    for binding in &mut session.calendar {
        binding.source_file = copied.clone();
    }
    for document in &mut session.normalized_documents {
        if document.kind == DailyEvidenceKindV1::Calendar {
            document.source_files = vec![file_key(&copied)];
            let commitment = DailyNormalizedSourceCommitmentV1Wire {
                schema: "owner-beta-daily-normalized-source-v1".to_owned(),
                kind: document.kind,
                session_date: document.session_date,
                source_files: document.source_files.clone(),
            };
            document.source_evidence_sha256 =
                hash(&daily_normalized_source_preimage_v1(&commitment).unwrap());
        }
    }
    let copied_evidence = daily_price_evidence_preimage_v1(std::slice::from_ref(&session)).unwrap();
    assert_ne!(hash(&copied_evidence), hash(&original_evidence));

    let mut wrong_terminal_target = session;
    for binding in &mut wrong_terminal_target.calendar {
        for (name, value) in &mut binding.source_file.request.query {
            if name == "BASS_DT" {
                *value = "20260901".to_owned();
            }
        }
        for origin in &mut binding.source_file.copied_from {
            for (name, value) in &mut origin.request.query {
                if name == "BASS_DT" {
                    *value = "20260901".to_owned();
                }
            }
        }
    }
    assert!(daily_price_evidence_preimage_v1(&[wrong_terminal_target]).is_err());
}

#[test]
fn multi_hop_copy_order_compares_each_nanosecond() {
    let fixture = fixture(false);
    let mut ordered = fixture.manifest.daily_sources[0].clone();
    for binding in &mut ordered.calendar {
        set_copy_depth(&mut binding.source_file, 2, 3);
    }
    refresh_calendar_documents(&mut ordered);
    assert!(daily_price_evidence_preimage_v1(std::slice::from_ref(&ordered)).is_ok());

    let mut inverted = ordered;
    for binding in &mut inverted.calendar {
        binding.source_file.copied_from[0].retrieved_at = ts("2026-08-31T09:00:00.123456786Z");
    }
    assert_eq!(
        daily_price_evidence_preimage_v1(&[inverted]),
        Err(DailyWireErrorV1::InvalidOrder),
    );
}

#[test]
fn page_copy_and_count_guards_fail_before_claims_are_hashed() {
    let fixture = fixture(false);
    let mut too_many_pages = fixture.manifest.daily_sources[0].clone();
    let pages = &mut too_many_pages.actions[0].pages;
    let repeated_page = pages[0].clone();
    pages.resize(11, repeated_page);
    assert_eq!(
        daily_price_evidence_preimage_v1(&[too_many_pages]),
        Err(DailyWireErrorV1::LimitExceeded)
    );

    let mut too_many_copies = fixture.manifest.daily_sources[0].clone();
    let file = &mut too_many_copies.calendar[0].source_file;
    let origin = DailyFileOriginV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        batch_date: file.batch_date,
        batch_json_sha256: file.batch_json_sha256.clone(),
        manifest_line_sha256: file.manifest_line_sha256.clone(),
        file_name: file.file_name.clone(),
        kind: file.kind,
        wire_sha256: file.wire_sha256.clone(),
        size_bytes: file.size_bytes,
        request: file.request.clone(),
        response_continuation: file.response_continuation.clone(),
        retrieved_at: file.retrieved_at,
        retrieval_precision: file.retrieval_precision.clone(),
    };
    file.copied_from = vec![origin; 17];
    assert_eq!(
        daily_price_evidence_preimage_v1(&[too_many_copies]),
        Err(DailyWireErrorV1::LimitExceeded)
    );

    let mut too_many_sessions = fixture.admission.clone();
    too_many_sessions.counts.session_count = 20_001;
    too_many_sessions.counts.bar_count = 220_011;
    assert_eq!(
        encode_daily_price_admission_v1(&too_many_sessions),
        Err(DailyWireErrorV1::LimitExceeded)
    );
}

#[test]
fn admission_aggregate_counts_accept_each_upper_bound_and_reject_plus_one() {
    let mut admission = fixture(false).admission;
    let limits = policy().limits;
    let added = limits.max_sessions - 2;
    let chains = limits.max_action_chains_per_session * added;
    admission.parent_admission_sha256 = Some(sh(77));
    admission.counts.session_count = limits.max_sessions;
    admission.counts.bar_count = limits.max_sessions * 11;
    admission.counts.added_session_count = added;
    admission.counts.calendar_row_count = limits.max_civil_dates_per_addition * added;
    admission.counts.action_chain_count = chains;
    admission.counts.action_page_count = limits.max_pages_per_action_chain * chains;
    admission.counts.source_file_count = limits.max_file_evidence_per_session * added;
    admission.counts.cash_dividend_row_count = limits.max_cash_rows_per_chain * chains;
    admission.counts.normalized_document_count = added * 3 + admission.counts.calendar_row_count;
    assert!(encode_daily_price_admission_v1(&admission).is_ok());

    let mut over = admission.clone();
    over.counts.calendar_row_count += 1;
    assert_eq!(
        encode_daily_price_admission_v1(&over),
        Err(DailyWireErrorV1::LimitExceeded),
    );
    let mut over = admission.clone();
    over.counts.action_chain_count += 1;
    assert_eq!(
        encode_daily_price_admission_v1(&over),
        Err(DailyWireErrorV1::LimitExceeded),
    );
    let mut over = admission.clone();
    over.counts.action_page_count += 1;
    assert_eq!(
        encode_daily_price_admission_v1(&over),
        Err(DailyWireErrorV1::LimitExceeded),
    );
    let mut over = admission.clone();
    over.counts.source_file_count += 1;
    assert_eq!(
        encode_daily_price_admission_v1(&over),
        Err(DailyWireErrorV1::LimitExceeded),
    );
    let mut over = admission;
    over.counts.cash_dividend_row_count += 1;
    assert_eq!(
        encode_daily_price_admission_v1(&over),
        Err(DailyWireErrorV1::LimitExceeded),
    );
}

#[test]
fn overlapping_cash_witnesses_can_outnumber_selected_normalized_rows() {
    let mut fixture = fixture(false);
    let mut session = fixture.manifest.daily_sources[0].clone();
    let dividend_index = session
        .actions
        .iter()
        .position(|action| action.logical_class == DailyActionClassV1::Dividend)
        .unwrap();
    let mut second_chain = session.actions[dividend_index].clone();
    let first_page = &mut second_chain.pages[0];
    first_page.file_name = "dividend-page-overlap.json".to_owned();
    let wire = b"synthetic-wire:dividend-page-overlap.json";
    first_page.wire_sha256 = hash(wire);
    first_page.size_bytes = wire.len() as u64;
    let witness = |source_file| DailyCashDividendRowV1Wire {
        symbol: "069500".to_owned(),
        record_date: date("2026-08-30"),
        dividend_kind: "CASH".to_owned(),
        face_value: "5".to_owned(),
        cash_dividend_amount: "1".to_owned(),
        cash_dividend_rate: "0.2".to_owned(),
        stock_dividend_rate: "0".to_owned(),
        cash_payment_date: None,
        stock_payment_date: None,
        odd_lot_payment_date: None,
        stock_kind: "COMMON".to_owned(),
        high_dividend_flag: "N".to_owned(),
        source_file,
    };
    let mut bad_decimal = witness(file_key(&session.actions[dividend_index].pages[0]));
    bad_decimal.face_value = "1.00".to_owned();
    assert_eq!(
        daily_cash_dividend_rows_preimage_v1(&[bad_decimal]),
        Err(DailyWireErrorV1::InvalidDecimal)
    );
    let first_source = file_key(&session.actions[dividend_index].pages[0]);
    session.actions[dividend_index].cash_dividends = vec![witness(first_source)];
    session.actions[dividend_index].cash_dividend_rows_sha256 = hash(
        &daily_cash_dividend_rows_preimage_v1(&session.actions[dividend_index].cash_dividends)
            .unwrap(),
    );
    second_chain.cash_dividends = vec![witness(file_key(&second_chain.pages[0]))];
    second_chain.cash_dividend_rows_sha256 =
        hash(&daily_cash_dividend_rows_preimage_v1(&second_chain.cash_dividends).unwrap());
    session.actions.push(second_chain);

    let expected_action_files: Vec<_> = session
        .actions
        .iter()
        .flat_map(|action| action.pages.iter().map(file_key))
        .collect();
    let corporate = session
        .normalized_documents
        .iter_mut()
        .find(|document| document.kind == DailyEvidenceKindV1::CorporateActions)
        .unwrap();
    corporate.row_count = 1;
    corporate.source_files = expected_action_files;
    corporate.source_files.sort_by_key(key_sort);
    let commitment = DailyNormalizedSourceCommitmentV1Wire {
        schema: "owner-beta-daily-normalized-source-v1".to_owned(),
        kind: corporate.kind,
        session_date: corporate.session_date,
        source_files: corporate.source_files.clone(),
    };
    corporate.source_evidence_sha256 =
        hash(&daily_normalized_source_preimage_v1(&commitment).unwrap());
    fixture.draft.daily_sources = vec![session];
    fixture.draft.normalization.row_count = 6;
    assert!(daily_price_content_preimage_v1(&fixture.draft).is_ok());
}

#[test]
fn opaque_external_hashes_may_be_self_consistent_without_trust() {
    let mut fake = fixture(false);
    let original_content = fake.manifest.content_sha256.clone();
    fake.draft.daily_sources[0]
        .normalized_documents
        .iter_mut()
        .find(|document| document.kind == DailyEvidenceKindV1::Bars)
        .unwrap()
        .sha256 = sh(777);
    rebuild(&mut fake);
    assert_ne!(fake.manifest.content_sha256, original_content);
    validate_daily_price_wire_bundle_v1(
        &fake.policy,
        &fake.manifest,
        &fake.proof,
        &fake.admission,
        &fake.pins,
    )
    .unwrap();
}

#[test]
fn parent_policy_code_and_source_commitments_change_only_descendants() {
    let mut parent = fixture(true);
    let original = parent.clone();
    parent.draft.parent_admission_sha256 = Some(sh(99));
    rebuild(&mut parent);
    assert_eq!(
        hash_daily_price_policy_v1(&parent.policy).unwrap(),
        hash_daily_price_policy_v1(&original.policy).unwrap()
    );
    assert_eq!(parent.proof, original.proof);
    assert_ne!(
        parent.manifest.content_sha256,
        original.manifest.content_sha256
    );
    assert_ne!(
        hash_daily_price_manifest_v1(&parent.manifest).unwrap(),
        hash_daily_price_manifest_v1(&original.manifest).unwrap()
    );
    assert_ne!(
        hash_daily_price_admission_v1(&parent.admission).unwrap(),
        hash_daily_price_admission_v1(&original.admission).unwrap()
    );
    validate_daily_price_wire_bundle_v1(
        &parent.policy,
        &parent.manifest,
        &parent.proof,
        &parent.admission,
        &parent.pins,
    )
    .unwrap();

    let mut policy_pin = fixture(false);
    let original = policy_pin.clone();
    policy_pin.policy.base.pins.candidate_content_sha256 = sh(101);
    policy_pin.draft.base = policy_pin.policy.base.clone();
    rebuild(&mut policy_pin);
    assert_ne!(
        hash_daily_price_policy_v1(&policy_pin.policy).unwrap(),
        hash_daily_price_policy_v1(&original.policy).unwrap()
    );
    assert_eq!(policy_pin.proof, original.proof);
    assert_ne!(
        policy_pin.manifest.content_sha256,
        original.manifest.content_sha256
    );
    assert_ne!(
        hash_daily_price_manifest_v1(&policy_pin.manifest).unwrap(),
        hash_daily_price_manifest_v1(&original.manifest).unwrap()
    );
    assert_ne!(
        hash_daily_price_admission_v1(&policy_pin.admission).unwrap(),
        hash_daily_price_admission_v1(&original.admission).unwrap()
    );
    validate_daily_price_wire_bundle_v1(
        &policy_pin.policy,
        &policy_pin.manifest,
        &policy_pin.proof,
        &policy_pin.admission,
        &policy_pin.pins,
    )
    .unwrap();

    let mut producer = fixture(false);
    let original = producer.clone();
    producer.draft.code_commit = CodeCommit::parse(&"3".repeat(40)).unwrap();
    rebuild(&mut producer);
    assert_eq!(producer.policy, original.policy);
    assert_eq!(producer.proof, original.proof);
    assert_ne!(
        producer.manifest.content_sha256,
        original.manifest.content_sha256
    );
    assert_ne!(
        hash_daily_price_manifest_v1(&producer.manifest).unwrap(),
        hash_daily_price_manifest_v1(&original.manifest).unwrap()
    );
    assert_ne!(
        hash_daily_price_admission_v1(&producer.admission).unwrap(),
        hash_daily_price_admission_v1(&original.admission).unwrap()
    );
    validate_daily_price_wire_bundle_v1(
        &producer.policy,
        &producer.manifest,
        &producer.proof,
        &producer.admission,
        &producer.pins,
    )
    .unwrap();

    let mut source = fixture(false);
    let original = source.clone();
    source.draft.daily_sources[0].bars[0].manifest_line_sha256 = sh(333);
    rebuild(&mut source);
    assert_eq!(source.policy, original.policy);
    assert_ne!(source.proof.evidence_sha256, original.proof.evidence_sha256);
    assert_ne!(source.proof, original.proof);
    assert_ne!(
        source.manifest.content_sha256,
        original.manifest.content_sha256
    );
    assert_ne!(
        hash_daily_price_manifest_v1(&source.manifest).unwrap(),
        hash_daily_price_manifest_v1(&original.manifest).unwrap()
    );
    assert_ne!(
        hash_daily_price_admission_v1(&source.admission).unwrap(),
        hash_daily_price_admission_v1(&original.admission).unwrap()
    );
    validate_daily_price_wire_bundle_v1(
        &source.policy,
        &source.manifest,
        &source.proof,
        &source.admission,
        &source.pins,
    )
    .unwrap();
}

#[test]
fn verifier_commit_only_changes_admission_hash_and_stale_links_fail() {
    let mut candidate = fixture(false);
    let original_manifest_hash = hash_daily_price_manifest_v1(&candidate.manifest).unwrap();
    let original_policy_hash = hash_daily_price_policy_v1(&candidate.policy).unwrap();
    let original_evidence_hash = candidate.proof.evidence_sha256.clone();
    let original_admission_hash = hash_daily_price_admission_v1(&candidate.admission).unwrap();
    candidate.admission.verifier_code_commit = CodeCommit::parse(&"4".repeat(40)).unwrap();
    let admission_hash = hash_daily_price_admission_v1(&candidate.admission).unwrap();
    candidate.pins.admission_sha256 = admission_hash.clone();
    assert_ne!(admission_hash, original_admission_hash);
    assert_eq!(
        hash_daily_price_manifest_v1(&candidate.manifest).unwrap(),
        original_manifest_hash
    );
    assert_eq!(
        hash_daily_price_policy_v1(&candidate.policy).unwrap(),
        original_policy_hash
    );
    assert_eq!(candidate.proof.evidence_sha256, original_evidence_hash);
    validate_daily_price_wire_bundle_v1(
        &candidate.policy,
        &candidate.manifest,
        &candidate.proof,
        &candidate.admission,
        &candidate.pins,
    )
    .unwrap();

    let mut stale = candidate.clone();
    stale.pins.policy_sha256 = sh(404);
    assert_eq!(
        validate_daily_price_wire_bundle_v1(
            &stale.policy,
            &stale.manifest,
            &stale.proof,
            &stale.admission,
            &stale.pins,
        ),
        Err(DailyWireErrorV1::HashMismatch)
    );

    let mut stale_manifest = candidate.clone();
    stale_manifest.manifest.content_sha256 = sh(405);
    assert_eq!(
        encode_daily_price_manifest_v1(&stale_manifest.manifest),
        Err(DailyWireErrorV1::HashMismatch)
    );

    let mut first_addition_parent = fixture(false);
    first_addition_parent.draft.parent_admission_sha256 = Some(sh(406));
    assert_eq!(
        seal_daily_price_manifest_v1(&first_addition_parent.draft),
        Err(DailyWireErrorV1::InconsistentFields)
    );
}
