use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread,
};

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
    Shutdown,
}

pub(crate) struct PersistenceHandle {
    tx: Sender<PersistenceCommand>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for PersistenceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersistenceHandle").finish_non_exhaustive()
    }
}

impl PersistenceHandle {
    pub(crate) fn start(storage: Arc<dyn Storage>) -> Result<Self, DownloadError> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

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
                        PersistenceCommand::Upsert { record, reply } => {
                            let result = runtime.block_on(storage.upsert(record.as_ref()));
                            let _ = reply.send(result);
                        }
                        PersistenceCommand::Delete { id, reply } => {
                            let result = runtime.block_on(storage.delete(id));
                            let _ = reply.send(result);
                        }
                        PersistenceCommand::Shutdown => break,
                    }
                }
            })
            .map_err(|error| DownloadError::Storage(error.to_string()))?;

        ready_rx
            .recv()
            .map_err(|error| DownloadError::Storage(error.to_string()))??;

        Ok(Self {
            tx,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub(crate) fn upsert(&self, record: DownloadRecord) -> Result<(), DownloadError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(PersistenceCommand::upsert(Box::new(record), reply_tx))
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| DownloadError::Storage("persistence worker stopped".to_string()))?
    }

    pub(crate) fn delete(&self, id: DownloadId) -> Result<(), DownloadError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(PersistenceCommand::delete(id, reply_tx))
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
        let _ = self.tx.send(PersistenceCommand::Shutdown);
        if let Some(worker) = self.worker.lock().ok().and_then(|mut guard| guard.take()) {
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
