//! Bounded blocking work used by Paper's curated-file paths.
//!
//! Curated Parquet reads are synchronous at the `market-data` boundary.  They
//! must never run directly on a Tokio worker thread, and a canceled future
//! cannot forcibly stop a closure that has already entered `spawn_blocking`.
//! This seam gives every Paper file read an application deadline, a
//! cooperative cancellation flag, and one shared slot so a wedged filesystem
//! read cannot multiply on every polling retry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{Semaphore, watch};

/// Curated reads have a shorter bound than the complete Paper operation.  The
/// outer runner deadline remains the final boundary for the operation and
/// cycle; this bound ensures a stalled read returns before a healthy DB write
/// phase would normally begin.
pub const PAPER_CURATED_IO_DEADLINE: Duration = Duration::from_secs(10);

static PAPER_BLOCKING_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn blocking_slots() -> Arc<Semaphore> {
    PAPER_BLOCKING_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

/// Why a bounded blocking operation stopped.
#[derive(Debug)]
pub enum BlockingIoError<E> {
    /// The Paper daemon received its shutdown signal.
    Canceled,
    /// The operation or the shared blocking slot exceeded its deadline.
    TimedOut,
    /// The closure returned its domain error.
    Failed(E),
}

struct CancellationOnDrop(Arc<AtomicBool>);

impl Drop for CancellationOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn shutdown_requested(mut shutdown: watch::Receiver<bool>) -> bool {
    loop {
        if *shutdown.borrow() {
            return true;
        }
        if shutdown.changed().await.is_err() {
            return false;
        }
    }
}

/// Runs synchronous curated-file work without blocking a Tokio worker.
///
/// The closure receives a cancellation flag and should check it between
/// files/rows.  A native filesystem call that is already blocked cannot be
/// interrupted portably, so the permit is deliberately owned by the
/// blocking closure and the caller's future returns on timeout or shutdown;
/// later Paper retries cannot start another read until the first one exits.
pub async fn run_bounded_blocking<F, T, E>(
    timeout: Duration,
    shutdown: Option<watch::Receiver<bool>>,
    work: F,
) -> Result<T, BlockingIoError<E>>
where
    F: FnOnce(Arc<AtomicBool>) -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    if timeout.is_zero() {
        return Err(BlockingIoError::TimedOut);
    }
    let canceled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancellationOnDrop(canceled.clone());
    let acquire = blocking_slots().acquire_owned();
    tokio::pin!(acquire);
    let timer = tokio::time::sleep(timeout);
    tokio::pin!(timer);
    let permit = match shutdown.clone() {
        Some(shutdown) => {
            tokio::select! {
                biased;
                _ = shutdown_requested(shutdown) => {
                    canceled.store(true, Ordering::Release);
                    return Err(BlockingIoError::Canceled);
                }
                permit = &mut acquire => permit.map_err(|_| BlockingIoError::Canceled)?,
                _ = &mut timer => return Err(BlockingIoError::TimedOut),
            }
        }
        None => {
            tokio::select! {
                biased;
                permit = &mut acquire => permit.map_err(|_| BlockingIoError::Canceled)?,
                _ = &mut timer => return Err(BlockingIoError::TimedOut),
            }
        }
    };

    let closure_canceled = canceled.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(closure_canceled)
    });
    tokio::pin!(task);
    match shutdown {
        Some(shutdown) => {
            tokio::select! {
                biased;
                result = &mut task => result
                    .map_err(|_| BlockingIoError::Canceled)?
                    .map_err(BlockingIoError::Failed),
                _ = shutdown_requested(shutdown) => {
                    canceled.store(true, Ordering::Release);
                    task.as_ref().abort();
                    Err(BlockingIoError::Canceled)
                }
                _ = &mut timer => {
                    canceled.store(true, Ordering::Release);
                    task.as_ref().abort();
                    Err(BlockingIoError::TimedOut)
                }
            }
        }
        None => tokio::select! {
            biased;
            result = &mut task => result
                .map_err(|_| BlockingIoError::Canceled)?
                .map_err(BlockingIoError::Failed),
            _ = &mut timer => {
                canceled.store(true, Ordering::Release);
                task.as_ref().abort();
                Err(BlockingIoError::TimedOut)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    static TEST_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

    #[tokio::test(start_paused = true)]
    async fn stalled_blocking_work_returns_at_its_deadline() {
        let _test_lock = TEST_LOCK
            .get_or_init(tokio::sync::Mutex::default)
            .lock()
            .await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_bounded_blocking(
            Duration::from_secs(5),
            None,
            move |canceled| {
                let _ = started_tx.send(());
                while !canceled.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                let _ = finished_tx.send(());
                Ok::<_, ()>(())
            },
        ));
        started_rx.await.expect("blocking closure started");
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(matches!(
            task.await.expect("blocking task join"),
            Err(BlockingIoError::TimedOut)
        ));
        finished_rx.await.expect("blocking closure observed cancel");
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_cancels_blocking_work_without_waiting_for_the_deadline() {
        let _test_lock = TEST_LOCK
            .get_or_init(tokio::sync::Mutex::default)
            .lock()
            .await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_bounded_blocking(
            Duration::from_secs(60),
            Some(shutdown_rx),
            move |canceled| {
                let _ = started_tx.send(());
                while !canceled.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                let _ = finished_tx.send(());
                Ok::<_, ()>(())
            },
        ));
        started_rx.await.expect("blocking closure started");
        shutdown_tx.send(true).expect("shutdown signal");
        assert!(matches!(
            task.await.expect("blocking task join"),
            Err(BlockingIoError::Canceled)
        ));
        finished_rx.await.expect("blocking closure observed cancel");
    }
}
