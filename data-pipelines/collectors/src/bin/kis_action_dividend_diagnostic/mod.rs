//! One Owner-approved diagnostic (2026-10-22). No production admission path.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use kis_client::transport::{HttpRequest, HttpResponse};
use kis_client::{KisError, MarketDataReply, ReadCoordinationMode, Transport};
use market_data::contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind};
use market_data::providers::kis::validate_kis_dividend_diagnostic;
use market_data::storage::{BatchSpec, RawStore};
use serde_json::{Value, json};

const PATH: &str = "/uapi/domestic-stock/v1/ksdinfo/dividend";
const TR: &str = "HHKDB669102C0";
const PROVIDER: &str = "kis-dividend-diagnostic-20261022";
const CONFIRM: &str = "I_APPROVE_ONCE_DIVIDEND_4_QUERIES_MAX16_GET_NO_ADMISSION";
const LABELS: [&str; 4] = [
    "target-symbol-single-day",
    "target-symbol-wide-range",
    "whole-market-single-day",
    "whole-market-wide-range",
];

fn query(index: usize) -> Vec<(String, String)> {
    let wide = index % 2 == 1;
    [
        ("CTS", ""),
        ("GB1", "0"),
        ("F_DT", if wide { "20260829" } else { "20260915" }),
        ("T_DT", if wide { "20261018" } else { "20260915" }),
        ("SHT_CD", if index < 2 { "153130" } else { "" }),
        ("HIGH_GB", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn write_once(path: &Path, value: &Value) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "EVIDENCE_ENCODING")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "EVIDENCE_CREATE_FAILED")?;
    file.write_all(&bytes)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|_| "EVIDENCE_SYNC_FAILED")?;
    std::fs::File::open(path.parent().ok_or("EVIDENCE_PARENT")?)
        .and_then(|f| f.sync_all())
        .map_err(|_| "EVIDENCE_DIRECTORY_SYNC_FAILED")
}

#[derive(Default)]
struct Budget {
    logical: usize,
    total: usize,
    per_query: [usize; 4],
}
struct BoundedTransport<T> {
    inner: T,
    budget: Arc<Mutex<Budget>>,
    evidence: PathBuf,
}
impl<T: Transport> Transport for BoundedTransport<T> {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, KisError> {
        let invalid = || KisError::SchemaDrift {
            endpoint: PATH.into(),
            detail: "DIAGNOSTIC_BOUNDARY_REJECTED".into(),
        };
        let ordinal = {
            let mut budget = self.budget.lock().map_err(|_| invalid())?;
            let i = budget.logical;
            if i >= 4
                || request.method != "GET"
                || request.path != PATH
                || request.tr_id != TR
                || request.body.is_some()
                || !request.secret_query.is_empty()
                || request.query != query(i)
                || request
                    .headers
                    .get("tr_cont")
                    .is_some_and(|v| !v.is_empty())
                || budget.total >= 16
                || budget.per_query[i] >= 4
            {
                return Err(invalid());
            }
            let ordinal = budget.total + 1;
            write_once(
                &self.evidence.join(format!("wire-{ordinal:02}-intent.json")),
                &json!({"at": UtcTimestamp::now(), "logicalQuery": i,
                        "wireAttemptOrdinal": ordinal, "queryAttempt": budget.per_query[i] + 1,
                        "method":"GET", "endpoint":PATH,"trId":TR,"query":query(i)}),
            )
            .map_err(|_| invalid())?;
            budget.total += 1;
            budget.per_query[i] += 1;
            budget.total
        };
        let result = self.inner.send(request).await;
        write_once(&self.evidence.join(format!("wire-{ordinal:02}-terminal.json")),
            &json!({"at":UtcTimestamp::now(),"wireAttemptOrdinal":ordinal,
                    "responseReceived":result.is_ok(),"httpStatus":result.as_ref().ok().map(|v|v.status)}))
            .map_err(|_| invalid())?;
        result
    }
}

fn retain(
    store: &RawStore,
    entitlement: &str,
    index: usize,
    reply: MarketDataReply,
) -> Result<Value, &'static str> {
    if reply.body.len() > 4 * 1024 * 1024 {
        return Err("DIAGNOSTIC_RESPONSE_SIZE");
    }
    validate_kis_dividend_diagnostic(&reply.body)?;
    let doc: Value =
        serde_json::from_slice(&reply.body).map_err(|_| "KIS_RESPONSE_MALFORMED_JSON")?;
    let rows: Vec<&Value> = match &doc["output1"] {
        Value::Array(rows) => rows.iter().collect(),
        row => vec![row],
    };
    let mut target_hashes = Vec::new();
    let mut day_rows = 0;
    for row in &rows {
        let symbol = row["sht_cd"].as_str().ok_or("DIAGNOSTIC_SYMBOL_MISSING")?;
        let day = row["record_date"]
            .as_str()
            .ok_or("DIAGNOSTIC_DATE_MISSING")?;
        if index < 2 && symbol != "153130" {
            return Err("DIAGNOSTIC_SYMBOL_MISMATCH");
        }
        let q = query(index);
        if day < q[2].1.as_str() || day > q[3].1.as_str() {
            return Err("DIAGNOSTIC_DATE_MISMATCH");
        }
        if day == "20260915" {
            day_rows += 1;
            if symbol == "153130" {
                target_hashes.push(
                    ContentHash::from_bytes(
                        &serde_json::to_vec(row).map_err(|_| "EVIDENCE_ENCODING")?,
                    )
                    .to_string(),
                );
            }
        }
    }
    let batch_id = BatchId::generate();
    let received = UtcTimestamp::now();
    let marker = reply.continuation.clone();
    let raw_hash = ContentHash::from_bytes(&reply.body).to_string();
    let bytes = reply.body.len();
    let envelope = RawEnvelope::new(
        batch_id,
        ResponseKind::CorporateActions,
        format!("diagnostic-dividend-{}.json", LABELS[index]),
        reply.body,
        received,
        RequestMetadata {
            endpoint: PATH.into(),
            query: query(index),
            headers: vec![
                ("tr_id".into(), TR.into()),
                ("tr_cont".into(), "".into()),
                ("authorization".into(), "[REDACTED]".into()),
                ("appkey".into(), "[REDACTED]".into()),
                ("appsecret".into(), "[REDACTED]".into()),
            ],
            mode: FetchMode::Credentialed,
        },
    )
    .with_response_continuation(reply.continuation);
    let date = TradingDate::parse("2026-09-15").map_err(|_| "DIAGNOSTIC_DATE")?;
    let entry = store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER,
                market: "kr",
                date: &date,
                batch_id,
                entitlement_reference: Some(entitlement),
                mode: FetchMode::Credentialed,
            },
            &[envelope],
        )
        .map_err(|_| "DIAGNOSTIC_RAW_COMMIT_FAILED_OR_UNCERTAIN")?;
    store
        .read_batch_bytes(PROVIDER, "kr", &entry)
        .map_err(|_| "DIAGNOSTIC_RAW_READBACK_FAILED")?;
    Ok(
        json!({"query":LABELS[index],"batchId":batch_id,"receivedAt":received,
        "provider":PROVIDER,"rowCount":rows.len(),"sep15Rows":day_rows,
        "targetSep15RowHashes":target_hashes,"responseBytes":bytes,"sha256":raw_hash,
        "responseContinuation":marker,"admitted":false,"completenessAsserted":false}),
    )
}

async fn capture<F, Fut>(
    store: &RawStore,
    entitlement: &str,
    evidence: &Path,
    budget: &Arc<Mutex<Budget>>,
    mut get: F,
) -> Result<Vec<Value>, &'static str>
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = Result<MarketDataReply, KisError>>,
{
    let mut results = Vec::new();
    for (i, label) in LABELS.iter().enumerate() {
        budget.lock().map_err(|_| "BUDGET_LOCK")?.logical = i;
        write_once(
            &evidence.join(format!("query-{i}-intent.json")),
            &json!({"at":UtcTimestamp::now(),"label":label,"query":query(i),"pagination":false}),
        )?;
        let reply = get(i)
            .await
            .map_err(|_| "DIAGNOSTIC_GET_FAILED_OR_UNCERTAIN")?;
        let incomplete = reply.continuation.as_deref() == Some("M");
        let summary = retain(store, entitlement, i, reply)?;
        write_once(&evidence.join(format!("query-{i}-result.json")), &summary)?;
        results.push(summary);
        if incomplete {
            return Err("DIAGNOSTIC_CONTINUATION_INCOMPLETE");
        }
    }
    Ok(results)
}

fn execute() -> Result<Value, &'static str> {
    if std::env::var("KIS_DIVIDEND_DIAGNOSTIC_CONFIRM")
        .ok()
        .as_deref()
        != Some(CONFIRM)
    {
        return Err("DIAGNOSTIC_OWNER_ACK_REQUIRED");
    }
    let coordination =
        super::ProductionReadCoordination::from_env().map_err(|_| "CONFIG_INVALID")?;
    if coordination.mode() != ReadCoordinationMode::SharedRequired {
        return Err("SHARED_COORDINATION_REQUIRED");
    }
    let config = super::read_local_config(true).map_err(|_| "CONFIG_INVALID")?;
    let parent = config.raw_root.join("raw");
    if !parent.is_dir() || parent.is_symlink() {
        return Err("RAW_ROOT_INVALID");
    }
    let evidence = parent.join("dividend-conflict-diagnostic-20261022-once");
    // Durable one-use claim precedes credential resolution or any network.
    fs::create_dir(&evidence).map_err(|_| "DIAGNOSTIC_ALREADY_CLAIMED_OR_UNAVAILABLE")?;
    std::fs::File::open(&parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| "CLAIM_SYNC_FAILED")?;
    write_once(
        &evidence.join("claim.json"),
        &json!({"at":UtcTimestamp::now(),
        "ownerApproval":"2026-10-22 one exception", "maxWireGETs":16, "logicalQueries":4,
        "automaticReplay":false,"productionAdmission":false}),
    )?;
    let budget = Arc::new(Mutex::new(Budget::default()));
    let outcome = (|| {
        let client = super::build_client(&config.app_key_file, &config.app_secret_file, |inner| {
            BoundedTransport {
                inner,
                budget: Arc::clone(&budget),
                evidence: evidence.clone(),
            }
        })
        .map_err(|_| "DIAGNOSTIC_CLIENT_UNAVAILABLE")?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "DIAGNOSTIC_RUNTIME_UNAVAILABLE")?;
        runtime.block_on(capture(
            &RawStore::new(&config.raw_root),
            &config.entitlement_reference,
            &evidence,
            &budget,
            |i| {
                let client = &client;
                async move { client.get(PATH, TR, &query(i), None).await }
            },
        ))
    })();
    let measured = budget.lock().map_err(|_| "BUDGET_LOCK")?;
    let result = json!({"status":if outcome.is_ok(){"DIAGNOSTIC_CAPTURED"}else{"DIAGNOSTIC_STOPPED"},
        "at":UtcTimestamp::now(),"wireGETDispatches":measured.total,"perQueryDispatches":measured.per_query,
        "results":outcome.as_ref().ok(),"errorCode":outcome.as_ref().err(),
        "completenessAsserted":false,"productionAdmission":false,"automaticReplay":false});
    write_once(&evidence.join("result.json"), &result)?;
    println!("{result}");
    outcome.map(|_| result)
}

pub(super) fn main(args: &[String]) -> ExitCode {
    if args == ["--plan"] {
        println!(
            "{}",
            json!({"status":"PLAN_ONLY","queries":(0..4).map(query).collect::<Vec<_>>(),
            "maxWireGETsIncludingRetries":16,"singlePage":true,"sharedCoordinationRequired":true,
            "provider":PROVIDER,"productionAdmission":false,"networkCalls":0})
        );
        return ExitCode::SUCCESS;
    }
    if args != ["--execute"] {
        eprintln!("DIAGNOSTIC_EXACT_ARGUMENTS_REQUIRED");
        return ExitCode::from(64);
    }
    match execute() {
        Ok(_) => ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("{code}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn empty(marker: &str) -> MarketDataReply {
        MarketDataReply {
            body: br#"{"rt_cd":"0","output1":[]}"#.to_vec(),
            continuation: Some(marker.into()),
        }
    }
    #[tokio::test]
    async fn four_queries_commit_isolated_raw_without_admission() {
        let d = tempfile::tempdir().unwrap();
        let store = RawStore::new(d.path());
        let budget = Arc::new(Mutex::new(Budget::default()));
        let calls = AtomicUsize::new(0);
        let result = capture(&store, "fixture", d.path(), &budget, |i| {
            assert_eq!(i, calls.fetch_add(1, Ordering::SeqCst));
            std::future::ready(Ok(empty("F")))
        })
        .await
        .unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(store.read_manifest(PROVIDER, "kr").unwrap().len(), 4);
        assert!(!store.manifest_path("kis", "kr").exists());
    }
    #[tokio::test]
    async fn continuation_is_retained_then_stops_without_following() {
        let d = tempfile::tempdir().unwrap();
        let store = RawStore::new(d.path());
        let budget = Arc::new(Mutex::new(Budget::default()));
        let calls = AtomicUsize::new(0);
        let result = capture(&store, "fixture", d.path(), &budget, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(empty("M")))
        })
        .await;
        assert_eq!(result.unwrap_err(), "DIAGNOSTIC_CONTINUATION_INCOMPLETE");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.read_manifest(PROVIDER, "kr").unwrap().len(), 1);
    }
    #[tokio::test]
    async fn failed_query_stops_entire_run() {
        let d = tempfile::tempdir().unwrap();
        let budget = Arc::new(Mutex::new(Budget::default()));
        let calls = AtomicUsize::new(0);
        let result = capture(
            &RawStore::new(d.path()),
            "fixture",
            d.path(),
            &budget,
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(KisError::Auth {
                    reason: "fixture".into(),
                }))
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), "DIAGNOSTIC_GET_FAILED_OR_UNCERTAIN");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    struct Counting(AtomicUsize);
    impl Transport for Counting {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, KisError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(HttpResponse::ok("{}"))
        }
    }
    fn request(i: usize) -> HttpRequest {
        let mut r = HttpRequest::get(PATH, TR);
        r.query = query(i);
        r
    }
    #[tokio::test]
    async fn transport_enforces_query_and_wire_budgets_before_dispatch() {
        let d = tempfile::tempdir().unwrap();
        let budget = Arc::new(Mutex::new(Budget::default()));
        let t = BoundedTransport {
            inner: Counting(AtomicUsize::new(0)),
            budget: Arc::clone(&budget),
            evidence: d.path().into(),
        };
        let mut wrong = request(0);
        wrong.path = "/forbidden".into();
        assert!(t.send(wrong).await.is_err());
        for i in 0..4 {
            budget.lock().unwrap().logical = i;
            for _ in 0..4 {
                t.send(request(i)).await.unwrap();
            }
            assert!(t.send(request(i)).await.is_err());
        }
        assert_eq!(t.inner.0.load(Ordering::SeqCst), 16);
    }
    #[test]
    fn positive_dividend_is_retained_byte_exact_and_wrong_scope_fails() {
        let d = tempfile::tempdir().unwrap();
        let store = RawStore::new(d.path());
        let row = json!({"sht_cd":"153130","record_date":"20260915","divi_kind":"1",
            "face_val":"1000","per_sto_divi_amt":"10","divi_rate":"1","stk_divi_rate":"0",
            "divi_pay_dt":"","stk_div_pay_dt":"","odd_pay_dt":"","stk_kind":"","high_divi_gb":""});
        let body = serde_json::to_vec(&json!({"rt_cd":"0","output1":[row.clone()]})).unwrap();
        let summary = retain(
            &store,
            "fixture",
            0,
            MarketDataReply {
                body: body.clone(),
                continuation: Some("F".into()),
            },
        )
        .unwrap();
        assert_eq!(summary["rowCount"], 1);
        assert_eq!(summary["targetSep15RowHashes"].as_array().unwrap().len(), 1);
        assert_eq!(
            summary["sha256"],
            ContentHash::from_bytes(&body).to_string()
        );
        let mut wrong = row;
        wrong["sht_cd"] = json!("069500");
        assert_eq!(
            retain(
                &store,
                "fixture",
                0,
                MarketDataReply {
                    body: serde_json::to_vec(&json!({"rt_cd":"0","output1":[wrong]})).unwrap(),
                    continuation: None
                }
            )
            .unwrap_err(),
            "DIAGNOSTIC_SYMBOL_MISMATCH"
        );
        assert_eq!(store.read_manifest(PROVIDER, "kr").unwrap().len(), 1);
    }
    #[test]
    fn malformed_or_out_of_scope_response_never_commits() {
        let d = tempfile::tempdir().unwrap();
        let store = RawStore::new(d.path());
        let mut reply = empty("F");
        reply.body = br#"{"rt_cd":"0","output1":[{"sht_cd":"153130"}]}"#.to_vec();
        assert!(retain(&store, "fixture", 0, reply).is_err());
        assert!(!store.manifest_path(PROVIDER, "kr").exists());
    }
    #[test]
    fn existing_evidence_cannot_be_replayed_or_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("claim.json");
        write_once(&p, &json!({"original":true})).unwrap();
        assert!(write_once(&p, &json!({"replacement":true})).is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "{\"original\":true}\n");
    }
}
