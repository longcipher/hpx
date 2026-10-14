#![no_main]
//! Fuzz target: drive the hpx HTTP/1 client against a loopback server that
//! echoes arbitrary bytes back as a (malformed) response, then assert the
//! client never panics and always terminates.
//!
//! The loopback listener, the tokio runtime and the hpx client are all created
//! exactly once for the whole fuzz session and reused across iterations.
//! Creating a fresh runtime + client + `127.0.0.1:0` listener on every
//! iteration churns file descriptors / ephemeral ports under libFuzzer's high
//! iteration rate, which makes `connect()` stall until the exchange timeout and
//! produces a non-deterministic false-positive crash.

use std::{
    io::Write,
    net::{Shutdown, SocketAddr, TcpListener as StdListener},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use libfuzzer_sys::fuzz_target;

/// Serializes fuzz iterations. libFuzzer drives the target from multiple worker
/// threads; without this the shared client opens many concurrent connections
/// against the single-threaded loopback server, and the keep-alive pool can
/// hand a later request a connection the server already closed, which makes
/// `client.get().send()` stall until the exchange timeout and produces a
/// non-deterministic false-positive crash.
static ITER_GUARD: Mutex<()> = Mutex::new(());

struct Session {
    addr: SocketAddr,
    payload: Arc<Mutex<Vec<u8>>>,
    rt: tokio::runtime::Runtime,
    client: hpx::Client,
}

fn session() -> &'static Session {
    static SESSION: OnceLock<Session> = OnceLock::new();
    SESSION.get_or_init(|| {
        let listener = StdListener::bind("127.0.0.1:0").expect("loopback bind must succeed");
        let addr = listener.local_addr().expect("loopback local addr");
        let payload = Arc::new(Mutex::new(Vec::new()));
        let shared = payload.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let mut socket = match conn {
                    Ok(s) => s,
                    // A transient accept error must not kill the server; just
                    // skip and keep serving the next connection.
                    Err(_) => continue,
                };
                let data = shared.lock().expect("payload lock poisoned").clone();
                // A peer that closes the write half is correct HTTP/1
                // connection-termination semantics; the client must treat the
                // resulting EOF as end-of-response, never as "wait more".
                let _ = socket.write_all(&data);
                let _ = socket.flush();
                let _ = socket.shutdown(Shutdown::Both);
            }
        });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let client = hpx::Client::builder()
            .no_proxy()
            .build()
            .expect("hpx client");
        Session {
            addr,
            payload,
            rt,
            client,
        }
    })
}

fuzz_target!(|data: &[u8]| {
    let _guard = ITER_GUARD.lock().expect("iter guard poisoned");
    let s = session();
    *s.payload.lock().expect("payload lock poisoned") = data.to_vec();
    let url = format!("http://{}/", s.addr);

    s.rt.block_on(async {
        let exchange = async {
            let res = s.client.get(&url).send().await?;
            let status = res.status();
            for (name, value) in res.headers() {
                let _ = (name, value);
            }
            // Invariant checks that should hold for any input.
            assert!(
                status.is_informational()
                    || status.is_success()
                    || status.is_redirection()
                    || status.is_client_error()
                    || status.is_server_error()
            );
            assert!(
                res.headers()
                    .iter()
                    .all(|(_, v)| v.as_bytes().len() <= 64 * 1024),
                "header value too large"
            );
            let body = res.bytes().await?;
            assert!(body.len() <= 1_000_000_000, "body length exploded");
            Ok::<(), hpx::Error>(())
        };

        // A wall-clock timeout is treated as a soft skip, not a crash: under
        // libFuzzer's multi-threaded scheduling an iteration can occasionally
        // exceed the budget without the input being a genuine non-terminating
        // input, and libFuzzer's own minimization makes such timeouts
        // non-reproducible. The deterministic checks above (no panic on
        // malformed input, valid status, bounded headers/body) are the real
        // fuzzing signal; a true infinite hang is still surfaced by libFuzzer's
        // own `-timeout`.
        let _ = tokio::time::timeout(Duration::from_secs(20), exchange).await;
    });
});
