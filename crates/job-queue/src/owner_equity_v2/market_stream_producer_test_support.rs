//! Private synthetic transport, database, and PostgreSQL relay fixtures for
//! the owning-producer integration tests. All provider-shaped traffic is local
//! and uses non-secret sentinels.

#[path = "../../tests/owner_market_stream_boundary_support/mod.rs"]
pub(super) mod boundary;

use std::error::Error;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::{FixedOffset, NaiveDate, Utc};
use kis_client::MarketStreamClient;
use kis_client::clock::{Clock, TestClock};
use kis_client::market_stream::{
    MarketStreamCommandStateSnapshot, MarketStreamConfig, MarketStreamSession,
    MarketSubscriptionOperation,
};
use kis_client::market_stream_approval::ApprovalClient;
use kis_client::market_stream_state::MarketStreamDomain;
use kis_client::market_stream_wire::MarketStreamSessionProof;
use kis_client::secret::Secret;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, oneshot};
use tokio::task::JoinSet;

const CLUSTER_SOCKET: &str = "/tmp/lagrange-kis-stream-20260921/pg-local/socket";
const CLUSTER_PORT: u16 = 55449;
const MAX_PG_FRAME_BYTES: usize = 16 * 1024 * 1024;

type TestError = Box<dyn Error + Send + Sync>;
const TASK_JOIN_TIMEOUT: Duration = Duration::from_secs(8);

tokio::task_local! {
    static FIXTURE_OWNER: FixtureOwner;
}

pub(super) struct FixtureOwner {
    controls: Arc<Mutex<Vec<Arc<TaskControl>>>>,
    cleanup: Arc<AsyncMutex<()>>,
    #[cfg(test)]
    cleanup_attempted: Arc<AtomicUsize>,
    #[cfg(test)]
    cleanup_acquired: Arc<AtomicUsize>,
    #[cfg(test)]
    cleanup_changed: Arc<Notify>,
    case_owner: bool,
}

impl Clone for FixtureOwner {
    fn clone(&self) -> Self {
        // Clones cross task-local/setup boundaries only.  They retain access
        // to registration and explicit cleanup, but are never the case's
        // best-effort Drop owner.
        Self {
            controls: self.controls.clone(),
            cleanup: self.cleanup.clone(),
            #[cfg(test)]
            cleanup_attempted: self.cleanup_attempted.clone(),
            #[cfg(test)]
            cleanup_acquired: self.cleanup_acquired.clone(),
            #[cfg(test)]
            cleanup_changed: self.cleanup_changed.clone(),
            case_owner: false,
        }
    }
}

impl FixtureOwner {
    pub(super) fn new() -> Self {
        Self {
            controls: Arc::new(Mutex::new(Vec::new())),
            cleanup: Arc::new(AsyncMutex::new(())),
            #[cfg(test)]
            cleanup_attempted: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            cleanup_acquired: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            cleanup_changed: Arc::new(Notify::new()),
            case_owner: true,
        }
    }

    fn register(&self, control: Arc<TaskControl>) {
        self.controls
            .lock()
            .expect("fixture owner lock")
            .push(control);
    }

    pub(super) async fn cleanup(&self) -> Result<(), TestError> {
        self.cleanup_with_timeout(TASK_JOIN_TIMEOUT).await
    }

    async fn cleanup_with_timeout(&self, timeout: Duration) -> Result<(), TestError> {
        // The async permit makes the entire pass cancellation-safe: cancelling
        // a cleaner releases only this permit, never registry ownership.
        #[cfg(test)]
        {
            self.cleanup_attempted.fetch_add(1, Ordering::AcqRel);
            self.cleanup_changed.notify_waiters();
        }
        let _cleanup = self.cleanup.lock().await;
        #[cfg(test)]
        {
            self.cleanup_acquired.fetch_add(1, Ordering::AcqRel);
            self.cleanup_changed.notify_waiters();
        }
        let mut first_error = None;
        loop {
            let controls = { self.controls.lock().expect("fixture owner lock").clone() };
            if controls.is_empty() {
                break;
            }
            let mut removed_any = false;
            let mut retained_any = false;
            for control in &controls {
                control.abort();
                if let Err(error) = control.join_with_timeout("fixture cleanup", timeout).await {
                    first_error.get_or_insert(error);
                }
                if control.is_fully_observed() {
                    self.controls
                        .lock()
                        .expect("fixture owner lock")
                        .retain(|candidate| !Arc::ptr_eq(candidate, control));
                    removed_any = true;
                } else {
                    retained_any = true;
                }
            }
            // A timed-out control remains registered and discoverable.  Do
            // not turn one cleanup call into an unbounded retry loop.
            if retained_any || !removed_any {
                break;
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for FixtureOwner {
    fn drop(&mut self) {
        // Task-local and setup clones are borrowed registration handles, not
        // owners.  A designated case owner alone may request best-effort
        // cancellation; deterministic observation remains `cleanup()`.
        if !self.case_owner {
            return;
        }
        let controls = self.controls.lock().expect("fixture owner lock").clone();
        for control in controls {
            control.abort();
        }
    }
}

#[cfg(test)]
mod fixture_owner_unit_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropCount(Arc<AtomicUsize>);

    impl Drop for DropCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn tracked_child(
        descendants: &Arc<DescendantTracker>,
    ) -> (
        tokio::task::JoinHandle<()>,
        oneshot::Receiver<()>,
        oneshot::Sender<()>,
        Arc<AtomicUsize>,
    ) {
        let guard = descendants.child();
        let drops = Arc::new(AtomicUsize::new(0));
        let sentinel = drops.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _guard = guard;
            let _sentinel = DropCount(sentinel);
            let _ = ready_tx.send(());
            let _ = release_rx.await;
        });
        (handle, ready_rx, release_tx, drops)
    }

    async fn wait_for_counter(counter: &AtomicUsize, changed: &Notify, target: usize) {
        loop {
            let notified = changed.notified();
            if counter.load(Ordering::Acquire) >= target {
                return;
            }
            notified.await;
        }
    }

    #[tokio::test]
    async fn pending_task_survives_temporary_and_task_local_owner_drops() {
        let owner = FixtureOwner::new();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let task = with_fixture_owner(owner.clone(), async move {
            spawn_owned(
                async move {
                    let _ = ready_tx.send(());
                    let _ = release_rx.await;
                    47_u8
                },
                current_fixture_owner(),
                None,
            )
        })
        .await;
        ready_rx.await.unwrap();
        drop(owner.clone());
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        release_tx.send(()).unwrap();
        let mut task = task;
        assert_eq!(task.join_result("gated value").await.unwrap(), 47);
        assert!(owner.cleanup().await.is_ok());
        assert!(owner.controls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_direct_join_retains_main_handle_for_owner_cleanup() {
        let owner = FixtureOwner::new();
        let drops = Arc::new(AtomicUsize::new(0));
        let sentinel = drops.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = spawn_owned(
            async move {
                let _sentinel = DropCount(sentinel);
                let _ = ready_tx.send(());
                std::future::pending::<()>().await;
            },
            Some(owner.clone()),
            None,
        );
        ready_rx.await.unwrap();
        let control = task.control.clone();
        let at_join = control.join_waiting.notified();
        let joining_control = control.clone();
        let joining = tokio::spawn(async move { joining_control.join("cancelled join").await });
        at_join.await;
        joining.abort();
        assert!(joining.await.unwrap_err().is_cancelled());
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        assert!(owner.cleanup().await.is_ok());
        assert_eq!(drops.load(Ordering::Acquire), 1);
        drop(task);
    }

    #[tokio::test]
    async fn panic_join_waits_for_child_and_repeated_cleanup_is_deterministic() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, child_drops) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let task = spawn_owned(
            async move { panic!("fixture-owner terminal panic") },
            Some(owner.clone()),
            Some(descendants),
        );
        let control = task.control.clone();
        let at_descendants = control.descendants_waiting.notified();
        let joining = tokio::spawn(async move {
            let mut task = task;
            task.join_result("panicking parent").await
        });
        at_descendants.await;
        assert_eq!(child_drops.load(Ordering::Acquire), 0);
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(joining.await.unwrap().is_err());
        assert_eq!(child_drops.load(Ordering::Acquire), 1);
        assert!(owner.cleanup().await.is_err());
        assert!(owner.cleanup().await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_cleanup_retains_parent_and_child_for_retry() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, child_drops) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let parent_drops = Arc::new(AtomicUsize::new(0));
        let parent_sentinel = parent_drops.clone();
        let _task = spawn_owned(
            async move {
                let _sentinel = DropCount(parent_sentinel);
            },
            Some(owner.clone()),
            Some(descendants),
        );
        let control = owner.controls.lock().unwrap()[0].clone();
        let at_descendants = control.descendants_waiting.notified();
        let cleaning_owner = owner.clone();
        let cleaning = tokio::spawn(async move { cleaning_owner.cleanup().await });
        at_descendants.await;
        assert_eq!(parent_drops.load(Ordering::Acquire), 1);
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        cleaning.abort();
        assert!(cleaning.await.unwrap_err().is_cancelled());
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(owner.cleanup().await.is_ok());
        assert_eq!(child_drops.load(Ordering::Acquire), 1);
        assert!(owner.controls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn caught_outer_body_panic_leaves_live_fixture_work_for_cleanup() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, child_drops) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let (task_tx, task_rx) = oneshot::channel();
        let scoped_owner = owner.clone();
        let outer = tokio::spawn(async move {
            with_fixture_owner(scoped_owner, async move {
                let task = spawn_owned(
                    async move { panic!("registered fixture panic") },
                    current_fixture_owner(),
                    Some(descendants),
                );
                task_tx.send(task).ok().expect("outer returns owned task");
                panic!("outer fixture body panic");
            })
            .await;
        });
        let task = task_rx.await.unwrap();
        assert!(outer.await.unwrap_err().is_panic());
        let at_descendants = task.control.descendants_waiting.notified();
        let cleaning_owner = owner.clone();
        let cleaning = tokio::spawn(async move { cleaning_owner.cleanup().await });
        at_descendants.await;
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(cleaning.await.unwrap().is_err());
        assert_eq!(child_drops.load(Ordering::Acquire), 1);
        assert!(owner.cleanup().await.is_ok());
        drop(task);
    }

    #[tokio::test]
    async fn parent_terminal_boundary_causally_precedes_descendant_release() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, child_drops) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let parent_drops = Arc::new(AtomicUsize::new(0));
        let parent_sentinel = parent_drops.clone();
        let task = spawn_owned(
            async move {
                let _sentinel = DropCount(parent_sentinel);
            },
            Some(owner.clone()),
            Some(descendants),
        );
        let control = task.control.clone();
        let at_descendants = control.descendants_waiting.notified();
        let joining_control = control.clone();
        let joining = tokio::spawn(async move { joining_control.join("causal child").await });
        at_descendants.await;
        assert_eq!(parent_drops.load(Ordering::Acquire), 1);
        assert_eq!(child_drops.load(Ordering::Acquire), 0);
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(joining.await.unwrap().is_ok());
        assert_eq!(child_drops.load(Ordering::Acquire), 1);
        assert!(owner.cleanup().await.is_ok());
        drop(task);
    }

    #[tokio::test]
    async fn descendant_timeout_stays_registered_and_retry_succeeds() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, child_drops) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let _task = spawn_owned(async {}, Some(owner.clone()), Some(descendants));
        assert!(
            owner
                .cleanup_with_timeout(Duration::from_millis(20))
                .await
                .is_err()
        );
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(owner.cleanup().await.is_ok());
        assert_eq!(child_drops.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn post_abort_pending_handle_stays_registered_for_retry() -> Result<(), TestError> {
        struct BlockingDrop {
            entered: Option<oneshot::Sender<()>>,
            release: Arc<(Mutex<bool>, std::sync::Condvar)>,
        }

        struct BlockingDropRelease(Arc<(Mutex<bool>, std::sync::Condvar)>);

        impl BlockingDropRelease {
            fn release(&self) {
                let (released, changed) = &*self.0;
                let mut released = released
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *released = true;
                changed.notify_all();
            }
        }

        impl Drop for BlockingDropRelease {
            fn drop(&mut self) {
                self.release();
            }
        }

        impl Drop for BlockingDrop {
            fn drop(&mut self) {
                if let Some(entered) = self.entered.take() {
                    let _ = entered.send(());
                }
                let (released, changed) = &*self.release;
                let mut released = released
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while !*released {
                    released = changed
                        .wait(released)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }

        async fn fail_after_bounded_drain(
            reason: String,
            release: &BlockingDropRelease,
            cleaning: &mut Option<tokio::task::JoinHandle<Result<(), TestError>>>,
            owner: &FixtureOwner,
            observation_timeout: Duration,
        ) -> TestError {
            release.release();
            let mut details = reason;
            if let Some(mut cleaning) = cleaning.take() {
                cleaning.abort();
                match tokio::time::timeout(observation_timeout, &mut cleaning).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) if error.is_cancelled() => {}
                    Ok(Err(error)) => {
                        details.push_str(&format!("; cleanup helper join failed: {error}"));
                    }
                    Err(_) => details.push_str("; cleanup helper did not terminate after abort"),
                }
            }
            match tokio::time::timeout(observation_timeout, owner.cleanup()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => details.push_str(&format!("; fixture drain failed: {error}")),
                Err(_) => details.push_str("; fixture drain timed out"),
            }
            details.into()
        }

        let observation_timeout = Duration::from_secs(2);
        let owner = FixtureOwner::new();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (entered_tx, entered_rx) = oneshot::channel();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let release_guard = BlockingDropRelease(release.clone());
        let task_release = release.clone();
        let _task = spawn_owned(
            async move {
                let _sentinel = BlockingDrop {
                    entered: Some(entered_tx),
                    release: task_release,
                };
                let _ = ready_tx.send(());
                std::future::pending::<()>().await;
            },
            Some(owner.clone()),
            None,
        );
        let mut cleaning = None;
        match tokio::time::timeout(observation_timeout, ready_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(fail_after_bounded_drain(
                    "blocking task readiness channel closed".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
            Err(_) => {
                return Err(fail_after_bounded_drain(
                    "blocking task readiness timed out".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
        }
        let cleaning_owner = owner.clone();
        cleaning = Some(tokio::spawn(async move {
            cleaning_owner
                .cleanup_with_timeout(Duration::from_millis(20))
                .await
        }));
        match tokio::time::timeout(observation_timeout, entered_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(fail_after_bounded_drain(
                    "blocking destructor entry channel closed".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
            Err(_) => {
                return Err(fail_after_bounded_drain(
                    "blocking destructor entry timed out".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
        }
        let Some(mut cleaning_handle) = cleaning.take() else {
            return Err(fail_after_bounded_drain(
                "cleanup helper ownership was lost".to_owned(),
                &release_guard,
                &mut cleaning,
                &owner,
                observation_timeout,
            )
            .await);
        };
        match tokio::time::timeout(observation_timeout, &mut cleaning_handle).await {
            Ok(Ok(Err(_))) => {}
            Ok(Ok(Ok(()))) => {
                return Err(fail_after_bounded_drain(
                    "cleanup unexpectedly observed the blocked task".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
            Ok(Err(error)) => {
                return Err(fail_after_bounded_drain(
                    format!("cleanup helper join failed: {error}"),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
            Err(_) => {
                cleaning = Some(cleaning_handle);
                return Err(fail_after_bounded_drain(
                    "cleanup helper observation timed out".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
        }
        let registered = owner
            .controls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        if registered != 1 {
            return Err(fail_after_bounded_drain(
                format!("expected one retained control, found {registered}"),
                &release_guard,
                &mut cleaning,
                &owner,
                observation_timeout,
            )
            .await);
        }
        release_guard.release();
        match tokio::time::timeout(observation_timeout, owner.cleanup()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(fail_after_bounded_drain(
                    format!("fixture cleanup retry failed: {error}"),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
            Err(_) => {
                return Err(fail_after_bounded_drain(
                    "fixture cleanup retry timed out".to_owned(),
                    &release_guard,
                    &mut cleaning,
                    &owner,
                    observation_timeout,
                )
                .await);
            }
        }
        if !owner
            .controls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
        {
            return Err(fail_after_bounded_drain(
                "fixture registry was not empty after retry".to_owned(),
                &release_guard,
                &mut cleaning,
                &owner,
                observation_timeout,
            )
            .await);
        }
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_cleanup_waiters_serialize_without_losing_registry() {
        let owner = FixtureOwner::new();
        let descendants = DescendantTracker::new();
        let (child, child_ready, release_child, _) = tracked_child(&descendants);
        child_ready.await.unwrap();
        let task = spawn_owned(async {}, Some(owner.clone()), Some(descendants));
        let boundary = task.control.descendants_waiting.notified();
        let first_owner = owner.clone();
        let first = tokio::spawn(async move { first_owner.cleanup().await });
        boundary.await;
        let second_owner = owner.clone();
        let second = tokio::spawn(async move { second_owner.cleanup().await });
        wait_for_counter(&owner.cleanup_attempted, &owner.cleanup_changed, 2).await;
        assert_eq!(owner.cleanup_acquired.load(Ordering::Acquire), 1);
        assert_eq!(owner.controls.lock().unwrap().len(), 1);
        release_child.send(()).unwrap();
        child.await.unwrap();
        assert!(first.await.unwrap().is_ok());
        assert!(second.await.unwrap().is_ok());
        assert_eq!(owner.cleanup_acquired.load(Ordering::Acquire), 2);
        assert!(owner.controls.lock().unwrap().is_empty());
        drop(task);
    }
}

pub(super) async fn with_fixture_owner<F, R>(owner: FixtureOwner, future: F) -> R
where
    F: Future<Output = R>,
{
    FIXTURE_OWNER.scope(owner, future).await
}

fn current_fixture_owner() -> Option<FixtureOwner> {
    FIXTURE_OWNER.try_with(Clone::clone).ok()
}

struct DescendantTracker {
    active: AtomicUsize,
    changed: Notify,
}

impl DescendantTracker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            active: AtomicUsize::new(0),
            changed: Notify::new(),
        })
    }

    fn child(self: &Arc<Self>) -> DescendantGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        DescendantGuard(self.clone())
    }

    async fn wait_empty_with_timeout(
        &self,
        label: &str,
        timeout: Duration,
    ) -> Result<(), TestError> {
        tokio::time::timeout(timeout, async {
            loop {
                // Register before checking so a child that completes between
                // the load and the await cannot lose its only notification.
                let notified = self.changed.notified();
                if self.active.load(Ordering::Acquire) == 0 {
                    return;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| format!("{label} descendants did not finish"))?;
        Ok(())
    }
}

struct DescendantGuard(Arc<DescendantTracker>);

impl Drop for DescendantGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_waiters();
    }
}

struct TaskControl {
    abort: tokio::task::AbortHandle,
    task: AsyncMutex<TaskState>,
    descendants: Option<Arc<DescendantTracker>>,
    fully_observed: std::sync::atomic::AtomicBool,
    join_waiting: Notify,
    descendants_waiting: Notify,
}

struct TaskState {
    handle: Option<tokio::task::JoinHandle<()>>,
    terminal: Option<TaskTerminal>,
}

#[derive(Clone)]
enum TaskTerminal {
    Completed,
    Cancelled,
    Failed(Arc<str>),
}

impl TaskControl {
    fn abort(&self) {
        self.abort.abort();
    }

    async fn join(&self, label: &str) -> Result<(), TestError> {
        self.join_with_timeout(label, TASK_JOIN_TIMEOUT).await
    }

    async fn join_with_timeout(&self, label: &str, timeout: Duration) -> Result<(), TestError> {
        let terminal = {
            let mut task = self.task.lock().await;
            if let Some(terminal) = task.terminal.clone() {
                terminal
            } else {
                let handle = task.handle.as_mut().expect("live task has a join handle");
                // Awaiting &mut JoinHandle is cancellation-safe.  If this
                // join future is cancelled, the handle remains in TaskState.
                self.join_waiting.notify_waiters();
                let joined = match tokio::time::timeout(timeout, &mut *handle).await {
                    Ok(joined) => joined,
                    Err(_) => {
                        self.abort();
                        match tokio::time::timeout(timeout, &mut *handle).await {
                            Ok(joined) => joined,
                            Err(_) => {
                                return Err(
                                    format!("{label} task did not terminate after abort").into()
                                );
                            }
                        }
                    }
                };
                let terminal = match joined {
                    Ok(()) => TaskTerminal::Completed,
                    Err(error) if error.is_cancelled() => TaskTerminal::Cancelled,
                    Err(error) => TaskTerminal::Failed(
                        format!("task join failed: {error}").into_boxed_str().into(),
                    ),
                };
                // A ready JoinHandle is consumed exactly once.  Cache its
                // classification before any cancellation point below.
                task.handle.take();
                task.terminal = Some(terminal.clone());
                terminal
            }
        };

        if let Some(descendants) = &self.descendants {
            self.descendants_waiting.notify_waiters();
            descendants.wait_empty_with_timeout(label, timeout).await?;
        }
        self.fully_observed.store(true, Ordering::Release);
        match terminal {
            TaskTerminal::Completed | TaskTerminal::Cancelled => Ok(()),
            TaskTerminal::Failed(error) => Err(format!("{label} {error}").into()),
        }
    }

    fn is_fully_observed(&self) -> bool {
        self.fully_observed.load(Ordering::Acquire)
    }
}

struct OwnedTask<T> {
    control: Arc<TaskControl>,
    result: Option<oneshot::Receiver<T>>,
}

impl<T> OwnedTask<T> {
    fn abort(&self) {
        self.control.abort();
    }

    async fn join_result(&mut self, label: &str) -> Result<T, TestError> {
        self.control.join(label).await?;
        self.result
            .take()
            .ok_or_else(|| format!("{label} result was already joined"))?
            .await
            .map_err(|_| format!("{label} result channel closed").into())
    }
}

impl<T> Drop for OwnedTask<T> {
    fn drop(&mut self) {
        self.control.abort();
    }
}

fn spawn_owned<F, T>(
    future: F,
    owner: Option<FixtureOwner>,
    descendants: Option<Arc<DescendantTracker>>,
) -> OwnedTask<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (result_tx, result_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let result = future.await;
        let _ = result_tx.send(result);
    });
    let abort = handle.abort_handle();
    let control = Arc::new(TaskControl {
        abort,
        task: AsyncMutex::new(TaskState {
            handle: Some(handle),
            terminal: None,
        }),
        descendants,
        fully_observed: std::sync::atomic::AtomicBool::new(false),
        join_waiting: Notify::new(),
        descendants_waiting: Notify::new(),
    });
    if let Some(owner) = owner {
        owner.register(control.clone());
    }
    OwnedTask {
        control,
        result: Some(result_rx),
    }
}

pub(super) struct CommandPlan {
    symbol: String,
    operation: MarketSubscriptionOperation,
    events_after_ack: Vec<(String, u32)>,
    ack_gate: Option<ServerAckGate>,
}

impl CommandPlan {
    pub(super) fn new(
        symbol: &str,
        operation: MarketSubscriptionOperation,
        events_after_ack: Vec<(String, u32)>,
    ) -> Self {
        Self {
            symbol: symbol.to_owned(),
            operation,
            events_after_ack,
            ack_gate: None,
        }
    }

    pub(super) fn paused_ack(
        symbol: &str,
        operation: MarketSubscriptionOperation,
        events_after_ack: Vec<(String, u32)>,
    ) -> (Self, AckPause) {
        let (reached_tx, reached_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let mut plan = Self::new(symbol, operation, events_after_ack);
        plan.ack_gate = Some(ServerAckGate {
            reached: reached_tx,
            release: release_rx,
        });
        (
            plan,
            AckPause {
                reached: reached_rx,
                release: Some(release_tx),
            },
        )
    }
}

struct ServerAckGate {
    reached: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

pub(super) struct AckPause {
    reached: oneshot::Receiver<()>,
    release: Option<oneshot::Sender<()>>,
}

impl AckPause {
    pub(super) async fn wait_until_reached(&mut self) -> Result<(), TestError> {
        tokio::time::timeout(Duration::from_secs(5), &mut self.reached)
            .await
            .map_err(|_| "synthetic ACK pause was not reached")??;
        Ok(())
    }

    pub(super) fn release(mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoopbackSummary {
    pub commands: Vec<(String, MarketSubscriptionOperation)>,
    pub market_record_batches: Vec<u8>,
    pub client_closed: bool,
}

pub(super) struct LoopbackHarness {
    session: Option<MarketStreamSession>,
    command_rx: mpsc::UnboundedReceiver<CommandMetadataOwned>,
    approval_task: OwnedTask<Result<(), String>>,
    websocket_task: OwnedTask<Result<LoopbackSummary, String>>,
    _directory: TempDir,
    domain: MarketStreamDomain,
    pub clock: Arc<TestClock>,
    session_date: NaiveDate,
    clock_origin: FixtureClockOrigin,
    clock_anchor: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandMetadataOwned {
    symbol: String,
    operation: MarketSubscriptionOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FixtureClockOrigin {
    exact_database_sample_ms: i64,
    synthetic_socket_anchor_ms: i64,
}

impl FixtureClockOrigin {
    fn from_database_sample(
        session_date: NaiveDate,
        exact_database_sample_ms: i64,
    ) -> Result<Self, String> {
        let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
        let sample_date = chrono::DateTime::<Utc>::from_timestamp_millis(exact_database_sample_ms)
            .ok_or("fixture database sample timestamp invalid")?
            .with_timezone(&kst)
            .date_naive();
        if sample_date != session_date {
            return Err("fixture database sample is outside its session date".to_owned());
        }
        let synthetic_socket_anchor_ms = exact_database_sample_ms
            .checked_sub(exact_database_sample_ms.rem_euclid(1_000))
            .ok_or("fixture clock anchor invalid")?;
        let anchor_date =
            chrono::DateTime::<Utc>::from_timestamp_millis(synthetic_socket_anchor_ms)
                .ok_or("fixture clock anchor timestamp invalid")?
                .with_timezone(&kst)
                .date_naive();
        if anchor_date != session_date {
            return Err("fixture clock anchor is outside its session date".to_owned());
        }
        Ok(Self {
            exact_database_sample_ms,
            synthetic_socket_anchor_ms,
        })
    }

    fn post_ack_target_ms(
        self,
        session_date: NaiveDate,
        elapsed_ms: i64,
        current_ms: i64,
    ) -> Result<i64, String> {
        if elapsed_ms < 0 {
            return Err("fixture clock elapsed time is negative".to_owned());
        }
        if current_ms < self.synthetic_socket_anchor_ms {
            return Err("fixture clock moved before its socket anchor".to_owned());
        }
        let modeled_database_upper_ms = self
            .exact_database_sample_ms
            .checked_add(elapsed_ms)
            .ok_or("fixture modeled database time invalid")?;
        if current_ms > modeled_database_upper_ms {
            return Err("fixture clock leads its modeled database time".to_owned());
        }
        let target_ms = synthetic_post_ack_target_ms(
            session_date,
            self.synthetic_socket_anchor_ms,
            elapsed_ms,
            current_ms,
        )?;
        if target_ms > modeled_database_upper_ms {
            return Err("fixture clock target leads its modeled database time".to_owned());
        }
        Ok(target_ms)
    }
}

impl LoopbackHarness {
    /// Only between completed reconciliation calls, with no ACK in flight.
    /// This is a bounded fixture timer, not a producer retry or synthetic jump.
    pub(super) async fn wait_for_command_eligibility(
        &self,
        not_before_ms: i64,
    ) -> Result<(), TestError> {
        let elapsed_ms = i64::try_from(self.clock_anchor.elapsed().as_millis())?;
        let remaining = eligibility_remaining(
            self.clock_origin, self.session_date, self.clock.now_ms(),
            elapsed_ms, not_before_ms,
        )?;
        if remaining > Duration::from_secs(5) {
            return Err("fixture eligibility exceeds existing five-second bound".into());
        }
        if !remaining.is_zero() {
            tokio::time::sleep(remaining).await;
        }
        advance_clock_from_database_sample(
            &self.clock, self.session_date, self.clock_origin, self.clock_anchor,
        )?;
        if self.clock.now_ms() < not_before_ms {
            return Err("fixture eligibility was not reached".into());
        }
        Ok(())
    }

    pub(super) fn take_session(&mut self) -> MarketStreamSession {
        self.session
            .take()
            .expect("loopback C2 session is transferred exactly once")
    }

    pub(super) async fn next_command(
        &mut self,
    ) -> Result<(String, MarketSubscriptionOperation), TestError> {
        let command = tokio::time::timeout(Duration::from_secs(5), self.command_rx.recv())
            .await
            .map_err(|_| "synthetic WebSocket command was not observed")?
            .ok_or("synthetic WebSocket command channel closed")?;
        Ok((command.symbol, command.operation))
    }

    pub(super) fn try_next_command(&mut self) -> Option<(String, MarketSubscriptionOperation)> {
        self.command_rx
            .try_recv()
            .ok()
            .map(|command| (command.symbol, command.operation))
    }

    pub(super) fn command_state_snapshot(
        &self,
    ) -> Result<MarketStreamCommandStateSnapshot, TestError> {
        Ok(self.domain.test_command_state_snapshot()?)
    }

    pub(super) async fn finish(mut self) -> Result<LoopbackSummary, TestError> {
        drop(self.session.take());
        let websocket = self
            .websocket_task
            .join_result("synthetic WebSocket server")
            .await;
        let approval = self
            .approval_task
            .join_result("synthetic approval server")
            .await;
        drop(self._directory);
        let summary = websocket?.map_err(|_| "synthetic WebSocket server failed")?;
        approval?.map_err(|_| "synthetic approval server failed")?;
        Ok(summary)
    }
}

pub(super) async fn loopback_session(
    session_date: NaiveDate,
    credential_slot_id: uuid::Uuid,
    now_ms: i64,
    plans: Vec<CommandPlan>,
    allow_early_close: bool,
) -> Result<LoopbackHarness, TestError> {
    let clock_origin = FixtureClockOrigin::from_database_sample(session_date, now_ms)?;
    let directory = tempfile::tempdir()?;
    let domain = MarketStreamDomain::for_test(directory.path(), credential_slot_id)?;
    let approval_listener = TcpListener::bind("127.0.0.1:0").await?;
    let approval_port = approval_listener.local_addr()?.port();
    let websocket_listener = TcpListener::bind("127.0.0.1:0").await?;
    let websocket_port = websocket_listener.local_addr()?.port();
    let clock = Arc::new(TestClock::at(clock_origin.synthetic_socket_anchor_ms));
    let clock_anchor = Instant::now();
    let approval = ApprovalClient::for_loopback(
        &format!("http://127.0.0.1:{approval_port}"),
        Secret::new("C3B_SYNTHETIC_APPKEY".to_owned()),
        Secret::new("C3B_SYNTHETIC_SECRET".to_owned()),
        domain.clone(),
        "7",
    )?
    .with_clock(clock.clone());
    let session_date_wire = session_date.format("%Y%m%d").to_string().parse::<u32>()?;
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
    let midnight_ms = session_date
        .and_hms_opt(0, 0, 0)
        .ok_or("KST midnight invalid")?
        .and_local_timezone(kst)
        .single()
        .ok_or("KST midnight ambiguous")?
        .timestamp_millis();
    let proof = MarketStreamSessionProof::new(session_date_wire, 0, 240_000, midnight_ms)?;
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let server_clock = clock.clone();
    let config = MarketStreamConfig::loopback(
        approval,
        &format!("ws://127.0.0.1:{websocket_port}/tryitout"),
    )?
    .with_session_proof(proof)
    .with_ack_timeout(Duration::from_secs(5))
    .with_clock(clock.clone());
    let owner = current_fixture_owner();
    let approval_task = spawn_owned(run_approval_server(approval_listener), owner.clone(), None);
    let websocket_task = spawn_owned(
        run_websocket_server(
            websocket_listener,
            session_date,
            server_clock,
            clock_origin,
            clock_anchor,
            plans,
            command_tx,
            allow_early_close,
        ),
        owner,
        None,
    );
    let session = tokio::time::timeout(
        Duration::from_secs(10),
        MarketStreamClient::new(config).connect(),
    )
    .await;
    let session = match session {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            approval_task.control.abort();
            websocket_task.control.abort();
            let _ = approval_task.control.join("approval setup cleanup").await;
            let _ = websocket_task.control.join("WebSocket setup cleanup").await;
            return Err(format!("synthetic C2 connection failed: {error}").into());
        }
        Err(_) => {
            approval_task.control.abort();
            websocket_task.control.abort();
            let _ = approval_task
                .control
                .join("approval setup timeout cleanup")
                .await;
            let _ = websocket_task
                .control
                .join("WebSocket setup timeout cleanup")
                .await;
            return Err("synthetic C2 connection timed out".into());
        }
    };
    let harness = LoopbackHarness {
        session: Some(session),
        command_rx,
        approval_task,
        websocket_task,
        _directory: directory,
        domain,
        clock,
        session_date,
        clock_origin,
        clock_anchor,
    };
    Ok(harness)
}

async fn run_approval_server(listener: TcpListener) -> Result<(), String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|_| "approval accept failed")?;
    let request = read_http_headers(&mut stream)
        .await
        .map_err(|_| "approval request read failed")?;
    let length = header_value(&request, "content-length")
        .ok_or("approval content length missing")?
        .parse::<usize>()
        .map_err(|_| "approval content length invalid")?;
    if length > 4096 {
        return Err("synthetic approval request length invalid".to_owned());
    }
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|_| "approval body read failed")?;
    let response_body = br#"{"approval_key":"C3B_SYNTHETIC_APPROVAL"}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|_| "approval response write failed")?;
    stream
        .write_all(response_body)
        .await
        .map_err(|_| "approval response body write failed")?;
    Ok(())
}

async fn run_websocket_server(
    listener: TcpListener,
    session_date: NaiveDate,
    clock: Arc<TestClock>,
    clock_origin: FixtureClockOrigin,
    clock_anchor: Instant,
    plans: Vec<CommandPlan>,
    command_tx: mpsc::UnboundedSender<CommandMetadataOwned>,
    allow_early_close: bool,
) -> Result<LoopbackSummary, String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|_| "WebSocket accept failed")?;
    let request = read_http_headers(&mut stream)
        .await
        .map_err(|_| "WebSocket handshake read failed")?;
    let key = header_value(&request, "sec-websocket-key").ok_or("WebSocket key missing")?;
    let mut digest = sha1_smol::Sha1::new();
    digest.update(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    let accept = base64::engine::general_purpose::STANDARD.encode(digest.digest().bytes());
    stream
        .write_all(
            format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .map_err(|_| "WebSocket handshake response failed")?;

    let mut commands = Vec::with_capacity(plans.len());
    let mut market_record_batches = Vec::new();
    for plan in plans {
        let (opcode, payload) = match read_ws_frame(&mut stream).await {
            Ok(frame) => frame,
            Err(_) if allow_early_close => {
                return Ok(LoopbackSummary {
                    commands,
                    market_record_batches,
                    client_closed: true,
                });
            }
            Err(_) => return Err("client closed before all planned commands".to_owned()),
        };
        if opcode != 0x1 {
            return Err("synthetic client command was not a text frame".to_owned());
        }
        let command: serde_json::Value =
            serde_json::from_slice(&payload).map_err(|_| "synthetic command JSON invalid")?;
        let symbol = command["body"]["input"]["tr_key"]
            .as_str()
            .ok_or("synthetic command symbol missing")?
            .to_owned();
        let operation = match command["header"]["tr_type"].as_str() {
            Some("1") => MarketSubscriptionOperation::Subscribe,
            Some("2") => MarketSubscriptionOperation::Unsubscribe,
            _ => return Err("synthetic command operation invalid".to_owned()),
        };
        if symbol != plan.symbol || operation != plan.operation {
            return Err("synthetic command did not match its bounded plan".to_owned());
        }
        commands.push((symbol.clone(), operation));
        let _ = command_tx.send(CommandMetadataOwned {
            symbol: symbol.clone(),
            operation,
        });
        if let Some(gate) = plan.ack_gate {
            let _ = gate.reached.send(());
            let _ = tokio::time::timeout(Duration::from_secs(5), gate.release).await;
        }
        let acknowledgement = match operation {
            MarketSubscriptionOperation::Subscribe => "SUBSCRIBE SUCCESS",
            MarketSubscriptionOperation::Unsubscribe => "UNSUBSCRIBE SUCCESS",
        };
        let ack = serde_json::to_vec(&serde_json::json!({
            "header": {"tr_id": "H0STCNT0", "tr_key": symbol, "encrypt": "N"},
            "body": {"rt_cd": "0", "msg_cd": "OPSP0000", "msg1": acknowledgement}
        }))
        .map_err(|_| "synthetic ACK encoding failed")?;
        advance_clock_from_database_sample(&clock, session_date, clock_origin, clock_anchor)?;
        let now_ms = clock.now_ms();
        let mut packed = ws_server_frame(0x1, &ack)?;
        if !plan.events_after_ack.is_empty() {
            let record_count = u8::try_from(plan.events_after_ack.len())
                .map_err(|_| "synthetic packed record count exceeded u8")?;
            market_record_batches.push(record_count);
            let payload = synthetic_market_payload(session_date, now_ms, &plan.events_after_ack)?;
            packed.extend_from_slice(&ws_server_frame(0x1, &payload)?);
        }
        if stream.write_all(&packed).await.is_err() {
            if allow_early_close {
                return Ok(LoopbackSummary {
                    commands,
                    market_record_batches,
                    client_closed: true,
                });
            }
            return Err("synthetic ACK/event write failed".to_owned());
        }
    }

    let client_closed =
        match tokio::time::timeout(Duration::from_secs(5), read_ws_frame(&mut stream)).await {
            Ok(Ok((0x8, _))) | Ok(Err(_)) => true,
            Ok(Ok(_)) => {
                return Err("unexpected client WebSocket frame after planned commands".to_owned());
            }
            Err(_) if allow_early_close => false,
            Err(_) => return Err("synthetic client socket did not close".to_owned()),
        };
    Ok(LoopbackSummary {
        commands,
        market_record_batches,
        client_closed,
    })
}

fn eligibility_remaining(
    origin: FixtureClockOrigin,
    session_date: NaiveDate,
    current_ms: i64,
    elapsed_ms: i64,
    not_before_ms: i64,
) -> Result<Duration, String> {
    // Validate current/modelled coherence before calculating any future time.
    origin.post_ack_target_ms(session_date, elapsed_ms, current_ms)?;
    let required_elapsed = not_before_ms
        .checked_sub(origin.synthetic_socket_anchor_ms)
        .ok_or("fixture eligibility interval overflow")?
        .max(0);
    // Validate the prospective session date without changing any clock.
    origin.post_ack_target_ms(session_date, elapsed_ms.max(required_elapsed), current_ms)?;
    let remaining_ms = required_elapsed.saturating_sub(elapsed_ms).max(0);
    Ok(Duration::from_millis(
        u64::try_from(remaining_ms).map_err(|_| "fixture eligibility interval invalid")?,
    ))
}

fn advance_clock_from_database_sample(
    clock: &TestClock,
    session_date: NaiveDate,
    clock_origin: FixtureClockOrigin,
    clock_anchor: Instant,
) -> Result<(), String> {
    let elapsed_ms = i64::try_from(clock_anchor.elapsed().as_millis()).unwrap_or(i64::MAX);
    advance_clock_from_elapsed(clock, session_date, clock_origin, elapsed_ms)
}

fn advance_clock_from_elapsed(
    clock: &TestClock,
    session_date: NaiveDate,
    clock_origin: FixtureClockOrigin,
    elapsed_ms: i64,
) -> Result<(), String> {
    // The socket epoch is the conservative whole-second floor of the exact
    // database sample. Advance from that same anchor so the whole-second wire
    // event cannot predate the socket or make its capture lead the modeled
    // later database observation.
    let target_ms = clock_origin.post_ack_target_ms(session_date, elapsed_ms, clock.now_ms())?;
    let delta_ms = target_ms.saturating_sub(clock.now_ms());
    if delta_ms > 0 {
        clock.advance_ms(delta_ms);
    }
    Ok(())
}

fn synthetic_post_ack_target_ms(
    session_date: NaiveDate,
    socket_open_ms: i64,
    elapsed_ms: i64,
    current_ms: i64,
) -> Result<i64, String> {
    let remainder_ms = socket_open_ms.rem_euclid(1_000);
    let first_wire_ms = if remainder_ms == 0 {
        socket_open_ms
    } else {
        socket_open_ms.saturating_add(1_000 - remainder_ms)
    };
    let elapsed_target_ms = socket_open_ms.saturating_add(elapsed_ms.max(0));
    let target_ms = current_ms.max(elapsed_target_ms).max(first_wire_ms);
    let wire_event_ms = target_ms.div_euclid(1_000).saturating_mul(1_000);
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
    let event_date = chrono::DateTime::<Utc>::from_timestamp_millis(wire_event_ms)
        .ok_or("synthetic event timestamp invalid")?
        .with_timezone(&kst)
        .date_naive();
    let received_date = chrono::DateTime::<Utc>::from_timestamp_millis(target_ms)
        .ok_or("synthetic receive timestamp invalid")?
        .with_timezone(&kst)
        .date_naive();
    if wire_event_ms < socket_open_ms || event_date != session_date || received_date != session_date
    {
        return Err("synthetic post-ACK event is outside its socket session".to_owned());
    }
    Ok(target_ms)
}

pub(super) async fn single_connection_worker_pool(
    database: &boundary::DisposableDatabase,
) -> Result<PgPool, TestError> {
    let database_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&database.migration_owner)
        .await?;
    if !database_name.starts_with("lagrange_ws3a_")
        || !database_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("single-connection pool requested outside generated database scope".into());
    }
    let supervisor_url = std::env::var(boundary::SUPERVISOR_ENV)?;
    let options: PgConnectOptions = supervisor_url.parse()?;
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(
            options
                .database(&database_name)
                .username("worker")
                .password("lagrange")
                .ssl_mode(PgSslMode::Disable),
        )
        .await?)
}

fn synthetic_market_payload(
    session_date: NaiveDate,
    now_ms: i64,
    records: &[(String, u32)],
) -> Result<Vec<u8>, String> {
    if records.is_empty() || records.len() > 30 {
        return Err("synthetic packed record count invalid".to_owned());
    }
    let kst = FixedOffset::east_opt(9 * 60 * 60).ok_or("KST offset invalid")?;
    let trade_time = chrono::DateTime::<Utc>::from_timestamp_millis(now_ms)
        .ok_or("test wall timestamp invalid")?
        .with_timezone(&kst)
        .format("%H%M%S")
        .to_string();
    let mut packed_fields = Vec::with_capacity(records.len() * 47);
    for (symbol, sequence) in records {
        let mut fields = vec![String::new(); 47];
        fields[0] = symbol.clone();
        fields[1] = trade_time.clone();
        fields[2] = (70_000 + sequence).to_string();
        fields[3] = "2".to_owned();
        fields[4] = "100".to_owned();
        fields[5] = "0.14".to_owned();
        fields[12] = (10 + sequence).to_string();
        fields[13] = (20 + sequence).to_string();
        fields[33] = session_date.format("%Y%m%d").to_string();
        fields[34] = "20".to_owned();
        fields[35] = "N".to_owned();
        fields[43] = "0".to_owned();
        fields[46] = "2".to_owned();
        packed_fields.extend(fields);
    }
    Ok(format!(
        "0|H0STCNT0|{:03}|{}",
        records.len(),
        packed_fields.join("^")
    )
    .into_bytes())
}

#[cfg(test)]
mod fixture_clock_unit_tests {
    use super::*;

    #[test]
    fn eligibility_wait_uses_computed_elapsed_and_preserves_database_upper_bound() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let sample = kst_ms(date, 10, 0, 0, 750);
        let origin = FixtureClockOrigin::from_database_sample(date, sample).unwrap();
        let clock = TestClock::at(origin.synthetic_socket_anchor_ms);
        let sample_wait = eligibility_remaining(origin, date, clock.now_ms(), 0, sample).unwrap();
        assert_eq!(sample_wait, Duration::from_millis(750));
        advance_clock_from_elapsed(&clock, date, origin, 750).unwrap();
        assert!(clock.now_ms() >= sample);
        let reserved_at = clock.now_ms();
        // Synthetic input represents the C2 hint; no fixture clock jump occurs.
        let not_before = reserved_at.checked_add(1_000).unwrap();
        let remaining = eligibility_remaining(origin, date, clock.now_ms(), 800, not_before).unwrap();
        assert_eq!(remaining, Duration::from_millis(950));
        let before = clock.now_ms();
        assert_eq!(before, sample, "computing eligibility must not advance time");
        advance_clock_from_elapsed(&clock, date, origin, 1_749).unwrap();
        assert!(clock.now_ms() < not_before);
        advance_clock_from_elapsed(&clock, date, origin, 1_750).unwrap();
        assert_eq!(clock.now_ms(), not_before);
        assert!(clock.now_ms() <= sample + 1_750);
    }

    #[test]
    fn eligibility_wait_rejects_incoherent_or_cross_session_targets() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let sample = kst_ms(date, 23, 59, 59, 500);
        let origin = FixtureClockOrigin::from_database_sample(date, sample).unwrap();
        assert!(eligibility_remaining(origin, date, sample + 1, 0, sample).is_err());
        assert!(eligibility_remaining(origin, date, origin.synthetic_socket_anchor_ms, -1, sample).is_err());
        assert!(eligibility_remaining(origin, date, sample, 500, sample + 500).is_err());
        assert!(eligibility_remaining(origin, date, sample, 500, i64::MIN).is_err());
    }

    fn kst_ms(
        session_date: NaiveDate,
        hour: u32,
        minute: u32,
        second: u32,
        millisecond: u32,
    ) -> i64 {
        let kst = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        session_date
            .and_hms_milli_opt(hour, minute, second, millisecond)
            .unwrap()
            .and_local_timezone(kst)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    fn payload_event_ms(session_date: NaiveDate, received_at_ms: i64) -> i64 {
        let payload =
            synthetic_market_payload(session_date, received_at_ms, &[("005930".to_owned(), 1)])
                .unwrap();
        let text = std::str::from_utf8(&payload).unwrap();
        let trade_time = text.split('|').nth(3).unwrap().split('^').nth(1).unwrap();
        let hour = trade_time[0..2].parse::<u32>().unwrap();
        let minute = trade_time[2..4].parse::<u32>().unwrap();
        let second = trade_time[4..6].parse::<u32>().unwrap();
        kst_ms(session_date, hour, minute, second, 0)
    }

    #[test]
    fn synthetic_post_ack_event_is_fresh_at_exact_and_subsecond_socket_open() {
        let session_date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let exact_second = kst_ms(session_date, 10, 0, 0, 0);
        let cases = [
            (exact_second, 0, exact_second, exact_second),
            (exact_second, 999, exact_second, exact_second + 999),
            (exact_second, 1_000, exact_second, exact_second + 1_000),
            (exact_second + 1, 0, exact_second + 1, exact_second + 1_000),
            (
                exact_second + 999,
                0,
                exact_second + 999,
                exact_second + 1_000,
            ),
            (
                exact_second + 999,
                2,
                exact_second + 999,
                exact_second + 1_001,
            ),
        ];

        for (socket_open_ms, elapsed_ms, current_ms, expected_target_ms) in cases {
            let target_ms =
                synthetic_post_ack_target_ms(session_date, socket_open_ms, elapsed_ms, current_ms)
                    .unwrap();
            assert_eq!(target_ms, expected_target_ms);
            let event_ms = payload_event_ms(session_date, target_ms);
            assert!(event_ms >= socket_open_ms);
            assert!(event_ms <= target_ms);
            assert!(target_ms.saturating_sub(event_ms) < 1_000);
        }
    }

    #[test]
    fn fixture_clock_origin_keeps_capture_within_database_sample() {
        let session_date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let exact_second = kst_ms(session_date, 10, 0, 0, 0);
        let database_sample_ms = exact_second + 250;
        let elapsed_ms = 20;
        let later_database_ms = database_sample_ms + elapsed_ms;
        let origin =
            FixtureClockOrigin::from_database_sample(session_date, database_sample_ms).unwrap();
        let clock = TestClock::at(origin.synthetic_socket_anchor_ms);

        advance_clock_from_elapsed(&clock, session_date, origin, elapsed_ms).unwrap();
        let captured_ms = clock.now_ms();
        let event_ms = payload_event_ms(session_date, captured_ms);

        assert!(event_ms >= origin.synthetic_socket_anchor_ms);
        assert!(event_ms <= captured_ms);
        assert!(
            captured_ms <= later_database_ms,
            "synthetic capture must not lead the modeled later database sample"
        );
    }

    #[test]
    fn fixture_clock_origin_matrix_is_coherent_and_session_bounded() {
        let session_date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let exact_second = kst_ms(session_date, 10, 0, 0, 0);
        let cases = [
            (exact_second, 0),
            (exact_second + 1, 0),
            (exact_second + 250, 20),
            (exact_second + 250, 749),
            (exact_second + 250, 750),
            (exact_second + 250, 1_000),
            (exact_second + 999, 0),
            (exact_second + 999, 1),
            (exact_second + 999, 1_001),
        ];

        for (database_sample_ms, elapsed_ms) in cases {
            let origin =
                FixtureClockOrigin::from_database_sample(session_date, database_sample_ms).unwrap();
            assert_eq!(origin.exact_database_sample_ms, database_sample_ms);
            assert_eq!(origin.synthetic_socket_anchor_ms.rem_euclid(1_000), 0);
            assert!(origin.synthetic_socket_anchor_ms <= database_sample_ms);

            let clock = TestClock::at(origin.synthetic_socket_anchor_ms);
            advance_clock_from_elapsed(&clock, session_date, origin, elapsed_ms).unwrap();
            let captured_ms = clock.now_ms();
            let event_ms = payload_event_ms(session_date, captured_ms);
            assert!(event_ms >= origin.synthetic_socket_anchor_ms);
            assert!(event_ms <= captured_ms);
            assert!(captured_ms <= database_sample_ms + elapsed_ms);
        }

        let midnight_sample = kst_ms(session_date, 0, 0, 0, 1);
        let midnight_origin =
            FixtureClockOrigin::from_database_sample(session_date, midnight_sample).unwrap();
        assert_eq!(
            midnight_origin.synthetic_socket_anchor_ms,
            kst_ms(session_date, 0, 0, 0, 0)
        );
        let midnight_clock = TestClock::at(midnight_origin.synthetic_socket_anchor_ms);
        advance_clock_from_elapsed(&midnight_clock, session_date, midnight_origin, 0).unwrap();
        assert_eq!(
            payload_event_ms(session_date, midnight_clock.now_ms()),
            midnight_origin.synthetic_socket_anchor_ms
        );

        let end_sample = kst_ms(session_date, 23, 59, 59, 999);
        let end_origin =
            FixtureClockOrigin::from_database_sample(session_date, end_sample).unwrap();
        let end_clock = TestClock::at(end_origin.synthetic_socket_anchor_ms);
        advance_clock_from_elapsed(&end_clock, session_date, end_origin, 1).unwrap();
        assert_eq!(
            payload_event_ms(session_date, end_clock.now_ms()),
            end_origin.synthetic_socket_anchor_ms
        );
        assert!(advance_clock_from_elapsed(&end_clock, session_date, end_origin, 1_000).is_err());
    }

    #[test]
    fn fixture_clock_origin_rejects_invalid_preconditions_and_keeps_deadline_exclusive() {
        let session_date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let other_date = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();
        let exact_second = kst_ms(session_date, 10, 0, 0, 0);
        let database_sample_ms = exact_second + 250;
        let origin =
            FixtureClockOrigin::from_database_sample(session_date, database_sample_ms).unwrap();

        assert!(FixtureClockOrigin::from_database_sample(other_date, database_sample_ms).is_err());
        assert!(FixtureClockOrigin::from_database_sample(session_date, i64::MAX).is_err());
        assert!(
            origin
                .post_ack_target_ms(session_date, -1, origin.synthetic_socket_anchor_ms)
                .is_err()
        );
        assert!(
            origin
                .post_ack_target_ms(session_date, 0, origin.synthetic_socket_anchor_ms - 1,)
                .is_err()
        );
        assert!(
            origin
                .post_ack_target_ms(session_date, 20, database_sample_ms + 21)
                .is_err()
        );
        assert!(
            origin
                .post_ack_target_ms(session_date, i64::MAX, origin.synthetic_socket_anchor_ms,)
                .is_err()
        );

        let reserved_ms = origin.synthetic_socket_anchor_ms;
        let deadline_ms = reserved_ms + 5_000;
        let before_deadline = origin
            .post_ack_target_ms(session_date, 4_999, reserved_ms)
            .unwrap();
        let at_deadline = origin
            .post_ack_target_ms(session_date, 5_000, before_deadline)
            .unwrap();
        assert!(reserved_ms <= before_deadline && before_deadline < deadline_ms);
        assert_eq!(at_deadline, deadline_ms);
        assert!(!(reserved_ms <= at_deadline && at_deadline < deadline_ms));
    }

    #[test]
    fn synthetic_post_ack_event_is_monotonic_and_session_bounded() {
        let session_date = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let exact_second = kst_ms(session_date, 10, 0, 0, 0);
        let current_ms = exact_second + 2_500;
        let target_ms =
            synthetic_post_ack_target_ms(session_date, exact_second + 1, 0, current_ms).unwrap();
        assert_eq!(target_ms, current_ms);
        let event_ms = payload_event_ms(session_date, target_ms);
        assert!(event_ms >= exact_second + 1);
        assert!(event_ms <= target_ms);

        let last_exact_second = kst_ms(session_date, 23, 59, 59, 0);
        let last_valid =
            synthetic_post_ack_target_ms(session_date, last_exact_second, 999, last_exact_second)
                .unwrap();
        assert_eq!(last_valid, last_exact_second + 999);
        assert_eq!(
            payload_event_ms(session_date, last_valid),
            last_exact_second
        );

        let last_millisecond = kst_ms(session_date, 23, 59, 59, 999);
        assert!(
            synthetic_post_ack_target_ms(session_date, last_millisecond, 0, last_millisecond,)
                .is_err()
        );
        assert!(
            synthetic_post_ack_target_ms(
                session_date,
                last_exact_second,
                1_000,
                last_exact_second,
            )
            .is_err()
        );
    }
}

async fn read_http_headers(stream: &mut TcpStream) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut one = [0_u8; 1];
    while bytes.len() < 16 * 1024 {
        stream.read_exact(&mut one).await?;
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "synthetic HTTP headers too large",
    ))
}

fn header_value(request: &[u8], wanted: &str) -> Option<String> {
    let text = String::from_utf8_lossy(request);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(wanted)
            .then(|| value.trim().to_owned())
    })
}

async fn read_ws_frame(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), std::io::Error> {
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await?;
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut extended = [0_u8; 2];
        stream.read_exact(&mut extended).await?;
        length = usize::from(u16::from_be_bytes(extended));
    }
    if length > 1_048_576 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "synthetic frame too large",
        ));
    }
    let mut mask = [0_u8; 4];
    if masked {
        stream.read_exact(&mut mask).await?;
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok((opcode, payload))
}

fn ws_server_frame(opcode: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        0..=125 => frame.push(payload.len() as u8),
        126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => return Err("synthetic WebSocket payload too large".to_owned()),
    }
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub(super) struct PgRelay {
    directory: TempDir,
    expected_database: String,
    shared: Arc<Mutex<Option<CommitIntercept>>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<OwnedTask<()>>,
}

enum CommitIntercept {
    DropResponse {
        reached: oneshot::Sender<()>,
    },
    HoldResponse {
        reached: oneshot::Sender<()>,
        resume: oneshot::Receiver<()>,
    },
    DropBeforeForward {
        reached: oneshot::Sender<()>,
    },
}

pub(super) struct CommitObservation {
    reached: oneshot::Receiver<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommitObservationOutcome {
    Observed,
    TimedOut,
    SenderClosed,
}

impl CommitObservation {
    pub(super) async fn receive(&mut self) -> CommitObservationOutcome {
        match (&mut self.reached).await {
            Ok(()) => CommitObservationOutcome::Observed,
            Err(_) => CommitObservationOutcome::SenderClosed,
        }
    }

    pub(super) async fn observe_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> CommitObservationOutcome {
        match tokio::time::timeout(timeout, self.receive()).await {
            Ok(outcome) => outcome,
            Err(_) => CommitObservationOutcome::TimedOut,
        }
    }

    pub(super) async fn wait(&mut self) -> Result<(), TestError> {
        tokio::time::timeout(Duration::from_secs(8), &mut self.reached)
            .await
            .map_err(|_| "relay did not observe PostgreSQL COMMIT")??;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn from_receiver(reached: oneshot::Receiver<()>) -> Self {
        Self { reached }
    }
}

pub(super) struct HeldCommit {
    observation: CommitObservation,
    resume: Option<oneshot::Sender<()>>,
}

impl HeldCommit {
    pub(super) async fn wait(&mut self) -> Result<(), TestError> {
        self.observation.wait().await
    }

    pub(super) fn release(mut self) {
        if let Some(resume) = self.resume.take() {
            let _ = resume.send(());
        }
    }
}

impl PgRelay {
    pub(super) async fn start(database_name: &str) -> Result<Self, TestError> {
        if !database_name.starts_with("lagrange_ws3a_")
            || !database_name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("relay database name is outside the generated test scope".into());
        }
        let directory = tempfile::tempdir()?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let socket_path = directory.path().join(format!(".s.PGSQL.{CLUSTER_PORT}"));
        let listener = UnixListener::bind(&socket_path)?;
        let shared = Arc::new(Mutex::new(None));
        let task_shared = shared.clone();
        let task_database = database_name.to_owned();
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let descendants = DescendantTracker::new();
        let task_descendants = descendants.clone();
        let owner = current_fixture_owner();
        let task = spawn_owned(
            async move {
                let mut clients = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        accepted = listener.accept() => match accepted {
                            Ok((client, _)) => {
                                let shared = task_shared.clone();
                                let expected_database = task_database.clone();
                                let child_guard = task_descendants.child();
                                clients.spawn(async move {
                                    let _child_guard = child_guard;
                                    relay_connection(client, shared, expected_database).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
                clients.shutdown().await;
            },
            owner,
            Some(descendants),
        );
        Ok(Self {
            directory,
            expected_database: database_name.to_owned(),
            shared,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub(super) async fn worker_pool(&self, database: &str) -> Result<PgPool, TestError> {
        if !database.starts_with("lagrange_ws3a_")
            || !database
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("relay database name is outside the generated test scope".into());
        }
        if database != self.expected_database {
            return Err("relay worker pool requested a different generated database".into());
        }
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(
                PgConnectOptions::new()
                    .socket(self.directory.path())
                    .port(CLUSTER_PORT)
                    .ssl_mode(PgSslMode::Disable)
                    .username("worker")
                    .password("lagrange")
                    .database(database),
            )
            .await?;
        let (role, current_db): (String, String) =
            sqlx::query_as("SELECT current_user, current_database()")
                .fetch_one(&pool)
                .await?;
        if role != "worker" || current_db != database {
            pool.close().await;
            return Err("relay did not authenticate the exact worker/database".into());
        }
        Ok(pool)
    }

    pub(super) fn drop_next_commit_response(&self) -> CommitObservation {
        let (reached_tx, reached_rx) = oneshot::channel();
        *self.shared.lock().expect("relay intercept lock") = Some(CommitIntercept::DropResponse {
            reached: reached_tx,
        });
        CommitObservation {
            reached: reached_rx,
        }
    }

    pub(super) fn hold_next_commit_response(&self) -> HeldCommit {
        let (reached_tx, reached_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = oneshot::channel();
        *self.shared.lock().expect("relay intercept lock") = Some(CommitIntercept::HoldResponse {
            reached: reached_tx,
            resume: resume_rx,
        });
        HeldCommit {
            observation: CommitObservation {
                reached: reached_rx,
            },
            resume: Some(resume_tx),
        }
    }

    pub(super) fn drop_next_commit_before_forward(&self) -> CommitObservation {
        let (reached_tx, reached_rx) = oneshot::channel();
        *self.shared.lock().expect("relay intercept lock") =
            Some(CommitIntercept::DropBeforeForward {
                reached: reached_tx,
            });
        CommitObservation {
            reached: reached_rx,
        }
    }

    pub(super) async fn close(mut self) -> Result<(), TestError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let mut task = task;
            task.join_result("PostgreSQL relay listener").await?;
        }
        Ok(())
    }
}

impl Drop for PgRelay {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn relay_connection(
    client: UnixStream,
    shared: Arc<Mutex<Option<CommitIntercept>>>,
    expected_database: String,
) {
    let upstream_path = PathBuf::from(CLUSTER_SOCKET).join(format!(".s.PGSQL.{CLUSTER_PORT}"));
    let Ok(upstream) = UnixStream::connect(upstream_path).await else {
        return;
    };
    let (mut client_read, mut client_write) = client.into_split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();
    let frontend = relay_frontend(
        &mut client_read,
        &mut upstream_write,
        shared.clone(),
        &expected_database,
    );
    let backend = async {
        loop {
            let frame = match read_pg_backend_frame(&mut upstream_read).await {
                Ok(frame) => frame,
                Err(_) => return,
            };
            if is_commit_complete(&frame) {
                let intercept = shared.lock().expect("relay intercept lock").take();
                match intercept {
                    Some(CommitIntercept::DropResponse { reached }) => {
                        let _ = reached.send(());
                        return;
                    }
                    Some(CommitIntercept::HoldResponse { reached, resume }) => {
                        let _ = reached.send(());
                        let _ = tokio::time::timeout(Duration::from_secs(8), resume).await;
                    }
                    Some(CommitIntercept::DropBeforeForward { reached }) => {
                        let _ = reached.send(());
                        return;
                    }
                    None => {}
                }
            }
            if client_write.write_all(&frame).await.is_err() {
                return;
            }
        }
    };
    tokio::pin!(frontend);
    tokio::pin!(backend);
    tokio::select! {
        _ = &mut frontend => {},
        _ = &mut backend => {},
    }
}

async fn relay_frontend<R: AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(
    reader: &mut R,
    writer: &mut W,
    shared: Arc<Mutex<Option<CommitIntercept>>>,
    expected_database: &str,
) -> Result<(), std::io::Error> {
    let mut startup_header = [0_u8; 4];
    reader.read_exact(&mut startup_header).await?;
    let startup_length = u32::from_be_bytes(startup_header) as usize;
    if !(8..=MAX_PG_FRAME_BYTES).contains(&startup_length) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "PostgreSQL startup frame size invalid",
        ));
    }
    let mut startup = Vec::with_capacity(startup_length);
    startup.extend_from_slice(&startup_header);
    startup.resize(startup_length, 0);
    reader.read_exact(&mut startup[4..]).await?;
    if !startup_selects_worker_database(&startup, expected_database) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "PostgreSQL relay permits only its generated worker database",
        ));
    }
    writer.write_all(&startup).await?;

    loop {
        let mut header = [0_u8; 5];
        reader.read_exact(&mut header).await?;
        let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if !(4..=MAX_PG_FRAME_BYTES).contains(&length) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PostgreSQL frontend frame size invalid",
            ));
        }
        let mut frame = Vec::with_capacity(length + 1);
        frame.extend_from_slice(&header);
        frame.resize(length + 1, 0);
        reader.read_exact(&mut frame[5..]).await?;
        if is_commit_query(&frame) {
            let mut state = shared.lock().expect("relay intercept lock");
            if matches!(
                state.as_ref(),
                Some(CommitIntercept::DropBeforeForward { .. })
            ) {
                if let Some(CommitIntercept::DropBeforeForward { reached }) = state.take() {
                    let _ = reached.send(());
                }
                return Ok(());
            }
        }
        writer.write_all(&frame).await?;
    }
}

fn startup_selects_worker_database(startup: &[u8], expected_database: &str) -> bool {
    if startup.len() < 9
        || u32::from_be_bytes([startup[4], startup[5], startup[6], startup[7]]) != 196_608
    {
        return false;
    }
    let parameters = &startup[8..];
    if parameters.last() != Some(&0) {
        return false;
    }
    let mut fields = parameters.split(|byte| *byte == 0);
    let mut user = None;
    let mut database = None;
    loop {
        let Some(key) = fields.next() else {
            return false;
        };
        if key.is_empty() {
            break;
        }
        let Some(value) = fields.next() else {
            return false;
        };
        match key {
            b"user" => user = Some(value),
            b"database" => database = Some(value),
            _ => {}
        }
    }
    user == Some(b"worker") && database == Some(expected_database.as_bytes())
}

async fn read_pg_backend_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, std::io::Error> {
    let mut header = [0_u8; 5];
    reader.read_exact(&mut header).await?;
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if !(4..=MAX_PG_FRAME_BYTES).contains(&length) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "PostgreSQL response frame size invalid",
        ));
    }
    let mut frame = Vec::with_capacity(length + 1);
    frame.extend_from_slice(&header);
    frame.resize(length + 1, 0);
    reader.read_exact(&mut frame[5..]).await?;
    Ok(frame)
}

fn is_commit_complete(frame: &[u8]) -> bool {
    frame.first() == Some(&b'C') && frame.get(5..).is_some_and(|payload| payload == b"COMMIT\0")
}

fn is_commit_query(frame: &[u8]) -> bool {
    frame.first() == Some(&b'Q')
        && frame
            .get(5..)
            .and_then(|payload| payload.strip_suffix(&[0]))
            .is_some_and(|query| query.eq_ignore_ascii_case(b"COMMIT"))
}

#[cfg(test)]
mod relay_observation_unit_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn commit_observation_distinguishes_success_timeout_and_sender_closure() {
        let (observed_tx, observed_rx) = oneshot::channel();
        let mut observed = CommitObservation::from_receiver(observed_rx);
        observed_tx
            .send(())
            .expect("observation receiver remains owned");
        assert_eq!(
            observed.observe_with_timeout(Duration::from_secs(8)).await,
            CommitObservationOutcome::Observed
        );

        let (closed_tx, closed_rx) = oneshot::channel();
        let mut closed = CommitObservation::from_receiver(closed_rx);
        drop(closed_tx);
        assert_eq!(
            closed.observe_with_timeout(Duration::from_secs(8)).await,
            CommitObservationOutcome::SenderClosed
        );

        let (_pending_tx, pending_rx) = oneshot::channel();
        let mut pending = CommitObservation::from_receiver(pending_rx);
        assert_eq!(
            pending.observe_with_timeout(Duration::from_secs(8)).await,
            CommitObservationOutcome::TimedOut
        );
    }

    #[test]
    fn commit_complete_recognizer_accepts_only_exact_postgresql_frame() {
        let exact = [b'C', 0, 0, 0, 11, b'C', b'O', b'M', b'M', b'I', b'T', 0];
        assert!(is_commit_complete(&exact));

        for rejected in [
            &b"C\0\0\0\x0bCOMMIT"[..],
            &b"C\0\0\0\x0cCOMMIT \0"[..],
            &b"C\0\0\0\x11RELEASE\0"[..],
            &b"Z\0\0\0\x0bCOMMIT\0"[..],
        ] {
            assert!(!is_commit_complete(rejected));
        }
    }
}
