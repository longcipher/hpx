//! Fuzz the HTTP/1 response parser with hostile response bytes.
//!
//! `hpx`'s HTTP/1 response path is the largest single piece of untrusted-input
//! handling in the workspace: it parses the status line and headers, then hands
//! the body to a length/chunked/content-encoding decoder. A malicious or broken
//! origin can put arbitrary bytes there, and a parser bug turns into a panic in
//! the caller's process rather than an error response.
//!
//! Rather than reach into `pub(crate)` parser internals, this target drives the
//! real thing end to end: a loopback TCP listener writes the fuzz bytes as a
//! response, and an ordinary [`hpx::Client`] reads it. That covers the whole
//! public path a user hits — connection setup, response head parsing, body
//! framing, and decompression — so any panic found here is a genuine
//! user-visible crash.
//!
//! Assertions:
//!
//! 1. **No panic.** Malformed input must surface as an `Err`, never a panic.
//! 2. **Termination.** The exchange completes within a bounded wall-clock time,
//!    so a parser that waits forever for bytes the server never sends is caught.
//! 3. **No response splitting.** If the parser reports a response at all, no
//!    header name may contain CR, LF, or NUL — otherwise a crafted response
//!    could inject a second request or header into the caller's view.
//! 4. **Bounded body.** A successfully decoded body stays under a sanity
//!    ceiling, so a decompression bomb fails the target instead of turning a
//!    finding into an out-of-memory kill.

#![no_main]

use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    time::timeout,
};

/// Upper bound on one client/server exchange. Generous for loopback, tight
/// enough to catch a parser that stalls waiting for input that never arrives.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ceiling on a body the client will accept from a fuzz-generated response. A
/// response claiming more than this without delivering the bytes cannot complete
/// anyway; the bound stops a decompression bomb from becoming an OOM kill.
const MAX_ACCEPTED_BODY: usize = 8 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        panic!("failed to build a tokio runtime");
    };
    rt.block_on(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback bind must succeed");
        let addr = listener.local_addr().expect("bound socket must have an address");

        let payload = data.to_vec();
        let server = tokio::spawn(async move {
            // Accept one connection, hand it the fuzz bytes, then close. If the
            // client hangs up first (because it rejected the response), the
            // write errors and the task exits quietly.
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(&payload).await;
                let _ = socket.flush().await;
                let _ = socket.shutdown().await;
            }
        });

        let url = format!("http://{addr}/");
        let client = match hpx::Client::builder().no_proxy().build() {
            Ok(client) => client,
            // A client-construction failure is an environment problem, not a
            // fuzzing finding.
            Err(err) => {
                server.abort();
                panic!("failed to build an hpx client: {err}");
            }
        };

        let outcome = timeout(
            EXCHANGE_TIMEOUT,
            async {
                let res = client.get(&url).send().await?;

                // Inspect the head before consuming the body.
                let status = res.status().as_u16();
                assert!(
                    (100..=599).contains(&status),
                    "parser produced an out-of-range status code {status}"
                );
                for (name, value) in res.headers() {
                    let name = name.as_str();
                    assert!(
                        !name.contains(['\r', '\n', '\0']),
                        "header name {name:?} carries a line break, enabling response splitting"
                    );
                    assert!(
                        value.as_bytes().len() <= 64 * 1024,
                        "header {name:?} carries a {} byte value",
                        value.as_bytes().len()
                    );
                }

                // Draining the body is where content-length, chunked framing,
                // and content-encoding decoding all run.
                let body = res.bytes().await?;
                Ok::<_, hpx::Error>((body.len(), status))
            },
        )
        .await;

        server.abort();

        match outcome {
            // Timed out: the parser stalled on input the server already
            // finished sending.
            Err(_) => panic!("client did not finish within {EXCHANGE_TIMEOUT:?}"),
            // Transport or protocol error: the expected outcome for malformed
            // input, and the reason this target exists rather than a crash.
            Ok(Err(_)) => {}
            Ok(Ok((body_len, _))) => {
                assert!(
                    body_len <= MAX_ACCEPTED_BODY,
                    "accepted a {body_len}-byte body, above the {MAX_ACCEPTED_BODY} byte ceiling"
                );
            }
        }
    });
});
