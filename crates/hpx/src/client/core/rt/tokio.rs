//! Tokio IO integration for core.
use std::{
    future::Future,
    pin::Pin,
    sync::OnceLock,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use pin_project_lite::pin_project;

use super::{Executor, Sleep, Timer};

/// hpx-owned runtime that drives every connection task (ConnTask, ClientTask,
/// idle-sender re-polls, ...).
///
/// Connection tasks are spawned with [`tokio::spawn`], which binds them to
/// whatever runtime happens to be current on the *creating* thread. In this
/// workspace a client's connections are created and reused across several
/// independent runtimes (executor worker, accounting re-sync thread, ...).
/// A connection created on runtime A then reused from runtime B stalls:
/// its request is queued onto the dispatch channel, but the task that must
/// poll it lives on A, which is idle at that moment — the request hangs until
/// the bounded dispatch timeout fires. Driving all connection tasks on this
/// single hpx-owned multi-thread runtime keeps them schedulable regardless of
/// which thread issues the request, making cross-runtime pool reuse safe.
static HPX_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[expect(
    clippy::expect_used,
    reason = "tokio runtime build failure is unrecoverable"
)]
fn hpx_runtime() -> tokio::runtime::Handle {
    HPX_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("hpx-net")
                .worker_threads(4)
                .build()
                .expect("failed to build the hpx connection runtime")
        })
        .handle()
        .clone()
}

/// Runs `fut` on the hpx-owned connection runtime, or awaits it directly
/// when already running there.
///
/// Connection establishment creates runtime-bound resources — most
/// importantly the socket's I/O-driver registration, and any timers created
/// during the handshake. A socket binds to whichever runtime is current when
/// it is created. If a connection is established inside a caller runtime that
/// is only driven periodically (e.g. a background thread that `block_on`s its
/// own current-thread runtime every 60s), the socket's read-ready events are
/// dispatched only by that dormant driver: writes still drain (the kernel
/// buffer is writable without a driver) but reads stall forever once the
/// runtime goes idle, surfacing as a fixed stall on the first request sent
/// after the connection has been idle. Establishing connections on this
/// continuously-driven runtime makes them usable from any thread.
pub(crate) async fn on_hpx_runtime<F, T>(fut: F) -> Result<T, tokio::task::JoinError>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = hpx_runtime();
    match tokio::runtime::Handle::try_current() {
        Ok(current) if current.id() == handle.id() => Ok(fut.await),
        _ => handle.spawn(fut).await,
    }
}

/// Future executor that utilises `tokio` threads.
#[non_exhaustive]
#[derive(Default, Debug, Clone)]
pub(crate) struct TokioExecutor {}

/// A Timer that uses the tokio runtime.
#[non_exhaustive]
#[derive(Default, Clone, Debug)]
pub(crate) struct TokioTimer;

// Use TokioSleep to get tokio::time::Sleep to implement Unpin.
// see https://docs.rs/tokio/latest/tokio/time/struct.Sleep.html
pin_project! {
    #[derive(Debug)]
    struct TokioSleep {
        #[pin]
        inner: tokio::time::Sleep,
    }
}

// ===== impl TokioExecutor =====

impl<Fut> Executor<Fut> for TokioExecutor
where
    Fut: Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        hpx_runtime().spawn(fut);
    }
}

impl TokioExecutor {
    /// Create new executor that relies on [`tokio::spawn`] to execute futures.
    pub(crate) const fn new() -> Self {
        Self {}
    }
}

// ==== impl TokioTimer =====

impl Timer for TokioTimer {
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
        Box::pin(TokioSleep {
            inner: tokio::time::sleep(duration),
        })
    }

    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
        Box::pin(TokioSleep {
            inner: tokio::time::sleep_until(deadline.into()),
        })
    }

    fn now(&self) -> Instant {
        tokio::time::Instant::now().into()
    }

    fn reset(&self, sleep: &mut Pin<Box<dyn Sleep>>, new_deadline: Instant) {
        if let Some(sleep) = sleep.as_mut().downcast_mut_pin::<TokioSleep>() {
            sleep.reset(new_deadline);
        }
    }
}

impl TokioTimer {
    /// Create a new TokioTimer
    pub(crate) const fn new() -> Self {
        Self {}
    }
}

impl Future for TokioSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}

impl Sleep for TokioSleep {}

impl TokioSleep {
    fn reset(self: Pin<&mut Self>, deadline: Instant) {
        self.project().inner.as_mut().reset(deadline.into());
    }
}
