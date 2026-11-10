//! Native tests are source definitions only until the established hosted suite
//! runs them. Empty protocol/codec fixtures prove framing and inventory, never
//! approval of an empty source_files group or an authenticated V3 cohort.

use super::*;

#[test]
fn held_stream_revisit_does_not_retain_a_second_reference_index() {
    let rows: BTreeMap<String, Vec<(String, Json)>> = REFERENCE_TABLES
        .into_iter()
        .chain(["jobs", "job_attempts"])
        .map(|table| (table.into(), vec![fixture_row(table)]))
        .collect();
    let bytes = stream(&rows);
    let first = decode_references(&mut Cursor::new(&bytes), limits()).unwrap();
    let second = decode_database_stream(
        &mut Cursor::new(&bytes),
        limits(),
        false,
        &mut |_, _, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(first.retained_rows, second.retained_rows);
    assert_eq!(first.retained_bytes, second.retained_bytes);
    assert_eq!(second.retained_rows, 31);
    assert!(second.tables.values().all(BTreeMap::is_empty));
    assert!(second.jobs.is_empty());
    assert!(second.attempts.is_empty());
    assert!(first.tables.values().all(|rows| rows.len() == 1));
}

#[test]
fn nonretaining_stream_revisit_keeps_original_row_and_byte_bounds() {
    let rows = BTreeMap::from([
        ("accounts".into(), vec![fixture_row("accounts")]),
        ("jobs".into(), vec![fixture_row("jobs")]),
    ]);
    let bytes = stream(&rows);
    let first = decode_references(&mut Cursor::new(&bytes), limits()).unwrap();
    for bounded in [
        SourceClosureLimits {
            max_reference_rows: first.retained_rows - 1,
            ..limits()
        },
        SourceClosureLimits {
            max_reference_bytes: first.retained_bytes - 1,
            ..limits()
        },
    ] {
        let result = decode_database_stream(
            &mut Cursor::new(&bytes),
            bounded,
            false,
            &mut |_, _, _, _| Ok(()),
        );
        assert!(matches!(result, Err(SourceClosureError::Limit)));
    }
    let mut trailing = bytes;
    trailing.extend_from_slice(b"[\"end\"]\n");
    assert!(
        decode_database_stream(
            &mut Cursor::new(trailing),
            limits(),
            false,
            &mut |_, _, _, _| Ok(())
        )
        .is_err()
    );
}

// These refusal/cleanup tests deliberately have no file, DB, Source or Root
// authority. They must never be counted as genuine archive execution.
fn refusal_raw_limits() -> crate::daily_price_only::source::ScanLimitValues {
    crate::daily_price_only::source::ScanLimitValues {
        max_manifest_bytes: 1,
        max_manifest_records: 1,
        max_batches: 1,
        max_files_per_batch: 1,
        max_files: 1,
        max_batch_metadata_bytes: 1,
        max_source_file_bytes: 1,
        max_retained_bytes: 1,
        max_open_descriptors: 1,
        max_data_root_bytes: 1,
        max_data_root_components: 1,
        max_metadata_json_depth: 1,
    }
}

#[test]
fn retained_archive_owner_refuses_unbound_invocation_before_any_path_or_callback() {
    let attempt = Attempt::default();
    let invocation = RawInvocation::new(&attempt);
    let mut visited = 0;
    let result = ArchiveOriginalOwner::open_in(
        &invocation,
        &attempt,
        &attempt,
        ArchiveOriginalPaths {
            files_bundle: Path::new("never-open-unbound-codec"),
            database_stream: Path::new("never-open-unbound-stream"),
            materialized: Path::new("never-open-unbound-tree"),
            raw_coordination: Path::new("never-open-unbound-coordination"),
        },
        limits(),
        refusal_raw_limits(),
        || true,
        |_| {
            visited += 1;
            Ok(())
        },
    );
    assert!(matches!(result, Err(SourceClosureError::NativeBinding)));
    assert_eq!(visited, 0);
    assert_eq!(attempt.report().live(), 0);
    assert!(!attempt.check());
}

#[test]
fn retained_archive_owner_refuses_foreign_attempt_before_any_callback() {
    let attempt = Attempt::default();
    let foreign = Attempt::default();
    let invocation = RawInvocation::new(&attempt);
    let mut visited = false;
    let result = ArchiveOriginalOwner::open_in(
        &invocation,
        &attempt,
        &foreign,
        ArchiveOriginalPaths {
            files_bundle: Path::new("x"),
            database_stream: Path::new("y"),
            materialized: Path::new("z"),
            raw_coordination: Path::new("w"),
        },
        limits(),
        refusal_raw_limits(),
        || true,
        |_| {
            visited = true;
            Ok(())
        },
    );
    assert!(matches!(result, Err(SourceClosureError::Custody)));
    assert!(!visited);
    assert!(!attempt.check());
    assert!(!foreign.check());
    assert_eq!(attempt.report().live(), 0);
    assert_eq!(foreign.report().live(), 0);
}

#[test]
fn cleanup_panic_keeps_the_real_attempt_dirty_and_does_not_skip_the_next_owner() {
    let attempt = Attempt::default();
    let mut calls = Vec::new();
    let mut clean = archive_cleanup_step(&attempt, || {
        calls.push(1);
        panic!("bounded cleanup fixture");
    });
    clean &= archive_cleanup_step(&attempt, || {
        calls.push(2);
        true
    });
    assert_eq!(calls, vec![1, 2]);
    assert!(!clean);
    assert!(!attempt.check());
    assert_eq!(attempt.report().live(), 0);
}

#[test]
fn cleanup_refusal_cannot_be_promoted_by_a_later_success() {
    let attempt = Attempt::default();
    let mut calls = 0;
    let mut clean = archive_cleanup_step(&attempt, || {
        calls += 1;
        false
    });
    clean &= archive_cleanup_step(&attempt, || {
        calls += 1;
        true
    });
    assert!(!clean);
    assert_eq!(calls, 2);
}

// Parser fixtures only: these rows establish no database, Source or Root
// authority. Genuine archive execution remains a separate staged requirement.
#[test]
fn full_database_visitor_retains_non_reference_ledger_and_delivery_rows() {
    let names = [
        "cash_ledger",
        "daily_equity",
        "paper_settlement_outbox",
        "notifications",
        "notification_deliveries",
        "owner_beta_paper_session_events",
    ];
    let rows: BTreeMap<String, Vec<(String, Json)>> = names
        .into_iter()
        .map(|table| (table.into(), vec![fixture_row(table)]))
        .collect();
    let mut seen = BTreeSet::new();
    let decoded = decode_references_with_rows(
        &mut Cursor::new(stream(&rows)),
        limits(),
        &mut |group, table, key, bytes| {
            assert!(DATABASE_GROUPS.contains(&group));
            assert_eq!(&parse(bytes).unwrap(), &rows[table][0].1);
            assert_eq!(key, rows[table][0].0);
            assert!(seen.insert(table.to_owned()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(seen, names.into_iter().map(str::to_owned).collect());
    assert_eq!(decoded.retained_rows, 0);
}

#[test]
fn full_database_visitor_error_never_completes_a_reference_index() {
    let rows = BTreeMap::from([("cash_ledger".into(), vec![fixture_row("cash_ledger")])]);
    let mut calls = 0;
    let result = decode_references_with_rows(
        &mut Cursor::new(stream(&rows)),
        limits(),
        &mut |_, _, _, _| {
            calls += 1;
            Err(SourceClosureError::Relationship)
        },
    );
    assert!(matches!(result, Err(SourceClosureError::Relationship)));
    assert_eq!(calls, 1);
}

#[test]
fn visited_database_rows_remain_provisional_until_the_real_stream_eof() {
    let rows = BTreeMap::from([("cash_ledger".into(), vec![fixture_row("cash_ledger")])]);
    let mut bytes = stream(&rows);
    bytes.extend_from_slice(b"[\"end\"]\n");
    let mut visited = 0;
    let result =
        decode_references_with_rows(&mut Cursor::new(bytes), limits(), &mut |_, _, _, _| {
            visited += 1;
            Ok(())
        });
    assert!(result.is_err());
    assert_eq!(visited, 1);
}
use std::io::Cursor;

fn limits() -> SourceClosureLimits {
    SourceClosureLimits {
        max_members: 1024,
        max_manifest_bytes: 1_048_576,
        max_reference_bytes: 4_194_304,
        max_reference_rows: 4096,
        max_open_descriptors: 8192,
        max_file_bytes: 16_777_216,
        max_stream_bytes: 16_777_216,
    }
}
fn value(value: serde_json::Value) -> Json {
    parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn manifest_value(extra: Vec<serde_json::Value>) -> serde_json::Value {
    let mut rows: Vec<_> = CLASSES
        .iter()
        .map(|name| {
            serde_json::json!({
                "path":name,"kind":"directory","uid":0,"gid":0,"mode":448,"size":0,"sha256":null
            })
        })
        .collect();
    rows.extend(extra);
    rows.sort_by_key(|row| {
        row["path"]
            .as_str()
            .unwrap()
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    serde_json::json!({"schema":"lagrange-paper-combined-files-v1","classes":CLASSES,"entries":rows})
}
fn codec_json(value: serde_json::Value) -> Vec<u8> {
    let mut out = Vec::new();
    self::value(value).write(&mut out, true).unwrap();
    out
}
fn file_entry(path: &str, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({"path":path,"kind":"file","uid":0,"gid":0,"mode":384,
        "size":bytes.len(),"sha256":digest(bytes)})
}

#[test]
fn canonical_decimal_unicode_and_type_vectors_are_independent() {
    let row = parse("{\"é\":\"한\",\"x\":true,\"a\":1.00e0,\"n\":null}".as_bytes()).unwrap();
    let exact = "{\"a\":1,\"n\":null,\"x\":true,\"é\":\"한\"}";
    assert_eq!(exact.len(), 36);
    assert_eq!(row.canonical().unwrap(), exact.as_bytes());
    let mut root = FramedRoot::default();
    root.add("t", "k", &row).unwrap();
    let independently_framed = "1:t1:k36:{\"a\":1,\"n\":null,\"x\":true,\"é\":\"한\"}".as_bytes();
    assert_eq!(root.value().sha256, digest(independently_framed));
    assert_eq!(root.value().bytes, independently_framed.len() as u64);
    assert_eq!(root.value().rows, 1);
    for number in ["1", "1.0", "1e0", "0.1e1", "10e-1"] {
        assert_eq!(parse(number.as_bytes()).unwrap().canonical().unwrap(), b"1");
    }
    assert_ne!(parse(b"1").unwrap(), parse(b"\"1\"").unwrap());
    assert_ne!(parse(b"true").unwrap(), parse(b"1").unwrap());
    assert_ne!(parse(b"null").unwrap(), parse(b"\"null\"").unwrap());
    assert!(
        parse(b"0.1").unwrap().serde_value().is_err(),
        "typed readers must not receive rounded f64"
    );
}

#[test]
fn framing_does_not_concatenate_ambiguous_table_keys() {
    let row = parse(b"{}").unwrap();
    let mut first = FramedRoot::default();
    first.add("a", "bc", &row).unwrap();
    let mut second = FramedRoot::default();
    second.add("ab", "c", &row).unwrap();
    assert_eq!(first.value().sha256, digest(b"1:a2:bc2:{}"));
    assert_eq!(second.value().sha256, digest(b"2:ab1:c2:{}"));
    assert_ne!(first.value(), second.value());
    assert_eq!(
        FramedRoot::default().value().sha256,
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn exact_decimal_exponents_and_bounds_never_round() {
    for (wire, canonical) in [
        ("-0.0", "0"),
        ("123.4500e-2", "1.2345"),
        ("1e3", "1000"),
        ("1e-4", "0.0001"),
        ("100.01e1", "1000.1"),
        ("9007199254740993", "9007199254740993"),
    ] {
        assert_eq!(
            parse(wire.as_bytes()).unwrap().canonical().unwrap(),
            canonical.as_bytes()
        );
    }
    for invalid in [
        "NaN",
        "Infinity",
        "01",
        "1.",
        "+1",
        ".1",
        "1e",
        "1e+",
        "--1",
        "1e9223372036854775807",
        "0e9223372036854775807",
        "1e131074",
        "1e-131074",
    ] {
        assert!(parse(invalid.as_bytes()).is_err(), "invalid numeric vector");
    }
}

#[test]
fn parser_rejects_duplicates_escaped_aliases_truncation_and_depth() {
    for bytes in [
        b"{\"x\":1,\"x\":2}".as_slice(),
        b"{\"x\":1,\"\\u0078\":2}",
        b"[1,]",
        b"{\"x\":}",
        b"\"\\ud800\"",
        b"\"unterminated",
        b"{}true",
        b"\xef\xbb\xbf{}",
        b"{\"x\":\xff}",
    ] {
        assert!(parse(bytes).is_err());
    }
    let allowed = format!("{}0{}", "[".repeat(64), "]".repeat(64));
    assert!(parse(allowed.as_bytes()).is_ok());
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(parse(deep.as_bytes()).is_err());
    let quoted = parse(br#"{"x":"[\\\"{]"}"#).unwrap();
    assert!(quoted.object().unwrap().contains_key("x"));
}

#[test]
fn aggregate_json_budget_counts_decimal_expansion_keys_and_nodes() {
    assert_eq!(
        parse_bounded(b"1e3", 256).unwrap().canonical().unwrap(),
        b"1000"
    );
    assert_eq!(
        parse_bounded(b"1e1000", 256),
        Err(SourceClosureError::Limit)
    );
    assert_eq!(
        parse_bounded(b"[null,null]", 256),
        Err(SourceClosureError::Limit)
    );
    assert_eq!(
        parse_bounded(br#"{"a":0,"b":0}"#, 512),
        Err(SourceClosureError::Limit)
    );
    assert!(parse_bounded(br#"{"a":0,"b":0}"#, 1024).is_ok());
    assert!(parse_bounded(b"null", 0).is_err());
}

#[test]
fn all_twenty_classes_and_unicode_codec_bytes_are_exact() {
    let bytes = codec_json(manifest_value(vec![file_entry(
        "paper_config/한.json",
        b"{}",
    )]));
    assert!(bytes.windows(b"\\ud55c".len()).any(|w| w == b"\\ud55c"));
    let catalog = manifest(&bytes, limits()).unwrap();
    assert_eq!(catalog.entries.len(), 21);
    assert_eq!(catalog.by_path.len(), 21);
    assert_eq!(
        hex_path("paper_config/한.json"),
        "70617065725f636f6e6669672fed959c2e6a736f6e"
    );
    assert_eq!(
        catalog.entries[catalog.by_path["paper_config/한.json"]].size,
        2
    );
}

#[test]
fn manifest_rejects_unknown_class_missing_empty_class_and_extra_fields() {
    let valid = manifest_value(Vec::new());
    for mutation in 0..5 {
        let mut bad = valid.clone();
        match mutation {
            0 => {
                bad["entries"].as_array_mut().unwrap().pop();
            }
            1 => {
                bad["classes"][0] = "caller_class".into();
            }
            2 => {
                bad["entries"][0]["claimed_verified"] = true.into();
            }
            3 => {
                bad["entries"][0]["kind"] = "file".into();
            }
            _ => {
                bad["schema"] = "lagrange-daily-files-v1".into();
            }
        }
        assert!(manifest(&codec_json(bad), limits()).is_err());
    }
}

#[test]
fn manifest_rejects_order_duplicate_parent_traversal_credentials_and_metadata() {
    for path in [
        "raw/../secret",
        "raw//x",
        "raw/./x",
        "raw/a\\b",
        "raw/commit.lock",
        "raw/x.sock",
        "raw/a.env",
        "raw/password",
        "raw/.pgpass.old",
        "raw/unlisted-parent/x",
    ] {
        assert!(
            manifest(
                &codec_json(manifest_value(vec![file_entry(path, b"x")])),
                limits()
            )
            .is_err()
        );
    }
    let mut bad = manifest_value(vec![file_entry("paper_config/x", b"x")]);
    bad["entries"].as_array_mut().unwrap().swap(0, 1);
    assert!(manifest(&codec_json(bad), limits()).is_err());
    let mut bad = manifest_value(Vec::new());
    let repeated = bad["entries"][0].clone();
    bad["entries"].as_array_mut().unwrap().insert(1, repeated);
    assert!(manifest(&codec_json(bad), limits()).is_err());
    for (field, wrong) in [
        ("uid", 1000),
        ("gid", 999),
        ("mode", 511),
        ("mode", 2541),
        ("size", -1),
    ] {
        let mut bad = manifest_value(Vec::new());
        bad["entries"][0][field] = wrong.into();
        assert!(manifest(&codec_json(bad), limits()).is_err());
    }
}

fn fixture_row(table: &str) -> (String, Json) {
    let p = parse(PROJECTION).unwrap();
    let a = p.get("archive_audit_v1").unwrap();
    let columns = a
        .get("archive_table_columns")
        .unwrap()
        .get(table)
        .unwrap()
        .array()
        .unwrap();
    let mut row: BTreeMap<String, Json> = columns
        .iter()
        .map(|c| (c.text().unwrap().to_owned(), Json::Null))
        .collect();
    let keys = a
        .get("archive_identity_key_components")
        .unwrap()
        .get(table)
        .unwrap()
        .array()
        .unwrap();
    let mut identity = Vec::new();
    for key in keys {
        let name = key.text().unwrap();
        let kind = a
            .get("archive_column_types")
            .unwrap()
            .get(table)
            .unwrap()
            .get(name)
            .unwrap()
            .text()
            .unwrap();
        let item = match kind {
            "uuid" => Json::String("00000000-0000-4000-8000-000000000001".into()),
            "date" => Json::String("2026-11-06".into()),
            "int2" | "int4" | "int8" => Json::Number("1".into()),
            _ => Json::String("a".into()),
        };
        identity.push(match &item {
            Json::String(v) | Json::Number(v) => v.clone(),
            _ => unreachable!(),
        });
        row.insert(name.into(), item);
    }
    (identity.join(":"), Json::Object(row))
}
fn stream(rows: &BTreeMap<String, Vec<(String, Json)>>) -> Vec<u8> {
    let p = parse(PROJECTION).unwrap();
    let a = p.get("archive_audit_v1").unwrap();
    let mut bytes = Vec::new();
    let mut append = |row: Json| {
        bytes.extend(row.canonical().unwrap());
        bytes.push(b'\n');
    };
    append(value(serde_json::json!([
        "header",
        "owner-beta-paper-db-stream-v1",
        digest(PROJECTION)
    ])));
    for (group, tables) in a
        .get("paper_source_table_projection")
        .unwrap()
        .object()
        .unwrap()
    {
        for table in tables.array().unwrap() {
            let table = table.text().unwrap();
            if let Some(records) = rows.get(table) {
                for (key, row) in records {
                    append(Json::Array(vec![
                        Json::String("row".into()),
                        Json::String(group.clone()),
                        Json::String(table.into()),
                        Json::String(key.clone()),
                        row.clone(),
                    ]));
                }
            }
            append(value(serde_json::json!(["table_end", group, table])));
        }
        append(value(serde_json::json!(["group_end", group])));
    }
    for table in REFERENCE_TABLES {
        if let Some(records) = rows.get(table) {
            for (key, row) in records {
                append(Json::Array(vec![
                    Json::String("reference".into()),
                    Json::String(table.into()),
                    Json::String(key.clone()),
                    row.clone(),
                ]));
            }
        }
    }
    append(value(serde_json::json!(["references_end"])));
    append(value(serde_json::json!(["end"])));
    bytes
}

#[test]
fn decoder_visits_all_twenty_nine_reference_sets_and_jobs_attempts() {
    let rows: BTreeMap<_, _> = REFERENCE_TABLES
        .into_iter()
        .chain(["jobs", "job_attempts"])
        .map(|table| (table.into(), vec![fixture_row(table)]))
        .collect();
    let bytes = stream(&rows);
    let decoded = decode_references(&mut Cursor::new(bytes), limits()).unwrap();
    assert_eq!(decoded.tables.len(), 29);
    assert!(decoded.tables.values().all(|rows| rows.len() == 1));
    assert_eq!(decoded.jobs.len(), 1);
    assert_eq!(decoded.attempts.len(), 1);
    assert_eq!(decoded.retained_rows, 31);
}

#[test]
fn decoder_requires_explicit_empty_tables_all_groups_end_and_transport_eof() {
    let bytes = stream(&BTreeMap::new());
    assert_eq!(
        decode_references(&mut Cursor::new(&bytes), limits())
            .unwrap()
            .tables
            .len(),
        29
    );
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && i + 1 < bytes.len() {
            assert!(decode_references(&mut Cursor::new(&bytes[..i + 1]), limits()).is_err());
        }
    }
    let mut extra = bytes.clone();
    extra.extend_from_slice(b"[\"end\"]\n");
    assert!(decode_references(&mut Cursor::new(extra), limits()).is_err());
    assert!(decode_references(&mut Cursor::new(&bytes[..bytes.len() - 1]), limits()).is_err());
}

#[test]
fn decoder_reference_removal_replacement_order_and_unknown_columns_fail() {
    let rows = BTreeMap::from([("accounts".into(), vec![fixture_row("accounts")])]);
    let bytes = stream(&rows);
    let text = String::from_utf8(bytes.clone()).unwrap();
    let removed: String = text
        .lines()
        .filter(|l| !l.starts_with("[\"reference\",\"accounts\""))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(decode_references(&mut Cursor::new(removed), limits()).is_err());
    let mut altered = rows.clone();
    if let Json::Object(row) = &mut altered.get_mut("accounts").unwrap()[0].1 {
        row.insert("extra".into(), Json::Bool(true));
    }
    assert!(decode_references(&mut Cursor::new(stream(&altered)), limits()).is_err());
    let mut duplicated = rows.clone();
    duplicated
        .get_mut("accounts")
        .unwrap()
        .push(fixture_row("accounts"));
    assert!(decode_references(&mut Cursor::new(stream(&duplicated)), limits()).is_err());
    let changed = text.replacen(
        "[\"reference\",\"accounts\"",
        "[\"reference\",\"unknown\"",
        1,
    );
    assert!(decode_references(&mut Cursor::new(changed), limits()).is_err());
    let mut bounded = limits();
    bounded.max_reference_bytes = 1;
    assert!(decode_references(&mut Cursor::new(bytes), bounded).is_err());
}

#[test]
fn exact_receipts_and_all_null_history_preserve_original_strings() {
    let request = r#"{"args":{},"kind":"BEGIN"}"#;
    let result = r#"{"outcome":"OK"}"#;
    let mut row = value(
        serde_json::json!({"request_canonical":request,"request_sha256":digest(request.as_bytes()),
        "result_canonical":result,"result_sha256":digest(result.as_bytes())}),
    );
    receipt(&row).unwrap();
    if let Json::Object(v) = &mut row {
        v.insert(
            "request_canonical".into(),
            Json::String("{\"kind\":\"BEGIN\",\"args\":{}}".into()),
        );
    }
    assert!(receipt(&row).is_err());
    let null = value(
        serde_json::json!({"request_canonical":null,"request_sha256":null,"result_canonical":null,"result_sha256":null}),
    );
    receipt(&null).unwrap();
    let mut mixed = null;
    if let Json::Object(v) = &mut mixed {
        v.insert("request_sha256".into(), Json::String(digest(b"x")));
    }
    assert!(receipt(&mixed).is_err());
}

#[test]
fn source_diagnostics_do_not_disclose_rows_paths_or_provider_prose() {
    let p = parse(PROJECTION).unwrap();
    let a = p.get("archive_audit_v1").unwrap();
    let (key, mut row) = fixture_row("jobs");
    let types = a
        .get("archive_column_types")
        .unwrap()
        .get("jobs")
        .unwrap()
        .object()
        .unwrap();
    assert!(types.contains_key("error_message"));
    if let Json::Object(v) = &mut row {
        v.insert(
            "error_message".into(),
            Json::String("private provider prose".into()),
        );
    }
    let error = validate_row("jobs", &key, &row, a).unwrap_err();
    assert!(!error.to_string().contains("private"));
    assert!(!error.to_string().contains("provider"));
    assert_eq!(
        SourceClosureError::HistoricalV2RegistryMissing.to_string(),
        "ARCHIVE_SOURCE_V2_REGISTRY_MISSING"
    );
}

#[test]
fn native_invocation_and_foreign_attempt_refuse_before_opening_paths() {
    let attempt = Attempt::default();
    let invocation = RawInvocation::new(&attempt);
    assert_eq!(
        native_scope(&invocation, &attempt, &attempt),
        Err(SourceClosureError::NativeBinding)
    );
    let foreign = Attempt::default();
    assert_eq!(
        native_scope(&invocation, &attempt, &foreign),
        Err(SourceClosureError::Custody)
    );
    assert!(!attempt.check());
    assert!(!foreign.check());
}

#[test]
fn zero_overflow_and_unbounded_budgets_are_refused() {
    let mut bad = limits();
    bad.max_members = 19;
    assert!(bad.validate().is_err());
    let mut bad = limits();
    bad.max_manifest_bytes = usize::MAX;
    assert!(bad.validate().is_err());
    let mut bad = limits();
    bad.max_open_descriptors = 0;
    assert!(bad.validate().is_err());
    let mut bad = limits();
    bad.max_stream_bytes = u64::MAX;
    assert!(bad.validate().is_err());
    let mut root = FramedRoot {
        rows: MAX_ROWS,
        ..FramedRoot::default()
    };
    assert!(root.add("x", "y", &Json::Null).is_err());
    let mut root = FramedRoot {
        bytes: i64::MAX as u64,
        ..FramedRoot::default()
    };
    assert!(root.add("x", "y", &Json::Null).is_err());
}

fn reference_vector(table: &str, fields: serde_json::Value) -> (String, Json) {
    let (_, mut row) = fixture_row(table);
    let Json::Object(ref mut object) = row else {
        unreachable!()
    };
    for (key, item) in value(fields).object().unwrap() {
        object.insert(key.clone(), item.clone());
    }
    let projection = parse(PROJECTION).unwrap();
    let audit = projection.get("archive_audit_v1").unwrap();
    let key = audit
        .get("archive_identity_key_components")
        .unwrap()
        .get(table)
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|field| match row.get(field.text().unwrap()).unwrap() {
            Json::String(v) | Json::Number(v) => v.clone(),
            _ => panic!("non-scalar fixture identity"),
        })
        .collect::<Vec<_>>()
        .join(":");
    validate_row(table, &key, &row, audit).unwrap();
    (key, row)
}

// Exact projection-shaped SQL vectors exercise joins only. They are not
// substitute approved artifacts, a whole domain closure or a source_files
// positive. Actual original payloads remain required by the consumer.
fn source_run_reference_vector() -> BTreeMap<String, Vec<(String, Json)>> {
    let owner = "00000000-0000-4000-8000-000000000002";
    let job = "00000000-0000-4000-8000-000000000003";
    let run = "00000000-0000-4000-8000-000000000004";
    let config = "00000000-0000-4000-8000-000000000005";
    let attempt = "00000000-0000-4000-8000-000000000006";
    let slot = digest(b"graph-vector-slot");
    let admission = digest(b"graph-vector-admission");
    let policy = digest(b"graph-vector-policy");
    [
        ("owner_beta_daily_admissions", serde_json::json!({"admission_sha256":admission,
            "owner_user_id":owner,"policy_sha256":policy,"range_end":"2026-11-06"})),
        ("owner_beta_paper_source_slots", serde_json::json!({"slot_id":slot,"owner_user_id":owner,
            "policy_sha256":policy,"session_date":"2026-11-06","activation_epoch":7})),
        ("owner_beta_paper_source_admissions", serde_json::json!({"slot_id":slot,"admission_sha256":admission})),
        ("owner_beta_paper_source_runs", serde_json::json!({"slot_id":slot,"run_id":run,"job_id":job,"attempt_id":attempt})),
        ("owner_beta_recommendation_runs", serde_json::json!({"id":run,"owner_user_id":owner,
            "job_id":job,"strategy_config_id":config,"as_of":"2026-11-06"})),
        ("owner_beta_daily_reservations", serde_json::json!({"owner_user_id":owner,
            "strategy_config_id":config,"session_date":"2026-11-06","trigger_version":"owner-beta-daily-v1",
            "run_id":run,"job_id":job,"admission_sha256":admission,"activation_epoch":7})),
        ("owner_beta_daily_publication_claims", serde_json::json!({"run_id":run,"job_id":job,
            "owner_user_id":owner,"attempt_id":attempt,"attempt_no":2,"claimed_by":"fixture-worker"})),
        ("jobs", serde_json::json!({"id":job,"owner_user_id":owner})),
        ("job_attempts", serde_json::json!({"id":attempt,"job_id":job,"attempt_no":2,"claimed_by":"fixture-worker"})),
    ].into_iter().map(|(table, fields)| (table.to_owned(), vec![reference_vector(table, fields)])).collect()
}

#[test]
fn source_run_joins_require_the_entire_same_owner_admission_reservation_attempt_chain() {
    let rows = source_run_reference_vector();
    let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
    relationships(&refs).unwrap();
    for table in [
        "owner_beta_daily_admissions",
        "owner_beta_paper_source_slots",
        "owner_beta_paper_source_admissions",
        "owner_beta_recommendation_runs",
        "owner_beta_daily_reservations",
        "owner_beta_daily_publication_claims",
        "jobs",
        "job_attempts",
    ] {
        let mut missing = rows.clone();
        missing.remove(table);
        let refs = decode_references(&mut Cursor::new(stream(&missing)), limits()).unwrap();
        assert_eq!(
            relationships(&refs),
            Err(SourceClosureError::Relationship),
            "missing {table}"
        );
    }
    for (table, field, wrong) in [
        (
            "owner_beta_daily_admissions",
            "policy_sha256",
            value(serde_json::json!(digest(b"other-policy"))),
        ),
        (
            "owner_beta_daily_admissions",
            "owner_user_id",
            value(serde_json::json!("00000000-0000-4000-8000-000000000099")),
        ),
        (
            "owner_beta_daily_admissions",
            "range_end",
            value(serde_json::json!("2026-11-05")),
        ),
        (
            "owner_beta_daily_reservations",
            "activation_epoch",
            value(serde_json::json!(8)),
        ),
        (
            "owner_beta_recommendation_runs",
            "strategy_config_id",
            value(serde_json::json!("00000000-0000-4000-8000-000000000099")),
        ),
        ("job_attempts", "attempt_no", value(serde_json::json!(3))),
        (
            "job_attempts",
            "claimed_by",
            value(serde_json::json!("another-worker")),
        ),
    ] {
        let mut changed = rows.clone();
        let Json::Object(row) = &mut changed.get_mut(table).unwrap()[0].1 else {
            unreachable!()
        };
        row.insert(field.into(), wrong);
        let refs = decode_references(&mut Cursor::new(stream(&changed)), limits()).unwrap();
        assert_eq!(
            relationships(&refs),
            Err(SourceClosureError::Relationship),
            "changed {table}.{field}"
        );
    }
}

#[test]
fn revoked_and_expired_entitlements_are_descriptive_history_not_current_authority() {
    for status in ["REVOKED", "EXPIRED"] {
        let row = reference_vector(
            "data_entitlements",
            serde_json::json!({"status":status,
            "contract_document_sha256":digest(b"retained-contract-original"),"contract_reference":"historical-fixture",
            "effective_from":"2020-01-01","effective_until":"2021-01-01"}),
        );
        let rows = BTreeMap::from([("data_entitlements".to_owned(), vec![row])]);
        let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
        relationships(&refs).unwrap();
        let retained = refs
            .rows("data_entitlements")
            .unwrap()
            .values()
            .next()
            .unwrap();
        assert_eq!(retained.get("status").unwrap().text().unwrap(), status);
        let mut changed = rows;
        let Json::Object(row) = &mut changed.get_mut("data_entitlements").unwrap()[0].1 else {
            unreachable!()
        };
        row.insert(
            "contract_document_sha256".into(),
            Json::String("caller-asserted".into()),
        );
        let refs = decode_references(&mut Cursor::new(stream(&changed)), limits()).unwrap();
        assert_eq!(relationships(&refs), Err(SourceClosureError::Relationship));
    }
}

#[test]
fn daily_original_sql_preimages_cannot_be_replaced_by_matching_outer_pins() {
    let admission_hash = digest(b"retained-admission-original");
    let admission = value(
        serde_json::json!({"content_sha256":digest(b"bars"),"manifest_sha256":digest(b"manifest"),
        "policy_sha256":digest(b"policy"),"parent_admission_sha256":null}),
    );
    let manifest = value(
        serde_json::json!({"range_start":"2026-11-05","range_end":"2026-11-06",
        "sessions":["2026-11-05","2026-11-06"],"bars":{"row_count":22}}),
    );
    let (_, row) = reference_vector(
        "owner_beta_daily_admissions",
        serde_json::json!({
        "admission_sha256":admission_hash,"content_sha256":digest(b"bars"),"manifest_sha256":digest(b"manifest"),
        "policy_sha256":digest(b"policy"),"parent_admission_sha256":null,"range_start":"2026-11-05",
        "range_end":"2026-11-06","session_count":2,"bar_count":22,"evidence_json":{
            "admission_sha256":admission_hash,"admission":admission.serde_value().unwrap(),"manifest":manifest.serde_value().unwrap()}}),
    );
    daily_database_binding(&row, &admission_hash, &admission, &manifest).unwrap();
    for field in ["admission", "manifest", "admission_sha256"] {
        let mut changed = row.clone();
        let Json::Object(outer) = &mut changed else {
            unreachable!()
        };
        let Json::Object(evidence) = outer.get_mut("evidence_json").unwrap() else {
            unreachable!()
        };
        evidence.insert(field.into(), Json::Null);
        assert!(daily_database_binding(&changed, &admission_hash, &admission, &manifest).is_err());
    }
    for field in ["bar_count", "session_count"] {
        let mut changed = row.clone();
        let Json::Object(outer) = &mut changed else {
            unreachable!()
        };
        outer.insert(field.into(), Json::Number("1".into()));
        assert!(daily_database_binding(&changed, &admission_hash, &admission, &manifest).is_err());
    }
    assert!(
        daily_database_binding(&row, &digest(b"different-original"), &admission, &manifest)
            .is_err()
    );
}

// These fixtures MUST run as actual Linux Root in the existing hosted harness.
// They never skip/xfail internally, inspect runtime state, or use a live root.
struct RootFixture {
    directory: std::path::PathBuf,
    materialized: std::path::PathBuf,
    codec: std::path::PathBuf,
    manifest: Vec<u8>,
}
impl RootFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            rustix::process::geteuid().as_raw(),
            0,
            "actual Linux Root is required"
        );
        let directory = std::path::PathBuf::from(format!(
            "/var/lib/lagrange-p5-source-consumer-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let materialized = directory.join("files");
        std::fs::create_dir(&materialized).unwrap();
        std::fs::set_permissions(&materialized, std::fs::Permissions::from_mode(0o700)).unwrap();
        for class in CLASSES {
            std::fs::create_dir(materialized.join(class)).unwrap();
            std::fs::set_permissions(
                materialized.join(class),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        let bytes = b"{\"held\":true}\n";
        let leaf = materialized.join("paper_config/한.json");
        std::fs::write(&leaf, bytes).unwrap();
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o600)).unwrap();
        let manifest = codec_json(manifest_value(vec![file_entry(
            "paper_config/한.json",
            bytes,
        )]));
        let mut bundle = MAGIC.to_vec();
        bundle.extend_from_slice(&(manifest.len() as u64).to_be_bytes());
        bundle.extend_from_slice(&manifest);
        bundle.extend_from_slice(bytes);
        let codec = directory.join("files.bundle");
        std::fs::write(&codec, bundle).unwrap();
        std::fs::set_permissions(&codec, std::fs::Permissions::from_mode(0o600)).unwrap();
        Self {
            directory,
            materialized,
            codec,
            manifest,
        }
    }
    fn open(&self, attempt: &Attempt) -> Materialized {
        Materialized::open(
            &self.materialized,
            manifest(&self.manifest, limits()).unwrap(),
            attempt,
            limits(),
            &mut || true,
        )
        .unwrap()
    }
}
impl Drop for RootFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_codec_original_and_all_twenty_classes_match() {
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let original =
        InputOriginal::open(&fixture.codec, &attempt, limits().max_stream_bytes).unwrap();
    let expected = digest(&std::fs::read(&fixture.codec).unwrap());
    {
        let mut keep = || true;
        let mut reader = OriginalReader::new(&original, &mut keep);
        let mut prefix = vec![0u8; MAGIC.len() + 8 + fixture.manifest.len()];
        reader.read_exact(&mut prefix).unwrap();
        tree.verify_codec(&mut reader, &mut || true).unwrap();
        assert_eq!(reader.completed_hash().unwrap(), expected);
        assert_eq!(original.hash(&mut || true).unwrap(), expected);
        let classes = class_rows(&tree).unwrap();
        assert_eq!(classes.len(), 20);
        let config = classes
            .iter()
            .find(|row| row.get("class").unwrap().text().unwrap() == "paper_config")
            .unwrap();
        assert_eq!(config.get("entries").unwrap().integer().unwrap(), 1);
        assert_eq!(config.get("bytes").unwrap().integer().unwrap(), 14);
    }
    drop(original);
    drop(tree);
    assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_same_bytes_named_replacement_is_not_the_original() {
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let path = fixture.materialized.join("paper_config/한.json");
    let content = std::fs::read(&path).unwrap();
    std::fs::rename(&path, fixture.directory.join("replaced-original")).unwrap();
    std::fs::write(&path, content).unwrap();
    assert!(tree.read("paper_config/한.json", 1024).is_err());
    assert!(tree.revalidate(&mut || true).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_omitted_pending_or_orphan_inventory_is_rejected() {
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    std::fs::write(fixture.materialized.join("raw/orphan"), b"not committed").unwrap();
    assert!(tree.revalidate(&mut || true).is_err());
    std::fs::remove_file(fixture.materialized.join("raw/orphan")).unwrap();
    std::fs::create_dir(fixture.materialized.join("daily/.pending-1-1")).unwrap();
    assert!(tree.revalidate(&mut || true).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_parent_replacement_is_checked_before_original_read() {
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    std::fs::rename(
        fixture.materialized.join("paper_config"),
        fixture.directory.join("old-parent"),
    )
    .unwrap();
    std::fs::create_dir(fixture.materialized.join("paper_config")).unwrap();
    assert!(tree.read("paper_config/한.json", 1024).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_callback_error_cancellation_and_changed_provisional_bytes() {
    for mutate in [false, true] {
        let fixture = RootFixture::new();
        let attempt = Attempt::default();
        let tree = fixture.open(&attempt);
        let index = tree.manifest.by_path["paper_config/한.json"];
        let outcome = tree.visit_member(index, &mut || true, &mut |_| {
            if mutate {
                std::fs::write(
                    fixture.materialized.join("paper_config/한.json"),
                    b"{\"held\":null}\n",
                )
                .unwrap();
            }
            Err(SourceClosureError::Interrupted)
        });
        assert!(outcome.is_err());
        if mutate {
            assert!(tree.read("paper_config/한.json", 1024).is_err());
        }
        assert_eq!(
            tree.visit_member(index, &mut || false, &mut |_| panic!("cancelled callback")),
            Err(SourceClosureError::Interrupted)
        );
    }
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_codec_named_replacement_and_in_place_rewrite_are_rejected() {
    for replace_name in [false, true] {
        let fixture = RootFixture::new();
        let attempt = Attempt::default();
        let original =
            InputOriginal::open(&fixture.codec, &attempt, limits().max_stream_bytes).unwrap();
        let content = std::fs::read(&fixture.codec).unwrap();
        if replace_name {
            std::fs::rename(&fixture.codec, fixture.directory.join("old-bundle")).unwrap();
        }
        std::fs::write(&fixture.codec, content).unwrap();
        assert!(original.hash(&mut || true).is_err());
        let mut byte = [0u8; 1];
        assert!(original.read_at(&mut byte, 0).is_err());
    }
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_wrong_mode_owner_hardlink_and_symlink_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    for change in 0..4 {
        let fixture = RootFixture::new();
        let attempt = Attempt::default();
        let path = fixture.materialized.join("paper_config/한.json");
        match change {
            0 => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap(),
            1 => {
                rustix::fs::chown(&path, Some(rustix::process::Uid::from_raw(10001)), None)
                    .unwrap();
            }
            2 => std::fs::hard_link(&path, fixture.directory.join("alias")).unwrap(),
            _ => {
                std::fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(&fixture.codec, &path).unwrap();
            }
        }
        assert!(
            Materialized::open(
                &fixture.materialized,
                manifest(&fixture.manifest, limits()).unwrap(),
                &attempt,
                limits(),
                &mut || true
            )
            .is_err()
        );
    }
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_acl_is_rejected_even_with_unchanged_owner() {
    use std::os::fd::AsRawFd;
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let i = tree.manifest.by_path["paper_config/한.json"];
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, perms, id) in [
        (1u16, 6u16, u32::MAX),
        (2, 4, 10001),
        (4, 0, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(perms.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    let name = std::ffi::CString::new("system.posix_acl_access").unwrap();
    // SAFETY: owned live descriptor and exact NUL-terminated name/bounded bytes.
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                tree.nodes[i].file.as_raw_fd(),
                name.as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        },
        0
    );
    assert!(super::super::archive_no_acl(&tree.nodes[i].file).is_err());
    assert!(tree.read("paper_config/한.json", 1024).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_callback_unwind_poison_and_all_owned_closes_remain_visible() {
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = HistoricalUseGuard {
            attempt: &attempt,
            armed: true,
        };
        let tree = fixture.open(&attempt);
        let index = tree.manifest.by_path["paper_config/한.json"];
        let _ = tree.visit_member(index, &mut || true, &mut |_| panic!("fixture unwind"));
    }));
    assert!(outcome.is_err());
    assert!(!attempt.check());
    assert!(attempt.finish(Ok::<(), ()>(())).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_late_close_failure_invalidates_provisional_member_and_retires_all_owners() {
    struct FailingClose;
    impl crate::checked_close::Close for FailingClose {
        fn close(self) -> std::result::Result<(), crate::checked_close::Errno> {
            Err(crate::checked_close::Errno::Io)
        }
    }
    let fixture = RootFixture::new();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let index = tree.manifest.by_path["paper_config/한.json"];
    let mut visited = 0;
    assert_eq!(
        tree.visit_member(index, &mut || true, &mut |_| {
            visited += 1;
            assert!(
                !attempt
                    .adopt(FailingClose, crate::checked_close::Role::Source)
                    .finish()
            );
            Ok(())
        }),
        Err(SourceClosureError::Custody)
    );
    assert_eq!(visited, 1);
    drop(tree);
    let failed = attempt.finish(Ok::<(), ()>(())).err().unwrap();
    assert_eq!(failed.report().unknown(), 1);
    assert_eq!(failed.report().live(), 0);
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_old_v2_and_current_v3_require_their_own_retained_registry_original() {
    use super::super::HistoricalArchiveVersion::{V2, V3};
    let v2 = crate::historical_price_only_approval::embedded_v2_archive_pins().unwrap();
    let v3 = crate::historical_price_only_approval::embedded_v3_archive_pins().unwrap();
    for (version, pins, expected) in [
        (V2, &v2, SourceClosureError::HistoricalV2RegistryMissing),
        (V3, &v3, SourceClosureError::HistoricalV3RegistryMissing),
    ] {
        let fixture = RootFixture::new();
        let attempt = Attempt::default();
        let tree = fixture.open(&attempt);
        let mut fields = serde_json::Map::new();
        for (field, hash) in HISTORICAL_PIN_FIELDS.iter().zip([
            pins.candidate_content_sha256(),
            pins.artifact_manifest_sha256(),
            pins.stage5_manifest_sha256(),
            pins.action_manifest_sha256(),
            pins.approval_registry_sha256(),
        ]) {
            fields.insert(
                (*field).into(),
                serde_json::Value::String(hash.as_str().into()),
            );
        }
        let table = if version == V2 {
            "owner_beta_recommendation_runs"
        } else {
            "owner_beta_simulation_runs"
        };
        let (key, row) = reference_vector(table, serde_json::Value::Object(fields));
        let mut refs = References::new(limits());
        refs.insert(table, &key, row).unwrap();
        assert_eq!(historical_selections(&tree, &refs).err(), Some(expected));
        drop(tree);
        assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
    }
}

// SOURCE_ONLY / NOT_RUN: independent fixed SQL wire vectors and complete
// decoder transport are used; these are not full source_files certificates.
#[test]
fn daily_sql_jsonb_preimages_have_exact_utf8_length_key_order_and_spaces() {
    let input = value(serde_json::json!({"zz":[true,null,42],"a":"한","é":{"b":false,"aa":"x"}}));
    let exact = "{\"a\": \"한\", \"zz\": [true, null, 42], \"é\": {\"b\": false, \"aa\": \"x\"}}";
    assert_eq!(daily_sql_json(&input).unwrap(), exact.as_bytes());
    let mut independent = b"sql-block-v1".to_vec();
    independent.extend_from_slice(exact.as_bytes());
    assert_eq!(
        daily_sql_binding(b"sql-block-v1", &input).unwrap(),
        digest(&independent)
    );
    assert_eq!(
        daily_sql_binding(b"sql-block-v1", &input).unwrap(),
        "sha256:d72bd78ac18fd665db4d5f497e831f0ffe636b6434da32447f93762269cd5bc1"
    );
    assert_ne!(
        daily_sql_binding(b"sql-calendar-v1", &input).unwrap(),
        digest(&independent)
    );
    assert!(daily_sql_json(&parse(b"0.1").unwrap()).is_err());
    assert!(daily_sql_json(&parse(b"9223372036854775808").unwrap()).is_err());
}
#[test]
fn daily_sql_transport_rejects_replaced_nested_preimages_not_just_outer_hash_shapes() {
    let source = value(serde_json::json!({"admission_sha256":digest(b"admission"),
        "source_frontier_sha256":digest(b"frontier"),"admission":{"counts":{"bars":11}},
        "manifest":{"sessions":["2026-11-06"],"vendor_snapshot":true}}));
    let pin = daily_sql_binding(b"sql-admission-v1", &source).unwrap();
    for replacement in [
        serde_json::json!(null),
        serde_json::json!("11"),
        serde_json::json!(false),
        serde_json::json!({"counts":{"bars":12}}),
    ] {
        let mut row = source.clone();
        let Json::Object(ref mut object) = row else {
            unreachable!()
        };
        object.insert("admission".into(), value(replacement));
        assert_ne!(daily_sql_binding(b"sql-admission-v1", &row).unwrap(), pin);
    }
}
fn event_rows() -> BTreeMap<String, Vec<(String, Json)>> {
    let owner = "00000000-0000-0000-0000-000000000001";
    let evidence = digest(b"actual-diagnostic-observation");
    let preimage = format!(
        "owner-beta-daily-source-event-v1\n{owner}\nBAR\nSOURCE_INTEGRITY\n2026-11-06\n2026-11-06\n{evidence}\n"
    );
    let row = reference_vector(
        "owner_beta_daily_source_events",
        serde_json::json!({
        "event_sha256":digest(preimage.as_bytes()),"owner_user_id":owner,"source_kind":"BAR",
        "range_start":"2026-11-06","range_end":"2026-11-06","code":"SOURCE_INTEGRITY",
        "evidence_sha256":evidence,"admission_sha256":null,"recorded_at":"2026-11-06T10:00:00.000000Z"}),
    );
    BTreeMap::from([("owner_beta_daily_source_events".into(), vec![row])])
}
#[test]
fn complete_decoded_daily_event_preimage_is_exact_and_mutations_fail() {
    let rows = event_rows();
    let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
    daily_history_events(&refs).unwrap();
    for field in [
        "source_kind",
        "code",
        "evidence_sha256",
        "range_start",
        "owner_user_id",
    ] {
        let mut rows = rows.clone();
        let Json::Object(ref mut row) =
            rows.get_mut("owner_beta_daily_source_events").unwrap()[0].1
        else {
            unreachable!()
        };
        let changed = match field {
            "source_kind" => "REFERENCE".into(),
            "code" => "SOURCE_REVOKED".into(),
            "evidence_sha256" => digest(b"replaced"),
            "range_start" => "2026-11-05".into(),
            _ => "00000000-0000-0000-0000-000000000002".into(),
        };
        row.insert(field.into(), Json::String(changed));
        let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
        assert!(daily_history_events(&refs).is_err());
    }
}
fn unfinished_poll_rows() -> BTreeMap<String, Vec<(String, Json)>> {
    let receipt = "00000000-0000-0000-0000-000000000003";
    let row = reference_vector(
        "owner_beta_daily_poll_receipts",
        serde_json::json!({
        "receipt_id":receipt,"stage":"ADMISSION","owner_user_id":"00000000-0000-0000-0000-000000000001",
        "policy_sha256":digest(b"historical-policy"),"activation_epoch":4,"started_at":"2026-11-06T10:00:00.000000Z",
        "completed_at":null,"result_code":null,"source_frontier_sha256":null,"admission_sha256":null,
        "event_sha256":null,"completion_binding_sha256":null}),
    );
    let state = reference_vector(
        "owner_beta_daily_poll_state",
        serde_json::json!({
        "stage":"ADMISSION","next_poll_at":"2026-11-06T10:01:00.000000Z","last_completed_at":null,
        "last_result_code":null,"verified_admission_sha256":null,"source_frontier_sha256":null,"latest_receipt_id":receipt}),
    );
    BTreeMap::from([
        ("owner_beta_daily_poll_receipts".into(), vec![row]),
        ("owner_beta_daily_poll_state".into(), vec![state]),
    ])
}
#[test]
fn unfinished_poll_replaces_old_completion_without_erasing_historical_receipts() {
    let rows = unfinished_poll_rows();
    let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
    daily_poll_state(&refs).unwrap();
    for field in [
        "last_result_code",
        "verified_admission_sha256",
        "source_frontier_sha256",
        "next_poll_at",
    ] {
        let mut rows = rows.clone();
        let Json::Object(ref mut state) = rows.get_mut("owner_beta_daily_poll_state").unwrap()[0].1
        else {
            unreachable!()
        };
        state.insert(
            field.into(),
            Json::String(match field {
                "next_poll_at" => "2026-11-06T10:02:00.000000Z".into(),
                "last_result_code" => "VERIFIED".into(),
                _ => digest(b"old-completion"),
            }),
        );
        let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
        assert!(daily_poll_state(&refs).is_err());
    }
}
#[test]
fn poll_state_rejects_missing_latest_wrong_stage_and_missing_stage_inventory() {
    for mutation in 0..3 {
        let mut rows = unfinished_poll_rows();
        if mutation == 0 {
            rows.remove("owner_beta_daily_poll_receipts");
        } else if mutation == 1 {
            rows.remove("owner_beta_daily_poll_state");
        } else {
            let Json::Object(ref mut receipt) =
                rows.get_mut("owner_beta_daily_poll_receipts").unwrap()[0].1
            else {
                unreachable!()
            };
            receipt.insert("stage".into(), Json::String("SCHEDULE".into()));
        }
        let refs = decode_references(&mut Cursor::new(stream(&rows)), limits()).unwrap();
        assert!(daily_poll_state(&refs).is_err());
    }
}

fn retained_curated_fixture() -> (RootFixture, crate::curate::schema::OriginalArtifactValues) {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = RootFixture::new();
    let row = crate::curate::schema::CuratedBar {
        instrument_id: domain::InstrumentId::parse("069500.KRX").unwrap(),
        trading_date: domain::TradingDate::parse("2020-01-31").unwrap(),
        market_open_ts: domain::UtcTimestamp::parse_rfc3339("2020-01-31T00:00:00Z").unwrap(),
        market_close_ts: domain::UtcTimestamp::parse_rfc3339("2020-01-31T06:30:00Z").unwrap(),
        open: domain::Price::parse("100").unwrap(),
        high: domain::Price::parse("102").unwrap(),
        low: domain::Price::parse("99").unwrap(),
        close: domain::Price::parse("101").unwrap(),
        volume: 7,
        trading_value: Some(707),
        currency: domain::Currency::KRW,
        source: "krx".into(),
        ingested_at: domain::UtcTimestamp::parse_rfc3339("2020-01-31T08:00:00Z").unwrap(),
        batch_id: domain::BatchId::generate(),
        raw_hash: ContentHash::from_bytes(b"ordinary-fixture-original"),
    };
    let path = fixture.materialized.join("curated/retained.parquet");
    crate::curate::schema::write_bars(&path, std::slice::from_ref(&row)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    rustix::fs::chown(
        &path,
        Some(rustix::process::Uid::from_raw(10001)),
        Some(rustix::process::Gid::from_raw(10001)),
    )
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut entry = file_entry("curated/retained.parquet", &bytes);
    entry["uid"] = serde_json::json!(10001);
    entry["gid"] = serde_json::json!(10001);
    fixture.manifest = codec_json(manifest_value(vec![
        entry,
        file_entry("paper_config/한.json", b"{\"held\":true}\n"),
    ]));
    let mut codec = MAGIC.to_vec();
    codec.extend_from_slice(&(fixture.manifest.len() as u64).to_be_bytes());
    codec.extend_from_slice(&fixture.manifest);
    codec.extend_from_slice(&bytes);
    codec.extend_from_slice(b"{\"held\":true}\n");
    std::fs::write(&fixture.codec, codec).unwrap();
    (
        fixture,
        crate::curate::schema::OriginalArtifactValues::Bars(vec![row]),
    )
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_retained_parquet_uses_existing_original_and_checked_close() {
    let (fixture, values) = retained_curated_fixture();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let index = tree.manifest.by_path["curated/retained.parquet"];
    let original =
        InputOriginal::open(&fixture.codec, &attempt, limits().max_stream_bytes).unwrap();
    {
        let mut keep = || true;
        let mut reader = OriginalReader::new(&original, &mut keep);
        let mut header = vec![0u8; MAGIC.len() + 8 + fixture.manifest.len()];
        reader.read_exact(&mut header).unwrap();
        tree.verify_codec(&mut reader, &mut || true).unwrap();
        assert!(reader.completed_hash().is_ok());
        tree.verify_curated_values(
            index,
            crate::curate::schema::BARS_SCHEMA_ID,
            &values,
            limits(),
            &|| true,
        )
        .unwrap();
        assert_eq!(
            tree.manifest.entries[index].uid, 10001,
            "Root verifier does not change the original owner"
        );
    }
    drop(original);
    drop(tree);
    assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_retained_parquet_replacement_during_borrow_invalidates_values() {
    use std::cell::Cell;
    let (fixture, values) = retained_curated_fixture();
    let attempt = Attempt::default();
    let tree = fixture.open(&attempt);
    let index = tree.manifest.by_path["curated/retained.parquet"];
    let path = fixture.materialized.join("curated/retained.parquet");
    let bytes = std::fs::read(&path).unwrap();
    let calls = Cell::new(0usize);
    let keep = || {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            std::fs::rename(&path, fixture.directory.join("held-parquet-old")).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            rustix::fs::chown(
                &path,
                Some(rustix::process::Uid::from_raw(10001)),
                Some(rustix::process::Gid::from_raw(10001)),
            )
            .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        true
    };
    assert!(
        tree.verify_curated_values(
            index,
            crate::curate::schema::BARS_SCHEMA_ID,
            &values,
            limits(),
            &keep
        )
        .is_err()
    );
    assert!(tree.check_node(index).is_err());
    drop(tree);
    // Direct typed component failure is never a completed consuming archive.
    assert!(attempt.finish(Err::<(), ()>(())).is_err());
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_retained_parquet_cancel_and_late_owner_close_failures_retire_every_owner() {
    use std::cell::Cell;
    struct FailingClose;
    impl crate::checked_close::Close for FailingClose {
        fn close(self) -> std::result::Result<(), crate::checked_close::Errno> {
            Err(crate::checked_close::Errno::Io)
        }
    }
    for late_close in [false, true] {
        let (fixture, values) = retained_curated_fixture();
        let attempt = Attempt::default();
        let tree = fixture.open(&attempt);
        let index = tree.manifest.by_path["curated/retained.parquet"];
        let calls = Cell::new(0usize);
        let keep = || {
            calls.set(calls.get() + 1);
            if late_close && calls.get() == 4 {
                assert!(
                    !attempt
                        .adopt(FailingClose, crate::checked_close::Role::Source)
                        .finish()
                );
            }
            late_close || calls.get() < 4
        };
        assert!(
            tree.verify_curated_values(
                index,
                crate::curate::schema::BARS_SCHEMA_ID,
                &values,
                limits(),
                &keep
            )
            .is_err()
        );
        drop(tree);
        let failure = attempt.finish(Err::<(), ()>(())).err().unwrap();
        assert_eq!(failure.report().live(), 0);
        if late_close {
            assert_eq!(failure.report().unknown(), 1);
        }
    }
}

#[test]
#[ignore = "actual Linux Root protected filesystem fixture; source-only until hosted"]
fn actual_root_retained_parquet_callback_unwind_keeps_attempt_poisoned() {
    let (fixture, values) = retained_curated_fixture();
    let attempt = Attempt::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = HistoricalUseGuard {
            attempt: &attempt,
            armed: true,
        };
        let tree = fixture.open(&attempt);
        let index = tree.manifest.by_path["curated/retained.parquet"];
        let calls = std::cell::Cell::new(0usize);
        let keep = || {
            calls.set(calls.get() + 1);
            assert!(
                calls.get() != 10,
                "fixture cancellation unwind after original read"
            );
            true
        };
        let _ = tree.verify_curated_values(
            index,
            crate::curate::schema::BARS_SCHEMA_ID,
            &values,
            limits(),
            &keep,
        );
    }));
    assert!(result.is_err());
    assert!(!attempt.check());
    let failure = attempt.finish(Ok::<(), ()>(())).err().unwrap();
    assert_eq!(failure.report().live(), 0);
}

#[test]
fn archive_admission_expired_deadline_refuses_before_opening_any_original() {
    let attempt = Attempt::default();
    let result = ArchiveAdmissionContext::open(&attempt, std::time::Instant::now());
    assert!(matches!(result, Err(SourceClosureError::Interrupted)));
    assert_eq!(attempt.report().live(), 0);
    assert_eq!(attempt.report().attempted(), 0);
}

#[test]
fn archive_admission_cannot_extend_the_fifteen_second_operation_limit() {
    let attempt = Attempt::default();
    let result = ArchiveAdmissionContext::open(
        &attempt,
        std::time::Instant::now() + std::time::Duration::from_secs(16),
    );
    assert!(matches!(result, Err(SourceClosureError::Interrupted)));
    assert_eq!(attempt.report().live(), 0);
    assert_eq!(attempt.report().attempted(), 0);
}

#[test]
fn archive_scope_refusal_poisons_actual_invocation_and_expected_ledgers() {
    let actual = Attempt::default();
    let supplied = Attempt::default();
    let expected = Attempt::default();
    let invocation = RawInvocation::new(&supplied);
    let failure =
        crate::paper_source_evidence::archive_scope_refusal(&invocation, &actual, &expected);
    assert_eq!(
        failure.kind(),
        crate::paper_source_evidence::ExecutionGateFailureKind::Scope
    );
    assert!(!actual.check());
    assert!(!supplied.check());
    assert!(!expected.check());
    assert_eq!(actual.report().live(), 0);
    assert_eq!(supplied.report().live(), 0);
    assert_eq!(expected.report().live(), 0);
}
