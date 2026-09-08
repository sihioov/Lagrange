use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Utc};
use collectors::intraday_quotes::IntradaySessionWindowContract;
use kis_client::auth::{AccessToken, TokenIssuer, TokenManager};
use kis_client::clock::{Clock, SystemClock};
use kis_client::error::KisError;
use kis_client::rate_limit::{Quota, RateLimiter};
use kis_client::read_coordination::{
    ReadCoordinationConfig, ReadCredentialSnapshot, STATE_FILE_NAME,
};
use kis_client::retry::Sleeper;
use kis_client::secret::{CredentialError, CredentialRef, CredentialSource, Secret};
use kis_client::transport::{HttpRequest, HttpResponse, Transport};
use kis_client::{CoordinatedReadAuth, KisMarketDataClient};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tempfile::{Builder, TempDir};
use uuid::Uuid;

use crate::intraday_quotes_support::IntradayTestDb;

pub const SYNTHETIC_APP_KEY_REF: &str = "B2B_C1_SYNTHETIC_APP_KEY";
pub const SYNTHETIC_APP_SECRET_PATH: &str = "/tmp/b2b-c1-synthetic-app-secret";

#[derive(Clone, Copy, Debug)]
pub struct SyntheticCredentials;

impl CredentialSource for SyntheticCredentials {
    fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError> {
        match reference {
            CredentialRef::Env { var } if var == SYNTHETIC_APP_KEY_REF => {
                Ok(Secret::new("synthetic-app-key".to_owned()))
            }
            CredentialRef::File { path } if path == SYNTHETIC_APP_SECRET_PATH => {
                Ok(Secret::new("synthetic-app-secret".to_owned()))
            }
            _ => Err(CredentialError::NotFound {
                location: reference.describe(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep_ms(&self, _ms: u64) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

#[derive(Clone)]
pub enum TransportStep {
    Response(HttpResponse),
    ResponseAfter(Duration, HttpResponse),
    Error(KisError),
    Hang,
    ResponseAfterShorteningProducerLease {
        wait_before_update: Duration,
        response: HttpResponse,
        pool: PgPool,
        owner_user_id: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRecord {
    pub dispatched_at: Instant,
    pub method: &'static str,
    pub path: String,
    pub tr_id: String,
    pub query: Vec<(String, String)>,
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct ScriptedTransport {
    requests: Arc<Mutex<Vec<RequestRecord>>>,
    steps: Arc<Mutex<VecDeque<TransportStep>>>,
}

impl ScriptedTransport {
    pub fn new(steps: impl IntoIterator<Item = TransportStep>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            steps: Arc::new(Mutex::new(steps.into_iter().collect())),
        }
    }

    pub fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("synthetic request records")
            .len()
    }

    pub fn requests(&self) -> Vec<RequestRecord> {
        self.requests
            .lock()
            .expect("synthetic request records")
            .clone()
    }
}

impl Transport for ScriptedTransport {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, KisError>> + Send {
        let requests = Arc::clone(&self.requests);
        let steps = Arc::clone(&self.steps);
        async move {
            requests
                .lock()
                .expect("synthetic request records")
                .push(RequestRecord {
                    dispatched_at: Instant::now(),
                    method: request.method,
                    path: request.path,
                    tr_id: request.tr_id,
                    query: request.query,
                    headers: request.headers,
                });

            let step = steps.lock().expect("synthetic transport steps").pop_front();
            match step {
                Some(TransportStep::Response(response)) => Ok(response),
                Some(TransportStep::ResponseAfter(delay, response)) => {
                    tokio::time::sleep(delay).await;
                    Ok(response)
                }
                Some(TransportStep::Error(error)) => Err(error),
                Some(TransportStep::Hang) => std::future::pending().await,
                Some(TransportStep::ResponseAfterShorteningProducerLease {
                    wait_before_update,
                    response,
                    pool,
                    owner_user_id,
                }) => {
                    tokio::time::sleep(wait_before_update).await;
                    let updated = sqlx::query(
                        "UPDATE public.owner_intraday_quote_producers
                            SET lease_expires_at = pg_catalog.clock_timestamp()
                                + INTERVAL '2 seconds',
                                heartbeat_at = pg_catalog.clock_timestamp(),
                                updated_at = pg_catalog.clock_timestamp()
                          WHERE owner_user_id = $1",
                    )
                    .bind(owner_user_id)
                    .execute(&pool)
                    .await
                    .expect("producer-bound fixture lease update");
                    assert_eq!(updated.rows_affected(), 1);
                    Ok(response)
                }
                None => Err(KisError::Connect {
                    reason: "synthetic transport response exhausted".to_owned(),
                }),
            }
        }
    }
}

pub fn assert_minimum_dispatch_spacing(harness: &ClientHarness, minimum: Duration) {
    let requests = harness.transport.requests();
    for pair in requests.windows(2) {
        let spacing = pair[1].dispatched_at.duration_since(pair[0].dispatched_at);
        assert!(
            spacing >= minimum,
            "synthetic GET dispatch spacing was shorter than the guarded minimum"
        );
    }
}

pub struct SyntheticIssuer {
    calls: AtomicUsize,
}

impl SyntheticIssuer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl TokenIssuer for SyntheticIssuer {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessToken {
            value: Secret::new("synthetic-access-token".to_owned()),
            expires_at_ms: SystemClock.now_ms() + 24 * 60 * 60 * 1_000,
        })
    }
}

pub type PipelineClient = KisMarketDataClient<ScriptedTransport, NoSleep, SyntheticCredentials>;

pub struct ClientHarness {
    pub client: Arc<PipelineClient>,
    pub transport: ScriptedTransport,
    pub issuer: Arc<SyntheticIssuer>,
    coordinator_root: TempDir,
}

impl ClientHarness {
    pub fn new(label: &str, steps: impl IntoIterator<Item = TransportStep>) -> Self {
        let coordinator_root = Builder::new()
            .prefix(&format!("kis-intraday-pipeline-{label}-"))
            .tempdir()
            .expect("unique synthetic coordinator directory");
        fs::set_permissions(coordinator_root.path(), fs::Permissions::from_mode(0o700))
            .expect("protect synthetic coordinator directory");
        let expected_uid = std::os::unix::fs::MetadataExt::uid(
            &fs::metadata(coordinator_root.path()).expect("synthetic coordinator metadata"),
        );
        let clock = Arc::new(SystemClock);
        let app_key_ref = CredentialRef::env(SYNTHETIC_APP_KEY_REF);
        let app_secret_ref = CredentialRef::file(SYNTHETIC_APP_SECRET_PATH);
        let credentials = SyntheticCredentials;
        let snapshot = ReadCredentialSnapshot::resolve(
            &credentials,
            app_key_ref.clone(),
            app_secret_ref.clone(),
            1,
        )
        .expect("synthetic credential snapshot");
        let issuer = SyntheticIssuer::new();
        let issuer_trait: Arc<dyn TokenIssuer> = issuer.clone();
        let auth = CoordinatedReadAuth::new(
            ReadCoordinationConfig::new(coordinator_root.path(), expected_uid),
            snapshot,
            clock.clone(),
            Arc::clone(&issuer_trait),
        )
        .expect("synthetic coordinated authentication");
        let transport = ScriptedTransport::new(steps);
        let client = KisMarketDataClient::new(
            ScriptedTransport::new(std::iter::empty()),
            NoSleep,
            Arc::new(TokenManager::new(clock.clone(), issuer_trait)),
            Arc::new(RateLimiter::new(clock, Quota::new(100, 100))),
            credentials,
            app_key_ref,
            app_secret_ref,
        )
        .with_shared_read_coordination(auth, transport.clone());
        Self {
            client: Arc::new(client),
            transport,
            issuer,
            coordinator_root,
        }
    }

    pub fn state_path(&self) -> PathBuf {
        self.coordinator_root.path().join(STATE_FILE_NAME)
    }

    pub fn state(&self) -> Value {
        let bytes = fs::read(self.state_path()).expect("synthetic coordinator state");
        serde_json::from_slice(&bytes).expect("canonical synthetic coordinator state")
    }
}

pub async fn install_current_window_contract(
    db: &mut IntradayTestDb,
    close_after_seconds: i64,
) -> Result<Arc<IntradaySessionWindowContract>, String> {
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
        .fetch_one(&db.superuser)
        .await
        .map_err(|_| "could not sample window fixture database clock".to_owned())?;
    let kst = FixedOffset::east_opt(9 * 60 * 60).expect("KST offset");
    let local_close = now.with_timezone(&kst) + chrono::Duration::seconds(close_after_seconds);
    if local_close.date_naive() != db.session_date {
        return Err("window fixture crossed the KST date boundary".to_owned());
    }
    let close_local = local_close.time().format("%H:%M:%S").to_string();
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "exchange": "KRX",
        "timezone": "Asia/Seoul",
        "entries": [{
            "date": db.session_date.to_string(),
            "disposition": "SPECIAL",
            "open_local": "00:00:00",
            "close_local": close_local,
            "evidence_url": "https://global.krx.co.kr/contents/test",
            "evidence_retrieved_at": (now - chrono::Duration::seconds(1)).to_rfc3339(),
            "evidence_sha256": format!("sha256:{}", "c".repeat(64)),
        }],
    }))
    .map_err(|_| "window fixture serialization failed".to_owned())?;
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    let contract = IntradaySessionWindowContract::from_bytes(&bytes, &hash)
        .map_err(|_| "window fixture contract validation failed".to_owned())?;
    db.window_contract_sha256 = hash;
    Ok(Arc::new(contract))
}

pub fn valid_quote_response(symbol: &str) -> HttpResponse {
    quote_response(symbol, "N")
}

#[allow(dead_code)]
pub fn halted_quote_response(symbol: &str) -> HttpResponse {
    quote_response(symbol, "Y")
}

fn quote_response(symbol: &str, temp_stop_yn: &str) -> HttpResponse {
    HttpResponse::ok(
        json!({
            "rt_cd": "0",
            "output": {
                "stck_shrn_iscd": symbol,
                "stck_prpr": "72500",
                "prdy_vrss": "1500",
                "prdy_vrss_sign": "2",
                "prdy_ctrt": "2.11",
                "stck_sdpr": "71000",
                "iscd_stat_cls_code": "00",
                "temp_stop_yn": temp_stop_yn
            }
        })
        .to_string(),
    )
    .with_header("tr_cont", "M")
}

pub fn malformed_quote_response() -> HttpResponse {
    HttpResponse::ok(r#"{"rt_cd":"0","output":{"stck_shrn_iscd":"005930"}}"#)
}

pub fn rate_limited_response(retry_after_seconds: u64) -> HttpResponse {
    HttpResponse::status(429, "synthetic provider payload")
        .with_header("retry-after", retry_after_seconds.to_string())
}

pub fn transport_timeout_step() -> TransportStep {
    TransportStep::Error(KisError::Broker {
        status: 504,
        endpoint: collectors::intraday_quotes::INTRADAY_QUOTE_PATH.to_owned(),
        body: "synthetic gateway payload".to_owned(),
    })
}

pub async fn publication_counts(pool: &PgPool) -> Result<(i64, i64, i64, i64, i64, i64), String> {
    let data_batches: i64 = sqlx::query_scalar("SELECT count(*) FROM public.data_batches")
        .fetch_one(pool)
        .await
        .map_err(|_| "could not count data batches".to_owned())?;
    let dataset_versions: i64 = sqlx::query_scalar("SELECT count(*) FROM public.dataset_versions")
        .fetch_one(pool)
        .await
        .map_err(|_| "could not count dataset versions".to_owned())?;
    let trading_calendars: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.trading_calendars")
            .fetch_one(pool)
            .await
            .map_err(|_| "could not count trading calendars".to_owned())?;
    let calendar_versions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.trading_calendar_versions")
            .fetch_one(pool)
            .await
            .map_err(|_| "could not count calendar versions".to_owned())?;
    let signal_snapshots: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_equity_signal_snapshots")
            .fetch_one(pool)
            .await
            .map_err(|_| "could not count signal snapshots".to_owned())?;
    let signal_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.owner_equity_signal_snapshot_rows")
            .fetch_one(pool)
            .await
            .map_err(|_| "could not count signal snapshot rows".to_owned())?;
    Ok((
        data_batches,
        dataset_versions,
        trading_calendars,
        calendar_versions,
        signal_snapshots,
        signal_rows,
    ))
}
