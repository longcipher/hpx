//! Heap regression test: building and dropping clients must not grow the heap.
//!
//! Lives in its own test binary because it installs `dhat` as the global
//! allocator and `dhat` allows only one profiler per process.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Clients built after the warm-up one. A leak of one TLS connector set per
/// client (about 300 KB with the webpki roots) adds up to megabytes here.
const CLIENTS: usize = 20;

/// Live-heap growth tolerated across all of [`CLIENTS`], far below the cost of
/// retaining a single client's TLS connectors.
const ALLOWED_GROWTH_BYTES: usize = 64 * 1024;

#[test]
fn dropped_clients_do_not_grow_the_heap() {
    let _profiler = dhat::Profiler::builder().testing().build();

    // The first build fills process-wide state such as the TLS connector
    // cache; measure only what the later builds leave behind.
    drop(hpx::Client::new());
    let baseline = dhat::HeapStats::get().curr_bytes;

    for _ in 0..CLIENTS {
        drop(hpx::Client::new());
    }

    let grown = dhat::HeapStats::get().curr_bytes.saturating_sub(baseline);
    assert!(
        grown < ALLOWED_GROWTH_BYTES,
        "{CLIENTS} built and dropped clients left {grown} bytes live"
    );
}
