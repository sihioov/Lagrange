use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use fs2::FileExt;
use kis_client::auth::{AccessToken, TokenIssuer, TokenManager};
use kis_client::clock::{Clock, TestClock};
use kis_client::error::KisError;
use kis_client::market_data::{IntradayAttemptError, KisMarketDataClient};
use kis_client::rate_limit::{Quota, RateLimiter};
use kis_client::read_coordination::{
    LOCK_FILE_NAME, ReadCallbackResult, ReadCoordinationConfig, ReadCoordinationError,
    ReadCoordinator, ReadCredentialSnapshot,
};
use kis_client::retry::Sleeper;
use kis_client::secret::{CredentialError, CredentialRef, CredentialSource, Secret};
use kis_client::transport::{HttpRequest, HttpResponse, Transport};

const PRICE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
const PRICE_TR: &str = "FHKST01010100";
const DAILY_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_TR: &str = "FHKST03010100";
const KST_DAY_MS: i64 = 15 * 60 * 60 * 1_000;

#[derive(Clone, Debug)]
struct RequestRecord {
    method: &'static str,
    path: String,
    tr_id: String,
    query: Vec<(String, String)>,
    headers: std::collections::BTreeMap<String, String>,
}

#[derive(Clone)]
enum TransportStep {
    Reply(Result<HttpResponse, KisError>),
    Hang,
}

#[derive(Clone)]
struct FakeTransport {
    requests: Arc<std::sync::Mutex<Vec<RequestRecord>>>,
    steps: Arc<std::sync::Mutex<VecDeque<TransportStep>>>,
    clock: Option<TestClock>,
    receipt_advance_ms: i64,
}

impl FakeTransport {
    fn new(steps: impl IntoIterator<Item = TransportStep>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            steps: Arc::new(std::sync::Mutex::new(steps.into_iter().collect())),
            clock: None,
            receipt_advance_ms: 0,
        }
    }

    fn with_receipt_clock(mut self, clock: TestClock, advance_ms: i64) -> Self {
        self.clock = Some(clock);
        self.receipt_advance_ms = advance_ms;
        self
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request records").len()
    }

    fn requests(&self) -> Vec<RequestRecord> {
        self.requests.lock().expect("request records").clone()
    }
}

impl Transport for FakeTransport {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, KisError>> + Send {
        let requests = Arc::clone(&self.requests);
        let steps = Arc::clone(&self.steps);
        let clock = self.clock.clone();
        let receipt_advance_ms = self.receipt_advance_ms;
        async move {
            requests
                .lock()
                .expect("request records")
                .push(RequestRecord {
                    method: request.method,
                    path: request.path,
                    tr_id: request.tr_id,
                    query: request.query,
                    headers: request.headers,
                });
            if let Some(clock) = clock {
                clock.advance_ms(receipt_advance_ms);
            }
            let step = steps
                .lock()
                .expect("transport steps")
                .pop_front()
                .unwrap_or_else(|| {
                    TransportStep::Reply(Err(KisError::Connect {
                        reason: "no fake response".to_owned(),
                    }))
                });
            match step {
                TransportStep::Reply(result) => result,
                TransportStep::Hang => std::future::pending().await,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FixedCredentials;

impl CredentialSource for FixedCredentials {
    fn resolve(&self, reference: &CredentialRef) -> Result<Secret<String>, CredentialError> {
        match reference {
            CredentialRef::Env { var } if var == "KIS_APP_KEY" => {
                Ok(Secret::new("fake-app-key".to_owned()))
            }
            CredentialRef::File { path } if path == "/run/secrets/kis_app_secret" => {
                Ok(Secret::new("fake-app-secret".to_owned()))
            }
            _ => Err(CredentialError::NotFound {
                location: reference.describe(),
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep_ms(&self, _ms: u64) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

struct FixedIssuer {
    clock: TestClock,
    calls: AtomicUsize,
    ttl_ms: i64,
}

impl FixedIssuer {
    fn new(clock: TestClock) -> Arc<Self> {
        Arc::new(Self {
            clock,
            calls: AtomicUsize::new(0),
            ttl_ms: 47 * 60 * 60 * 1_000,
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl TokenIssuer for FixedIssuer {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessToken {
            value: Secret::new("fake-access-token".to_owned()),
            expires_at_ms: self.clock.now_ms() + self.ttl_ms,
        })
    }
}

struct TempRoot {
    base: PathBuf,
    root: PathBuf,
    uid: u32,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "kis-intraday-attempt-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir(&base).expect("create fake base");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).expect("protect fake base");
        let root = base.join("state");
        fs::create_dir(&root).expect("create fake state root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("protect fake state root");
        let uid = fs::metadata(&root).expect("state root metadata").uid();
        Self { base, root, uid }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

struct Harness {
    client: KisMarketDataClient<FakeTransport, NoSleep, FixedCredentials>,
    intraday: FakeTransport,
    clock: TestClock,
    issuer: Arc<FixedIssuer>,
    root: TempRoot,
}

impl Harness {
    fn new(label: &str, steps: impl IntoIterator<Item = TransportStep>) -> Self {
        let root = TempRoot::new(label);
        let clock = TestClock::at(0);
        let issuer = FixedIssuer::new(clock.clone());
        let app_key_ref = CredentialRef::env("KIS_APP_KEY");
        let app_secret_ref = CredentialRef::file("/run/secrets/kis_app_secret");
        let snapshot = ReadCredentialSnapshot::resolve(
            &FixedCredentials,
            app_key_ref.clone(),
            app_secret_ref.clone(),
            1,
        )
        .expect("fake credential snapshot");
        let issuer_trait: Arc<dyn TokenIssuer> = issuer.clone();
        let auth = kis_client::CoordinatedReadAuth::new(
            ReadCoordinationConfig::new(&root.root, root.uid),
            snapshot,
            Arc::new(clock.clone()),
            Arc::clone(&issuer_trait),
        )
        .expect("fake coordinated auth");
        let intraday = FakeTransport::new(steps).with_receipt_clock(clock.clone(), 0);
        let eod = FakeTransport::new([]);
        let client = KisMarketDataClient::new(
            eod,
            NoSleep,
            Arc::new(TokenManager::new(Arc::new(clock.clone()), issuer_trait)),
            Arc::new(RateLimiter::new(
                Arc::new(clock.clone()),
                Quota::new(100, 100),
            )),
            FixedCredentials,
            app_key_ref,
            app_secret_ref,
        )
        .with_shared_read_coordination(auth, intraday.clone());
        Self {
            client,
            intraday,
            clock,
            issuer,
            root,
        }
    }

    fn at_receipt_offset(
        label: &str,
        steps: impl IntoIterator<Item = TransportStep>,
        receipt_advance_ms: i64,
    ) -> Self {
        let mut harness = Self::new(label, steps);
        harness.client = {
            let app_key_ref = CredentialRef::env("KIS_APP_KEY");
            let app_secret_ref = CredentialRef::file("/run/secrets/kis_app_secret");
            let snapshot = ReadCredentialSnapshot::resolve(
                &FixedCredentials,
                app_key_ref.clone(),
                app_secret_ref.clone(),
                1,
            )
            .expect("fake credential snapshot");
            let issuer_trait: Arc<dyn TokenIssuer> = harness.issuer.clone();
            let auth = kis_client::CoordinatedReadAuth::new(
                ReadCoordinationConfig::new(&harness.root.root, harness.root.uid),
                snapshot,
                Arc::new(harness.clock.clone()),
                Arc::clone(&issuer_trait),
            )
            .expect("fake coordinated auth");
            let intraday = harness
                .intraday
                .clone()
                .with_receipt_clock(harness.clock.clone(), receipt_advance_ms);
            KisMarketDataClient::new(
                FakeTransport::new([]),
                NoSleep,
                Arc::new(TokenManager::new(
                    Arc::new(harness.clock.clone()),
                    issuer_trait,
                )),
                Arc::new(RateLimiter::new(
                    Arc::new(harness.clock.clone()),
                    Quota::new(100, 100),
                )),
                FixedCredentials,
                app_key_ref,
                app_secret_ref,
            )
            .with_shared_read_coordination(auth, intraday)
        };
        harness
    }
}

fn ok_response() -> TransportStep {
    TransportStep::Reply(Ok(HttpResponse::ok(
        r#"{"rt_cd":"0","msg1":"provider body must not cross the API","output":{}}"#,
    )))
}

fn rate_limited_response(seconds: &str) -> TransportStep {
    TransportStep::Reply(Ok(HttpResponse::status(
        429,
        "provider body must not cross the API",
    )
    .with_header("retry-after", seconds)))
}

async fn eligible_attempt(
    harness: &Harness,
) -> Result<kis_client::IntradayAttemptReply, IntradayAttemptError> {
    harness
        .client
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], || async {
            Ok::<bool, &'static str>(true)
        })
        .await
}

#[tokio::test]
async fn success_returns_real_reservation_metadata_and_receipt_before_validation() {
    let harness = Harness::at_receipt_offset("success", [ok_response()], 37);
    let result = harness
        .client
        .get_intraday_attempt(
            PRICE_PATH,
            PRICE_TR,
            &[
                ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned()),
                ("FID_INPUT_ISCD".to_owned(), "005930".to_owned()),
            ],
            || async { Ok::<bool, &'static str>(true) },
        )
        .await
        .expect("single intraday attempt");

    assert_eq!(
        result.reply().body,
        br#"{"rt_cd":"0","msg1":"provider body must not cross the API","output":{}}"#
    );
    assert_eq!(result.reply().continuation, None);
    assert_eq!(result.metadata().kst_date(), "1970-01-01");
    assert_eq!(result.metadata().daily_attempt_ordinal(), 1);
    assert_eq!(result.metadata().reservation_fence(), 1);
    assert_eq!(result.metadata().received_at_ms(), 37);
    assert_eq!(harness.intraday.request_count(), 1);
    let request = &harness.intraday.requests()[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, PRICE_PATH);
    assert_eq!(request.tr_id, PRICE_TR);
    assert_eq!(
        request.query[0],
        ("FID_COND_MRKT_DIV_CODE".to_owned(), "J".to_owned())
    );
    assert_eq!(
        request.query[1],
        ("FID_INPUT_ISCD".to_owned(), "005930".to_owned())
    );
    assert!(!request.headers.contains_key("tr_cont"));
    assert!(format!("{result:?}").contains("<redacted>"));
    assert!(!format!("{result:?}").contains("provider body"));
}

#[tokio::test]
async fn wrong_channel_and_legacy_reject_before_auth_or_eligibility() {
    let harness = Harness::new("wrong-channel", [ok_response()]);
    let checked = Arc::new(AtomicUsize::new(0));
    let checked_by_callback = Arc::clone(&checked);
    let error = harness
        .client
        .get_intraday_attempt(DAILY_PATH, DAILY_TR, &[], move || async move {
            checked_by_callback.fetch_add(1, Ordering::SeqCst);
            Ok::<bool, &'static str>(true)
        })
        .await
        .expect_err("wrong channel");
    assert_eq!(error, IntradayAttemptError::UnsupportedEndpoint);
    assert_eq!(checked.load(Ordering::SeqCst), 0);
    assert_eq!(harness.issuer.calls(), 0);
    assert_eq!(harness.intraday.request_count(), 0);

    let legacy_transport = FakeTransport::new([ok_response()]);
    let legacy_probe = legacy_transport.clone();
    let clock = TestClock::at(0);
    let issuer = FixedIssuer::new(clock.clone());
    let issuer_trait: Arc<dyn TokenIssuer> = issuer.clone();
    let legacy = KisMarketDataClient::new(
        legacy_transport,
        NoSleep,
        Arc::new(TokenManager::new(Arc::new(clock.clone()), issuer_trait)),
        Arc::new(RateLimiter::new(Arc::new(clock), Quota::new(100, 100))),
        FixedCredentials,
        CredentialRef::env("KIS_APP_KEY"),
        CredentialRef::file("/run/secrets/kis_app_secret"),
    );
    let legacy_error = legacy
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], || async {
            Ok::<bool, &'static str>(true)
        })
        .await
        .expect_err("legacy shared boundary is required");
    assert_eq!(legacy_error, IntradayAttemptError::SharedReadRequired);
    assert_eq!(issuer.calls(), 0);
    assert_eq!(legacy_probe.request_count(), 0);
}

#[tokio::test]
async fn busy_shared_gate_is_nonblocking_and_does_not_run_the_callback() {
    let harness = Harness::new("busy", [ok_response()]);
    let _ = eligible_attempt(&harness).await.expect("prime state");
    harness.clock.advance_ms(5_000);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(harness.root.root.join(LOCK_FILE_NAME))
        .expect("stable lock");
    lock.try_lock_exclusive().expect("hold shared gate");
    let checked = Arc::new(AtomicUsize::new(0));
    let checked_by_callback = Arc::clone(&checked);
    let error = harness
        .client
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], move || async move {
            checked_by_callback.fetch_add(1, Ordering::SeqCst);
            Ok::<bool, &'static str>(true)
        })
        .await
        .expect_err("busy gate");
    assert_eq!(error, IntradayAttemptError::Busy);
    assert_eq!(checked.load(Ordering::SeqCst), 0);
    assert_eq!(harness.intraday.request_count(), 1);
    FileExt::unlock(&lock).expect("unlock shared gate");
}

#[tokio::test]
async fn caller_false_or_error_sends_no_get_and_retains_reservation_debt() {
    let harness = Harness::new("caller-false", [ok_response()]);
    let error = harness
        .client
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], || async {
            Ok::<bool, &'static str>(false)
        })
        .await
        .expect_err("caller denied");
    assert_eq!(error, IntradayAttemptError::CallerIneligible);
    assert_eq!(harness.intraday.request_count(), 0);
    let next = eligible_attempt(&harness).await.expect_err("debt remains");
    assert!(matches!(
        next,
        IntradayAttemptError::Coordination(ReadCoordinationError::ReservationActive { .. })
    ));

    let failed = Harness::new("caller-error", [ok_response()]);
    let error = failed
        .client
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], || async {
            Err::<bool, _>("database/provider prose must stay local")
        })
        .await
        .expect_err("caller error");
    assert_eq!(error, IntradayAttemptError::CallerEligibilityFailed);
    assert!(!error.to_string().contains("database/provider prose"));
    assert_eq!(failed.intraday.request_count(), 0);
}

#[tokio::test]
async fn delayed_or_cancelled_eligibility_never_crosses_dispatch() {
    let delayed = Harness::new("caller-delay", [ok_response()]);
    let clock = delayed.clock.clone();
    let error = delayed
        .client
        .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], move || async move {
            tokio::time::sleep(Duration::from_millis(260)).await;
            clock.advance_ms(260);
            Ok::<bool, &'static str>(true)
        })
        .await
        .expect_err("dispatch window");
    assert_eq!(error, IntradayAttemptError::DispatchWindowMissed);
    assert_eq!(delayed.intraday.request_count(), 0);
    assert!(matches!(
        eligible_attempt(&delayed).await,
        Err(IntradayAttemptError::Coordination(
            ReadCoordinationError::ReservationActive { .. }
        ))
    ));

    let cancelled = Harness::new("caller-cancel", [ok_response()]);
    let result = tokio::time::timeout(Duration::from_millis(30), {
        cancelled
            .client
            .get_intraday_attempt(PRICE_PATH, PRICE_TR, &[], || async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok::<bool, &'static str>(true)
            })
    })
    .await;
    assert!(result.is_err(), "eligibility future was cancelled");
    assert_eq!(cancelled.intraday.request_count(), 0);
    assert!(matches!(
        eligible_attempt(&cancelled).await,
        Err(IntradayAttemptError::Coordination(
            ReadCoordinationError::ReservationActive { .. }
        ))
    ));
}

#[tokio::test]
async fn unauthorized_rate_limited_and_timeout_are_single_get_attempts() {
    let unauthorized = Harness::new(
        "401",
        [TransportStep::Reply(Ok(HttpResponse::status(
            401,
            "secret body",
        )))],
    );
    let error = eligible_attempt(&unauthorized).await.expect_err("401");
    assert_eq!(error, IntradayAttemptError::Unauthorized);
    assert!(!error.to_string().contains("secret body"));
    assert_eq!(unauthorized.intraday.request_count(), 1);
    assert!(matches!(
        eligible_attempt(&unauthorized).await,
        Err(IntradayAttemptError::Coordination(
            ReadCoordinationError::GlobalSpacing { .. }
        ))
    ));
    assert_eq!(unauthorized.intraday.request_count(), 1);

    let limited = Harness::new("429", [rate_limited_response("9")]);
    let error = eligible_attempt(&limited).await.expect_err("429");
    assert_eq!(
        error,
        IntradayAttemptError::RateLimited {
            retry_after_ms: 9_000
        }
    );
    assert_eq!(limited.intraday.request_count(), 1);
    assert!(matches!(
        eligible_attempt(&limited).await,
        Err(IntradayAttemptError::Coordination(
            ReadCoordinationError::BrokerCooldown { .. }
        ))
    ));
    assert_eq!(limited.intraday.request_count(), 1);

    let timeout = Harness::new("timeout", [TransportStep::Hang]);
    let error = tokio::time::timeout(Duration::from_secs(4), eligible_attempt(&timeout))
        .await
        .expect("coordinator's dedicated three-second timeout")
        .expect_err("timeout");
    assert_eq!(error, IntradayAttemptError::Timeout);
    assert_eq!(timeout.intraday.request_count(), 1);
    assert!(matches!(
        eligible_attempt(&timeout).await,
        Err(IntradayAttemptError::Coordination(
            ReadCoordinationError::ReservationActive { .. }
        ))
    ));
}

#[tokio::test]
async fn date_rollover_resets_ordinal_and_clock_rollback_fails_closed() {
    let harness = Harness::new("dates", [ok_response(), ok_response()]);
    let first = eligible_attempt(&harness).await.expect("first day attempt");
    assert_eq!(first.metadata().kst_date(), "1970-01-01");
    assert_eq!(first.metadata().daily_attempt_ordinal(), 1);
    harness.clock.advance_ms(KST_DAY_MS);
    let second = eligible_attempt(&harness)
        .await
        .expect("next KST day attempt");
    assert_eq!(second.metadata().kst_date(), "1970-01-02");
    assert_eq!(second.metadata().daily_attempt_ordinal(), 1);
    assert_eq!(second.metadata().reservation_fence(), 2);
    harness.clock.advance_ms(-1);
    let error = eligible_attempt(&harness)
        .await
        .expect_err("clock rollback");
    assert_eq!(
        error,
        IntradayAttemptError::Coordination(ReadCoordinationError::ClockRollback)
    );
    assert_eq!(harness.intraday.request_count(), 2);
}

#[tokio::test]
async fn single_attempt_does_not_reuse_the_legacy_retry_loop() {
    let one = Harness::new(
        "new-one",
        [TransportStep::Reply(Ok(HttpResponse::status(
            503, "new one",
        )))],
    );
    let error = eligible_attempt(&one).await.expect_err("503");
    assert_eq!(error, IntradayAttemptError::ProviderUnavailable);
    assert_eq!(one.intraday.request_count(), 1);
}

#[tokio::test]
async fn budget_exhaustion_is_durable_and_next_kst_day_can_start_again() {
    let root = TempRoot::new("budget");
    let clock = TestClock::at(0);
    let issuer = FixedIssuer::new(clock.clone());
    let coordinator = ReadCoordinator::new(
        ReadCoordinationConfig::new(&root.root, root.uid),
        kis_client::read_coordination::ReadCredentials::new(
            Secret::new("fake-app-key".to_owned()),
            Secret::new("fake-app-secret".to_owned()),
            1,
        )
        .expect("credentials"),
        Arc::new(clock.clone()),
    );
    for _ in 0..5_000 {
        coordinator
            .execute_intraday(
                PRICE_PATH,
                PRICE_TR,
                issuer.as_ref(),
                kis_client::read_coordination::LockAcquisition::Bounded(Duration::from_secs(1)),
                Duration::from_secs(3),
                |_, dispatch| async move {
                    dispatch.begin_dispatch().expect("dispatch");
                    ReadCallbackResult::Success(())
                },
            )
            .await
            .expect("budget attempt");
        clock.advance_ms(5_000);
    }
    let exhausted = coordinator
        .execute_intraday(
            PRICE_PATH,
            PRICE_TR,
            issuer.as_ref(),
            kis_client::read_coordination::LockAcquisition::NonBlocking,
            Duration::from_secs(3),
            |_, _| async { ReadCallbackResult::Success(()) },
        )
        .await
        .expect_err("daily budget exhausted");
    assert_eq!(exhausted, ReadCoordinationError::IntradayBudgetExhausted);
    clock.advance_ms(KST_DAY_MS - 25_000_000);
    coordinator
        .execute_intraday(
            PRICE_PATH,
            PRICE_TR,
            issuer.as_ref(),
            kis_client::read_coordination::LockAcquisition::NonBlocking,
            Duration::from_secs(3),
            |_, dispatch| async move {
                dispatch.begin_dispatch().expect("next-day dispatch");
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect("next KST day resets the durable ledger");
}
