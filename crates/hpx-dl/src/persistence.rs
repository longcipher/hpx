use std::{sync::Arc, thread};

use crossbeam_channel::{Sender, bounded};

use crate::{
    DownloadError, DownloadId,
    storage::{DownloadRecord, Storage},
};

enum PersistenceCommand {
    Upsert {
        record: Box<DownloadRecord>,
        reply: Sender<Result<(), DownloadError>>,
    },
    Delete {
        id: DownloadId,
        reply: Sender<Result<(), DownloadError>>,
    },
    List {
        reply: Sender<Result<Vec<DownloadRecord>, DownloadError>>,
    },
}

pub(crate) struct PersistenceHandle {
    tx: Option<Sender<PersistenceCommand>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for PersistenceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistenceHandle").finish_non_exhaustive()
    }
}

impl PersistenceHandle {
    pub(crate) fn start(storage: Arc<dyn Storage>) -> Result<Self, DownloadError> {
        let (tx, rx) = bounded(64);
        let (ready_tx, ready_rx) = bounded(1);

        let worker = thread::Builder::new()
            .name("hpx-dl-storage".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(DownloadError::Storage(error.to_string())));
                        return;
                    }
                };

                if ready_tx.send(Ok(())).is_err() {
                    return;
                }

                while let Ok(command) = rx.recv() {
                    match command {
                        PersistenceCommand::Upsert { mut record, reply } => {
                            record.sanitize_for_persistence();
                            let result = runtime.block_on(storage.upsert(record.as_ref()));
                            let _ = reply.send(result);
                        }
                        PersistenceCommand::Delete { id, reply } => {
                            let result = runtime.block_on(storage.delete(id));
                            let _ = reply.send(result);
                        }
                        PersistenceCommand::List { reply } => {
                            let result = runtime.block_on(storage.list());
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|error| DownloadError::Storage(error.to_string()))?;

        ready_rx
            .recv()
            .map_err(|error| DownloadError::Storage(error.to_string()))??;

        Ok(Self {
            tx: Some(tx),
            worker: Some(worker),
        })
    }

    pub(crate) fn upsert(&self, record: DownloadRecord) -> Result<(), DownloadError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx
            .as_ref()
            .ok_or_else(|| DownloadError::Storage("persistence worker stopped".to_string()))?
            .send(PersistenceCommand::upsert(Box::new(record), reply_tx))
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?
    }

    pub(crate) fn delete(&self, id: DownloadId) -> Result<(), DownloadError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx
            .as_ref()
            .ok_or_else(|| DownloadError::Storage("persistence worker stopped".to_string()))?
            .send(PersistenceCommand::delete(id, reply_tx))
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?
    }

    pub(crate) fn list(&self) -> Result<Vec<DownloadRecord>, DownloadError> {
        let (reply_tx, reply_rx) = bounded(1);
        self.tx
            .as_ref()
            .ok_or_else(|| DownloadError::Storage("persistence worker stopped".to_string()))?
            .send(PersistenceCommand::List { reply: reply_tx })
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?
    }

    pub(crate) async fn upsert_async(
        self: Arc<Self>,
        record: DownloadRecord,
    ) -> Result<(), DownloadError> {
        tokio::task::spawn_blocking(move || self.upsert(record))
            .await
            .map_err(|error| DownloadError::TaskJoin(error.to_string()))?
    }
}

impl Drop for PersistenceHandle {
    fn drop(&mut self) {
        // Drop the sender first to close the channel. The worker thread will
        // process all remaining commands in the bounded buffer, then
        // recv() returns Err(RecvError) and the loop exits cleanly.
        self.tx.take();
        // Join the worker to ensure all pending commands are processed.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl PersistenceCommand {
    const fn upsert(record: Box<DownloadRecord>, reply: Sender<Result<(), DownloadError>>) -> Self {
        Self::Upsert { record, reply }
    }

    const fn delete(id: DownloadId, reply: Sender<Result<(), DownloadError>>) -> Self {
        Self::Delete { id, reply }
    }
}

#[cfg(all(test, feature = "test"))]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    use super::PersistenceHandle;
    use crate::{
        DownloadPriority, DownloadRecord, DownloadRequest, DownloadState, MemoryStorage,
        storage::Storage,
    };

    fn make_record() -> DownloadRecord {
        let id = crate::DownloadId::new();
        let now = 1_700_000_000;
        let request = DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        DownloadRecord {
            id,
            request: request.clone(),
            state: DownloadState::Queued,
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
            content_length: Some(10_000),
            bytes_downloaded: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
            segments: Vec::new(),
        }
    }

    #[test]
    fn concurrent_upsert_stress() {
        let storage = Arc::new(MemoryStorage::new());
        let handle =
            PersistenceHandle::start(storage.clone()).expect("failed to start persistence handle");

        let num_threads: usize = 8;
        let upserts_per_thread: usize = 100;
        let expected_total = num_threads * upserts_per_thread;

        let success_count = Arc::new(AtomicUsize::new(0));

        let handle = Arc::new(handle);
        let mut join_handles = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let handle = Arc::clone(&handle);
            let count = Arc::clone(&success_count);
            join_handles.push(thread::spawn(move || {
                for _ in 0..upserts_per_thread {
                    let record = make_record();
                    if handle.upsert(record).is_ok() {
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }

        for jh in join_handles {
            jh.join().expect("thread panicked");
        }

        let actual = success_count.load(Ordering::Relaxed);
        assert_eq!(
            actual, expected_total,
            "expected {expected_total} successful upserts, got {actual}"
        );

        let records = handle.list().expect("list failed");
        assert_eq!(
            records.len(),
            expected_total,
            "expected {expected_total} persisted records, got {}",
            records.len()
        );

        drop(handle);
    }

    #[test]
    fn concurrent_upsert_interleaved_with_list() {
        let storage = Arc::new(MemoryStorage::new());
        let handle =
            PersistenceHandle::start(storage.clone()).expect("failed to start persistence handle");

        let num_threads: usize = 4;
        let upserts_per_thread: usize = 50;
        let expected_total = num_threads * upserts_per_thread;

        let handle = Arc::new(handle);
        let mut join_handles = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let handle = Arc::clone(&handle);
            join_handles.push(thread::spawn(move || {
                for _ in 0..upserts_per_thread {
                    let record = make_record();
                    handle.upsert(record).expect("upsert failed");
                }
            }));
        }

        for jh in join_handles {
            jh.join().expect("thread panicked");
        }

        let records = handle.list().expect("list failed");
        assert_eq!(
            records.len(),
            expected_total,
            "expected {expected_total} records, got {}",
            records.len()
        );

        drop(handle);
    }

    #[test]
    fn upsert_and_delete_concurrent() {
        let storage = Arc::new(MemoryStorage::new());
        let handle =
            PersistenceHandle::start(storage.clone()).expect("failed to start persistence handle");

        let num_records: usize = 200;
        let handle = Arc::new(handle);

        let mut ids = Vec::with_capacity(num_records);

        for _ in 0..num_records {
            let record = make_record();
            let id = record.id;
            handle.upsert(record).expect("upsert failed");
            ids.push(id);
        }

        let mid = ids.len() / 2;
        let keep = &ids[..mid];
        let remove = &ids[mid..];

        let mut join_handles = Vec::with_capacity(2);

        {
            let handle = Arc::clone(&handle);
            let keep = keep.to_vec();
            join_handles.push(thread::spawn(move || {
                for id in keep {
                    let record = make_record_with_id(id);
                    handle.upsert(record).expect("re-upsert failed");
                }
            }));
        }

        {
            let handle = Arc::clone(&handle);
            let remove = remove.to_vec();
            join_handles.push(thread::spawn(move || {
                for id in remove {
                    handle.delete(id).expect("delete failed");
                }
            }));
        }

        for jh in join_handles {
            jh.join().expect("thread panicked");
        }

        let records = handle.list().expect("list failed");
        assert_eq!(
            records.len(),
            mid,
            "expected {mid} records after concurrent delete, got {}",
            records.len()
        );

        drop(handle);
    }

    fn make_record_with_id(id: crate::DownloadId) -> DownloadRecord {
        let now = 1_700_000_000;
        let request = DownloadRequest::builder("https://example.com/file.bin", "/tmp/file.bin")
            .priority(DownloadPriority::Normal)
            .build();
        DownloadRecord {
            id,
            request: request.clone(),
            state: DownloadState::Queued,
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
            content_length: Some(10_000),
            bytes_downloaded: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
            segments: Vec::new(),
        }
    }

    #[test]
    fn drop_drains_pending_commands() {
        let storage = Arc::new(MemoryStorage::new());
        let handle =
            PersistenceHandle::start(storage.clone()).expect("failed to start persistence handle");

        let num_records: usize = 128;
        let expected_total = num_records;

        let handle = Arc::new(handle);
        let mut join_handles = Vec::with_capacity(num_records);

        for _ in 0..num_records {
            let handle = Arc::clone(&handle);
            join_handles.push(thread::spawn(move || {
                let record = make_record();
                handle.upsert(record).expect("upsert failed");
            }));
        }

        for jh in join_handles {
            jh.join().expect("thread panicked");
        }

        drop(handle);

        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let records = rt.block_on(storage.list()).expect("list failed");
        assert_eq!(
            records.len(),
            expected_total,
            "expected {expected_total} records after drop, got {}",
            records.len()
        );
    }
}
