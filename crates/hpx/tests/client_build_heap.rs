//! Heap regression test: building and dropping clients must not grow the heap.
//!
//! Lives in its own test binary because it installs a counting global
//! allocator, which would otherwise also count other tests' allocations.
//!
//! It checks whichever TLS backend the feature set compiles in: BoringSSL under
//! `--all-features`, the rustls connector cache under
//! `--no-default-features --features rustls-tls,...`.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

/// Bytes currently allocated through [`CountingAlloc`].
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

/// The system allocator, keeping count of its live bytes. The default
/// `realloc` and `alloc_zeroed` go through `alloc` and `dealloc`, so they are
/// counted too.
struct CountingAlloc;

// SAFETY: every call is forwarded unchanged to `System`, which upholds the
// `GlobalAlloc` contract; the only addition is an atomic counter update.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller passes a `ptr` that `alloc` returned for `layout`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Clients built after the warm-up one. A leak of one TLS connector set per
/// client (about 300 KB with the webpki roots) adds up to megabytes here.
const CLIENTS: usize = 20;

/// Live-heap growth tolerated across all of [`CLIENTS`], far below the cost of
/// retaining a single client's TLS connectors.
const ALLOWED_GROWTH_BYTES: usize = 64 * 1024;

#[test]
fn dropped_clients_do_not_grow_the_heap() {
    // The first build fills process-wide state such as the TLS connector
    // cache; measure only what the later builds leave behind.
    drop(hpx::Client::new());
    let baseline = LIVE_BYTES.load(Ordering::Relaxed);

    for _ in 0..CLIENTS {
        drop(hpx::Client::new());
    }

    let grown = LIVE_BYTES.load(Ordering::Relaxed).saturating_sub(baseline);
    assert!(
        grown < ALLOWED_GROWTH_BYTES,
        "{CLIENTS} built and dropped clients left {grown} bytes live"
    );
}
