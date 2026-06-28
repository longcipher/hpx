//! Parallel page navigator — N OS-thread worker pool.
//!
//! `Page` (and its embedded V8 isolate) is `!Send`. This module spawns
//! `N` dedicated OS threads, each with its own tokio current-thread
//! runtime and its own `Page` instances. Jobs are dispatched from the
//! caller over `std::sync::mpsc` channels. Results come back over
//! `tokio::sync::oneshot` so the caller can `.await` them naturally.
//!
//! Round-robin scheduling — caller-visible API is just
//! `pager.navigate(url, profile).await`.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{page::Page, stealth::StealthProfile};

/// Result of a single navigation.
pub struct NavigateResult {
    /// Final body HTML, or empty on error.
    pub html: String,
    /// Wall-clock time from job dispatch through navigation completion.
    pub elapsed: Duration,
    /// Some(message) if navigate returned Err. None on success.
    pub error: Option<String>,
}

/// Internal job message.
struct Job {
    url: String,
    profile: Option<StealthProfile>,
    result_tx: tokio::sync::oneshot::Sender<NavigateResult>,
}

struct WorkerHandle {
    tx: mpsc::Sender<Job>,
    _thread: thread::JoinHandle<()>,
}

/// N-worker parallel navigation pool. Each worker thread owns its own
/// tokio runtime + its own `Page` instances.
pub struct ParallelPager {
    workers: Vec<WorkerHandle>,
    next_worker: AtomicUsize,
}

impl ParallelPager {
    /// Spawn `num_workers` OS threads.
    pub fn new(num_workers: usize) -> Self {
        assert!(num_workers > 0, "ParallelPager needs at least 1 worker");
        let workers = (0..num_workers)
            .map(|i| {
                let (tx, rx) = mpsc::channel::<Job>();
                let thread = thread::Builder::new()
                    .name(format!("hpx-browser-pager-{i}"))
                    .stack_size(64 * 1024 * 1024)
                    .spawn(move || worker_main(rx))
                    .unwrap_or_else(|e| panic!("failed to spawn pager worker: {e}"));
                WorkerHandle {
                    tx,
                    _thread: thread,
                }
            })
            .collect();
        Self {
            workers,
            next_worker: AtomicUsize::new(0),
        }
    }

    /// Dispatch a navigation to the next worker (round-robin).
    pub async fn navigate(
        &self,
        url: impl Into<String>,
        profile: Option<StealthProfile>,
    ) -> NavigateResult {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let job = Job {
            url: url.into(),
            profile,
            result_tx,
        };

        let idx = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        match self.workers[idx].tx.send(job) {
            Ok(_) => {}
            Err(send_err) => {
                return NavigateResult {
                    html: String::new(),
                    elapsed: Duration::default(),
                    error: Some(format!(
                        "worker {idx} unavailable (likely panicked): {send_err}"
                    )),
                };
            }
        }
        match result_rx.await {
            Ok(r) => r,
            Err(_) => NavigateResult {
                html: String::new(),
                elapsed: Duration::default(),
                error: Some("worker dropped result sender (panic during navigate)".to_string()),
            },
        }
    }

    /// Number of workers spawned at construction time.
    pub fn num_workers(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for ParallelPager {
    fn drop(&mut self) {
        self.workers.clear();
    }
}

fn worker_main(rx: mpsc::Receiver<Job>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("[pager-worker] failed to build tokio runtime: {e}");
            return;
        }
    };

    while let Ok(job) = rx.recv() {
        let begin = Instant::now();
        let url = job.url.clone();
        let profile = job.profile;

        let result: NavigateResult = rt.block_on(async move {
            match Page::from_html("<html><head></head><body></body></html>", profile.is_some())
                .await
            {
                Ok(mut page) => match page.navigate(&url).await {
                    Ok(()) => NavigateResult {
                        html: page.content(),
                        elapsed: begin.elapsed(),
                        error: None,
                    },
                    Err(e) => NavigateResult {
                        html: String::new(),
                        elapsed: begin.elapsed(),
                        error: Some(format!("{e}")),
                    },
                },
                Err(e) => NavigateResult {
                    html: String::new(),
                    elapsed: begin.elapsed(),
                    error: Some(format!("{e}")),
                },
            }
        });

        let _ = job.result_tx.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_pager_spawns_and_drops_cleanly() {
        let pager = ParallelPager::new(2);
        assert_eq!(pager.num_workers(), 2);
        drop(pager);
    }

    #[tokio::test]
    async fn parallel_navigate_returns_result() {
        let pager = ParallelPager::new(1);
        // about:blank is not an HTTP URI — verify pager dispatches and returns without panicking
        let result = pager.navigate("about:blank", None).await;
        assert!(result.elapsed.as_nanos() > 0, "job should have run");
        drop(pager);
    }
}
