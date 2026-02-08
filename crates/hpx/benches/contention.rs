//! Benchmarks for contention hot paths.
//!
//! Measures pool checkout/checkin and H2 ping record_data under concurrency.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use parking_lot::Mutex;

type HostKey = Arc<str>;

const HOST_COUNT: usize = 10;
const OPS_PER_WORKER: usize = 1_000;
const SHARD_COUNT: usize = 16;

trait PoolOps: Send + Sync {
    fn checkout(&self, host: &HostKey) -> Option<u64>;
    fn checkin(&self, host: HostKey, conn: u64);
}

struct SingleLockPool {
    inner: Mutex<HashMap<HostKey, Vec<u64>>>,
}

impl SingleLockPool {
    fn new(hosts: &[HostKey], per_host: usize) -> Self {
        let mut map = HashMap::with_capacity(hosts.len());
        for (idx, host) in hosts.iter().enumerate() {
            let base = idx as u64 * per_host as u64;
            let conns = (0..per_host).map(|i| base + i as u64).collect();
            map.insert(Arc::clone(host), conns);
        }
        Self {
            inner: Mutex::new(map),
        }
    }
}

impl PoolOps for SingleLockPool {
    fn checkout(&self, host: &HostKey) -> Option<u64> {
        self.inner.lock().get_mut(host).and_then(|list| list.pop())
    }

    fn checkin(&self, host: HostKey, conn: u64) {
        self.inner
            .lock()
            .entry(host)
            .or_insert_with(Vec::new)
            .push(conn);
    }
}

struct ShardedLockPool {
    shards: Vec<Mutex<HashMap<HostKey, Vec<u64>>>>,
}

impl ShardedLockPool {
    fn new(hosts: &[HostKey], per_host: usize, shard_count: usize) -> Self {
        let shard_count = shard_count.max(1);
        let mut shards = Vec::with_capacity(shard_count);
        for _ in 0..shard_count {
            shards.push(Mutex::new(HashMap::new()));
        }
        for (idx, host) in hosts.iter().enumerate() {
            let base = idx as u64 * per_host as u64;
            let conns: Vec<u64> = (0..per_host).map(|i| base + i as u64).collect();
            let shard = Self::shard_for(&shards, host, shard_count);
            shard.lock().insert(Arc::clone(host), conns);
        }
        Self { shards }
    }

    fn shard_for<'a>(
        shards: &'a [Mutex<HashMap<HostKey, Vec<u64>>>],
        host: &HostKey,
        shard_count: usize,
    ) -> &'a Mutex<HashMap<HostKey, Vec<u64>>> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        host.hash(&mut hasher);
        let idx = (hasher.finish() as usize) % shard_count;
        &shards[idx]
    }
}

impl PoolOps for ShardedLockPool {
    fn checkout(&self, host: &HostKey) -> Option<u64> {
        let shard = Self::shard_for(&self.shards, host, self.shards.len());
        shard.lock().get_mut(host).and_then(|list| list.pop())
    }

    fn checkin(&self, host: HostKey, conn: u64) {
        let shard = Self::shard_for(&self.shards, &host, self.shards.len());
        shard.lock().entry(host).or_insert_with(Vec::new).push(conn);
    }
}

fn run_pool_workload<P: PoolOps + 'static>(pool: Arc<P>, hosts: Arc<Vec<HostKey>>, workers: usize) {
    let mut handles = Vec::with_capacity(workers);
    for worker in 0..workers {
        let pool = Arc::clone(&pool);
        let hosts = Arc::clone(&hosts);
        handles.push(std::thread::spawn(move || {
            for i in 0..OPS_PER_WORKER {
                let host = &hosts[(worker + i) % hosts.len()];
                if let Some(conn) = pool.checkout(host) {
                    pool.checkin(Arc::clone(host), black_box(conn));
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
}

fn bench_pool_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool_checkout_checkin");
    let hosts: Arc<Vec<HostKey>> = Arc::new(
        (0..HOST_COUNT)
            .map(|i| Arc::<str>::from(format!("host-{i}")))
            .collect(),
    );

    for &workers in &[1usize, 4, 16] {
        let single_pool = Arc::new(SingleLockPool::new(&hosts, workers));
        let sharded_pool = Arc::new(ShardedLockPool::new(&hosts, workers, SHARD_COUNT));

        group.bench_with_input(
            BenchmarkId::new("single_lock", workers),
            &workers,
            |b, &workers| {
                b.iter(|| run_pool_workload(Arc::clone(&single_pool), Arc::clone(&hosts), workers));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sharded", workers),
            &workers,
            |b, &workers| {
                b.iter(|| {
                    run_pool_workload(Arc::clone(&sharded_pool), Arc::clone(&hosts), workers)
                });
            },
        );
    }

    group.finish();
}

trait RecordOps: Send + Sync {
    fn record_data(&self, len: usize);
}

struct MutexRecorder {
    inner: Mutex<MutexState>,
}

struct MutexState {
    bytes: u64,
    last_read_at: u64,
}

impl MutexRecorder {
    fn new() -> Self {
        Self {
            inner: Mutex::new(MutexState {
                bytes: 0,
                last_read_at: 0,
            }),
        }
    }
}

impl RecordOps for MutexRecorder {
    fn record_data(&self, len: usize) {
        let mut inner = self.inner.lock();
        inner.last_read_at = inner.last_read_at.wrapping_add(1);
        inner.bytes = inner.bytes.wrapping_add(len as u64);
    }
}

struct AtomicRecorder {
    bytes: AtomicU64,
    last_read_at: AtomicU64,
}

impl AtomicRecorder {
    fn new() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            last_read_at: AtomicU64::new(0),
        }
    }
}

impl RecordOps for AtomicRecorder {
    fn record_data(&self, len: usize) {
        self.bytes.fetch_add(len as u64, Ordering::Relaxed);
        self.last_read_at.fetch_add(1, Ordering::Relaxed);
    }
}

fn run_record_workload<R: RecordOps + 'static>(recorder: Arc<R>, workers: usize) {
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let recorder = Arc::clone(&recorder);
        handles.push(std::thread::spawn(move || {
            for _ in 0..OPS_PER_WORKER {
                recorder.record_data(black_box(128));
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
}

fn bench_record_data(c: &mut Criterion) {
    let mut group = c.benchmark_group("h2_record_data");

    for &workers in &[1usize, 4, 16] {
        let mutex_recorder = Arc::new(MutexRecorder::new());
        let atomic_recorder = Arc::new(AtomicRecorder::new());

        group.bench_with_input(
            BenchmarkId::new("mutex", workers),
            &workers,
            |b, &workers| {
                b.iter(|| run_record_workload(Arc::clone(&mutex_recorder), workers));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("atomic", workers),
            &workers,
            |b, &workers| {
                b.iter(|| run_record_workload(Arc::clone(&atomic_recorder), workers));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_pool_contention, bench_record_data);
criterion_main!(benches);
