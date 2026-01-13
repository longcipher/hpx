//! Benchmarks for connection pool performance.
//!
//! These benchmarks measure the overhead of connection pool operations
//! including acquire, release, and concurrent access patterns.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use parking_lot::{Mutex, RwLock};

/// A simple mock connection for benchmarking
#[derive(Debug)]
#[allow(dead_code)]
struct MockConnection {
    id: usize,
    host: String,
}

impl MockConnection {
    fn new(id: usize, host: &str) -> Self {
        Self {
            id,
            host: host.to_string(),
        }
    }
}

/// A simple connection pool using Mutex
struct MutexPool {
    connections: Mutex<Vec<MockConnection>>,
    next_id: AtomicUsize,
    host: String,
}

impl MutexPool {
    fn new(host: &str, initial_size: usize) -> Self {
        let connections: Vec<MockConnection> = (0..initial_size)
            .map(|i| MockConnection::new(i, host))
            .collect();
        Self {
            connections: Mutex::new(connections),
            next_id: AtomicUsize::new(initial_size),
            host: host.to_string(),
        }
    }

    fn acquire(&self) -> Option<MockConnection> {
        self.connections.lock().pop()
    }

    fn release(&self, conn: MockConnection) {
        self.connections.lock().push(conn);
    }

    fn acquire_or_create(&self) -> MockConnection {
        if let Some(conn) = self.acquire() {
            conn
        } else {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            MockConnection::new(id, &self.host)
        }
    }
}

/// A connection pool using RwLock for reads
struct RwLockPool {
    connections: RwLock<Vec<MockConnection>>,
    next_id: AtomicUsize,
    host: String,
}

impl RwLockPool {
    fn new(host: &str, initial_size: usize) -> Self {
        let connections: Vec<MockConnection> = (0..initial_size)
            .map(|i| MockConnection::new(i, host))
            .collect();
        Self {
            connections: RwLock::new(connections),
            next_id: AtomicUsize::new(initial_size),
            host: host.to_string(),
        }
    }

    fn size(&self) -> usize {
        self.connections.read().len()
    }

    fn acquire(&self) -> Option<MockConnection> {
        self.connections.write().pop()
    }

    fn release(&self, conn: MockConnection) {
        self.connections.write().push(conn);
    }

    fn acquire_or_create(&self) -> MockConnection {
        if let Some(conn) = self.acquire() {
            conn
        } else {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            MockConnection::new(id, &self.host)
        }
    }
}

/// Benchmark pool acquire/release operations
fn bench_pool_acquire_release(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool_acquire_release");

    for pool_size in [10, 100, 1000].iter() {
        let mutex_pool = MutexPool::new("example.com", *pool_size);
        let rwlock_pool = RwLockPool::new("example.com", *pool_size);

        group.bench_with_input(
            BenchmarkId::new("mutex_pool", pool_size),
            &mutex_pool,
            |b, pool| {
                b.iter(|| {
                    let conn = pool.acquire_or_create();
                    pool.release(conn);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("rwlock_pool", pool_size),
            &rwlock_pool,
            |b, pool| {
                b.iter(|| {
                    let conn = pool.acquire_or_create();
                    pool.release(conn);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark pool size checking (read operation)
fn bench_pool_size_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool_size_check");

    let rwlock_pool = RwLockPool::new("example.com", 100);

    group.bench_function("rwlock_read", |b| {
        b.iter(|| rwlock_pool.size());
    });

    group.finish();
}

/// Benchmark host-keyed pool lookup
fn bench_host_keyed_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("host_keyed_pool");

    // Create pools for multiple hosts
    let hosts = [
        "api.example.com",
        "api.another.com",
        "api.third.com",
        "cdn.example.com",
        "auth.example.com",
    ];

    let pools: HashMap<String, Arc<MutexPool>> = hosts
        .iter()
        .map(|h| (h.to_string(), Arc::new(MutexPool::new(h, 10))))
        .collect();

    let pools_rwlock: RwLock<HashMap<String, Arc<MutexPool>>> = RwLock::new(pools.clone());

    // Benchmark lookup by host
    group.bench_function("hashmap_lookup", |b| {
        b.iter(|| pools.get("api.example.com"));
    });

    group.bench_function("rwlock_hashmap_lookup", |b| {
        b.iter(|| pools_rwlock.read().get("api.example.com").cloned());
    });

    // Benchmark full acquire cycle with host lookup
    group.bench_function("full_acquire_cycle", |b| {
        b.iter(|| {
            if let Some(pool) = pools.get("api.example.com") {
                let conn = pool.acquire_or_create();
                pool.release(conn);
            }
        });
    });

    group.finish();
}

/// Benchmark concurrent pool access (single-threaded simulation)
fn bench_simulated_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulated_concurrent");

    let pool = Arc::new(MutexPool::new("example.com", 100));

    // Simulate multiple sequential operations (as would happen with contention)
    group.bench_function("sequential_10_ops", |b| {
        b.iter(|| {
            for _ in 0..10 {
                let conn = pool.acquire_or_create();
                // Simulate some work
                std::hint::black_box(&conn);
                pool.release(conn);
            }
        });
    });

    group.bench_function("sequential_100_ops", |b| {
        b.iter(|| {
            for _ in 0..100 {
                let conn = pool.acquire_or_create();
                std::hint::black_box(&conn);
                pool.release(conn);
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_pool_acquire_release,
    bench_pool_size_check,
    bench_host_keyed_pool,
    bench_simulated_concurrent_access
);
criterion_main!(benches);
