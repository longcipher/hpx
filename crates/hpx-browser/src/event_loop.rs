//! Browser event loop wrapping deno_core's V8 event loop with
//! requestAnimationFrame scheduling and idle detection.

use std::time::{Duration, Instant};

use crate::{dom::Dom, js_runtime::BrowserJsRuntime};

/// Reason the event loop became idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReason {
    /// All pending work completed (no timers, no promises, no async ops).
    AllWorkDone,
    /// The timeout was reached.
    Timeout,
}

/// Browser event loop coordinator.
///
/// Drives JS execution, timers (via deno_core async ops), requestAnimationFrame
/// callbacks, and idle detection.
pub struct BrowserEventLoop {
    runtime: BrowserJsRuntime,
}

/// Error type for event loop operations.
#[derive(Debug, thiserror::Error)]
pub enum EventLoopError {
    #[error("js error: {0}")]
    Js(#[from] crate::js_runtime::JsError),
    #[error("event loop timeout")]
    Timeout,
    #[error("event loop error: {0}")]
    Other(String),
}

const RAF_BOOTSTRAP: &str = r#"
(() => {
  if (globalThis.__rafCallbacks) return;
  globalThis.__rafCallbacks = [];
  globalThis.__rafScheduled = false;

  globalThis.requestAnimationFrame = (cb) => {
    const id = globalThis.__rafCallbacks.length + 1;
    globalThis.__rafCallbacks.push({ id, cb });
    if (!globalThis.__rafScheduled) {
      globalThis.__rafScheduled = true;
    }
    return id;
  };

  globalThis.cancelAnimationFrame = (id) => {
    globalThis.__rafCallbacks = globalThis.__rafCallbacks.filter(c => c.id !== id);
  };

  globalThis.__fireRafCallbacks = () => {
    const cbs = globalThis.__rafCallbacks.splice(0);
    globalThis.__rafScheduled = false;
    const ts = typeof performance !== 'undefined' ? performance.now() : Date.now();
    for (const { cb } of cbs) {
      try { cb(ts); } catch (e) { console.error('rAF callback error:', e); }
    }
  };
})();
"#;

impl BrowserEventLoop {
    pub fn new() -> Self {
        let runtime = BrowserJsRuntime::new(Dom::new());
        Self::with_runtime(runtime)
    }

    /// Create from an existing runtime. Injects rAF bootstrap.
    pub fn with_runtime(mut runtime: BrowserJsRuntime) -> Self {
        let _ = runtime.execute_script(RAF_BOOTSTRAP);
        Self { runtime }
    }

    /// Create from an existing runtime without injecting rAF bootstrap.
    pub fn with_runtime_raw(runtime: BrowserJsRuntime) -> Self {
        Self { runtime }
    }

    /// Run the event loop until idle or timeout.
    ///
    /// Each tick fires pending rAF callbacks then runs deno_core's event loop
    /// with a 100ms slice. Returns `AllWorkDone` when deno_core reports no
    /// more pending work, or `Timeout` if the deadline is reached.
    pub async fn run_until_idle(
        &mut self,
        timeout: Duration,
    ) -> Result<IdleReason, EventLoopError> {
        let deadline = Instant::now() + timeout;

        loop {
            if Instant::now() >= deadline {
                return Ok(IdleReason::Timeout);
            }

            // Fire pending requestAnimationFrame callbacks
            let _ = self.runtime.execute_script(
                "if (globalThis.__fireRafCallbacks) globalThis.__fireRafCallbacks();",
            );

            let remaining = deadline.saturating_duration_since(Instant::now());
            let tick_timeout = remaining.min(Duration::from_millis(100));

            match tokio::time::timeout(tick_timeout, self.runtime.run_event_loop()).await {
                Ok(Ok(())) => {
                    // Event loop tick done. Fire rAF one more time in case
                    // user code registered callbacks during the tick.
                    let _ = self.runtime.execute_script(
                        "if (globalThis.__fireRafCallbacks) globalThis.__fireRafCallbacks();",
                    );
                    // Check if rAF or async work appeared.
                    let has_work = self
                        .runtime
                        .execute_script(
                            r#"
                            (globalThis.__rafCallbacks && globalThis.__rafCallbacks.length > 0) ? "true" : "false"
                            "#,
                        )
                        .unwrap_or_else(|_| "false".to_string());
                    if has_work == "true" {
                        continue;
                    }
                    return Ok(IdleReason::AllWorkDone);
                }
                Ok(Err(e)) => return Err(EventLoopError::Js(e)),
                Err(_elapsed) => {
                    // Tick timed out — still has pending work, loop again.
                    continue;
                }
            }
        }
    }

    /// Execute a script and return the result as a string.
    pub fn execute_script(&mut self, code: &str) -> Result<String, EventLoopError> {
        self.runtime
            .execute_script(code)
            .map_err(EventLoopError::Js)
    }

    /// Execute a script then run until idle.
    pub async fn execute_and_run(
        &mut self,
        code: &str,
        timeout: Duration,
    ) -> Result<IdleReason, EventLoopError> {
        self.execute_script(code)?;
        self.run_until_idle(timeout).await
    }

    /// Get a reference to the underlying JS runtime.
    pub fn runtime(&self) -> &BrowserJsRuntime {
        &self.runtime
    }

    /// Get a mutable reference to the underlying JS runtime.
    pub fn runtime_mut(&mut self) -> &mut BrowserJsRuntime {
        &mut self.runtime
    }

    /// Consume the event loop and return the runtime.
    pub fn into_runtime(self) -> BrowserJsRuntime {
        self.runtime
    }
}

impl Default for BrowserEventLoop {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_loop() -> BrowserEventLoop {
        BrowserEventLoop::new()
    }

    #[tokio::test]
    async fn idle_detection_no_work() {
        let mut evloop = create_loop();
        let reason = evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();
        assert_eq!(reason, IdleReason::AllWorkDone);
    }

    #[tokio::test]
    async fn set_timeout_fires() {
        let mut evloop = create_loop();
        evloop
            .execute_script(
                r#"
                globalThis.__timerResult = 'pending';
                setTimeout(() => { globalThis.__timerResult = 'timer fired'; }, 50);
                "#,
            )
            .unwrap();

        let reason = evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();
        assert_eq!(reason, IdleReason::AllWorkDone);

        let result = evloop.execute_script("globalThis.__timerResult").unwrap();
        assert_eq!(result, "timer fired");
    }

    #[tokio::test]
    async fn promise_resolves() {
        let mut evloop = create_loop();
        evloop
            .execute_script(
                r#"
                globalThis.__promiseResult = 'pending';
                Promise.resolve().then(() => { globalThis.__promiseResult = 'resolved'; });
                "#,
            )
            .unwrap();

        evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();

        let result = evloop.execute_script("globalThis.__promiseResult").unwrap();
        assert_eq!(result, "resolved");
    }

    #[tokio::test]
    async fn chained_set_timeout() {
        let mut evloop = create_loop();
        evloop
            .execute_script(
                r#"
                globalThis.__chainResult = '';
                setTimeout(() => {
                    globalThis.__chainResult += '1';
                    setTimeout(() => {
                        globalThis.__chainResult += '2';
                    }, 10);
                }, 10);
                "#,
            )
            .unwrap();

        evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();

        let result = evloop.execute_script("globalThis.__chainResult").unwrap();
        assert_eq!(result, "12");
    }

    #[tokio::test]
    async fn request_animation_frame() {
        let mut evloop = create_loop();
        evloop
            .execute_script(
                r#"
                globalThis.__rafResult = 'pending';
                requestAnimationFrame((ts) => {
                    globalThis.__rafResult = 'raf:' + (typeof ts);
                });
                "#,
            )
            .unwrap();

        evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();

        let result = evloop.execute_script("globalThis.__rafResult").unwrap();
        assert_eq!(result, "raf:number");
    }

    #[tokio::test]
    async fn raf_cancel() {
        let mut evloop = create_loop();
        evloop
            .execute_script(
                r#"
                globalThis.__rafCancelResult = 'not fired';
                const id = requestAnimationFrame(() => {
                    globalThis.__rafCancelResult = 'should not fire';
                });
                cancelAnimationFrame(id);
                "#,
            )
            .unwrap();

        evloop.run_until_idle(Duration::from_secs(5)).await.unwrap();

        let result = evloop
            .execute_script("globalThis.__rafCancelResult")
            .unwrap();
        assert_eq!(result, "not fired");
    }

    #[tokio::test]
    async fn timeout_respected() {
        let mut evloop = create_loop();
        // Schedule a timer far in the future
        evloop
            .execute_script("setTimeout(() => {}, 10000);")
            .unwrap();

        let reason = evloop
            .run_until_idle(Duration::from_millis(200))
            .await
            .unwrap();
        assert_eq!(reason, IdleReason::Timeout);
    }

    #[tokio::test]
    async fn execute_and_run_smoke() {
        let mut evloop = create_loop();
        evloop.execute_script("globalThis.__ear = 'init';").unwrap();

        let reason = evloop
            .execute_and_run(
                "setTimeout(() => { globalThis.__ear = 'done'; }, 10);",
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        assert_eq!(reason, IdleReason::AllWorkDone);
        let result = evloop.execute_script("globalThis.__ear").unwrap();
        assert_eq!(result, "done");
    }
}
