use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use kis_client::auth::{AccessToken, TokenIssuer};
use kis_client::clock::Clock;
use kis_client::error::KisError;
use kis_client::read_coordination::{
    LockAcquisition, ReadCoordinationConfig, ReadCoordinationError, ReadCoordinator,
    ReadCredentials,
};
use kis_client::secret::Secret;

static TEMP_NONCE: AtomicUsize = AtomicUsize::new(1);

pub struct PrivateTemp {
    base: PathBuf,
    root: PathBuf,
    uid: u32,
}

impl PrivateTemp {
    pub fn new(label: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "kis-read-coordination-{label}-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).expect("create test base");
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).expect("protect test base");
        let root = base.join("state");
        fs::create_dir(&root).expect("create state root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("protect state root");
        let uid = fs::metadata(&root).expect("state metadata").uid();
        Self { base, root, uid }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn coordinator(&self, clock: Arc<dyn Clock>, generation: u64) -> ReadCoordinator {
        coordinator_at(
            self.root.clone(),
            self.uid,
            clock,
            generation,
            "app-key",
            "app-secret",
        )
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

pub fn coordinator_at(
    root: PathBuf,
    uid: u32,
    clock: Arc<dyn Clock>,
    generation: u64,
    app_key: &str,
    app_secret: &str,
) -> ReadCoordinator {
    ReadCoordinator::new(
        ReadCoordinationConfig::new(root, uid),
        ReadCredentials::new(
            Secret::new(app_key.to_owned()),
            Secret::new(app_secret.to_owned()),
            generation,
        )
        .expect("valid fake credentials"),
        clock,
    )
}

#[derive(Clone)]
pub struct SettableClock(Arc<AtomicI64>);

impl SettableClock {
    pub fn at(now_ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now_ms)))
    }

    pub fn set(&self, now_ms: i64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, delta_ms: i64) {
        self.0.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for SettableClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct CountingIssuer {
    clock: SettableClock,
    calls: AtomicUsize,
    token_label: &'static str,
    advance_ms: i64,
}

impl CountingIssuer {
    pub fn new(clock: SettableClock) -> Self {
        Self {
            clock,
            calls: AtomicUsize::new(0),
            token_label: "fake-access-token",
            advance_ms: 0,
        }
    }

    pub fn advancing(clock: SettableClock, advance_ms: i64) -> Self {
        Self {
            clock,
            calls: AtomicUsize::new(0),
            token_label: "fake-access-token",
            advance_ms,
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl TokenIssuer for CountingIssuer {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.advance_ms != 0 {
            self.clock.advance(self.advance_ms);
        }
        Ok(AccessToken {
            value: Secret::new(self.token_label.to_owned()),
            expires_at_ms: self.clock.now_ms() + 3_600_000,
        })
    }
}

pub struct FailingIssuer {
    pub calls: AtomicUsize,
}

#[async_trait::async_trait]
impl TokenIssuer for FailingIssuer {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(KisError::Auth {
            reason: "synthetic issuer failure".to_owned(),
        })
    }
}

pub async fn prime_token(
    coordinator: &ReadCoordinator,
    issuer: &dyn TokenIssuer,
) -> Result<AccessToken, ReadCoordinationError> {
    coordinator
        .token(
            issuer,
            LockAcquisition::Bounded(std::time::Duration::from_secs(1)),
        )
        .await
}
