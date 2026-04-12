use std::{
    borrow::Borrow,
    collections::hash_map::Entry,
    hash::{Hash, Hasher},
};

use boring::ssl::{SslSession, SslSessionRef, SslVersion};
use parking_lot::Mutex;
use schnellru::ByLength;

use crate::hash::{HASHER, HashMap, LruMap};

const DEFAULT_SHARD_COUNT: usize = 16;

/// A typed key for indexing TLS sessions in the cache.
///
/// This wrapper provides type safety and allows different key types
/// (e.g., hostname, connection parameters) to be used for session lookup.
#[derive(Hash, PartialEq, Eq, Clone)]
pub struct SessionKey<T>(pub T);

/// A hashable wrapper around `SslSession` for use in hash-based collections.
///
/// Uses the session ID for hashing and equality, enabling efficient
/// storage and lookup in HashMap/HashSet while maintaining session semantics.
#[derive(Clone)]
struct HashSession(SslSession);

impl PartialEq for HashSession {
    fn eq(&self, other: &HashSession) -> bool {
        self.0.id() == other.0.id()
    }
}

impl Eq for HashSession {}

impl Hash for HashSession {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.0.id().hash(state);
    }
}

impl Borrow<[u8]> for HashSession {
    fn borrow(&self) -> &[u8] {
        self.0.id()
    }
}

/// A two-level cache for TLS sessions organized by host keys with LRU eviction.
///
/// Maintains both forward (key → sessions) and reverse (session → key) lookups
/// for efficient session storage, retrieval, and cleanup operations.
pub struct SessionCache<T> {
    shards: Vec<Mutex<SessionCacheShard<T>>>,
    shard_count: usize,
}

struct SessionCacheShard<T> {
    reverse: HashMap<HashSession, SessionKey<T>>,
    per_host_sessions: HashMap<SessionKey<T>, LruMap<HashSession, ()>>,
    per_host_session_capacity: usize,
}

impl<T> SessionCacheShard<T>
where
    T: Hash + Eq + Clone,
{
    fn new(per_host_session_capacity: usize) -> Self {
        Self {
            per_host_sessions: HashMap::with_hasher(HASHER),
            reverse: HashMap::with_hasher(HASHER),
            per_host_session_capacity,
        }
    }

    fn insert(&mut self, key: SessionKey<T>, session: SslSession) {
        let per_host_sessions = self
            .per_host_sessions
            .entry(key.clone())
            .or_insert_with(|| {
                LruMap::with_hasher(ByLength::new(self.per_host_session_capacity as _), HASHER)
            });

        if per_host_sessions.len() >= self.per_host_session_capacity
            && let Some((evicted_session, _)) = per_host_sessions.pop_oldest()
        {
            self.reverse.remove(&evicted_session);
        }

        let session = HashSession(session);
        per_host_sessions.insert(session.clone(), ());
        self.reverse.insert(session, key);
    }

    fn get(&mut self, key: &SessionKey<T>) -> Option<SslSession> {
        let session = {
            let per_host_sessions = self.per_host_sessions.get_mut(key)?;
            per_host_sessions.peek_oldest()?.0.clone().0
        };

        if session.protocol_version() == SslVersion::TLS1_3 {
            self.remove(&session);
        }

        Some(session)
    }

    fn remove(&mut self, session: &SslSessionRef) {
        let key = match self.reverse.remove(session.id()) {
            Some(key) => key,
            None => return,
        };

        if let Entry::Occupied(mut per_host_sessions) = self.per_host_sessions.entry(key) {
            per_host_sessions
                .get_mut()
                .remove(&HashSession(session.to_owned()));
            if per_host_sessions.get().is_empty() {
                per_host_sessions.remove();
            }
        }
    }
}

impl<T> SessionCache<T>
where
    T: Hash + Eq + Clone,
{
    pub fn with_capacity(per_host_session_capacity: usize) -> SessionCache<T> {
        Self::with_capacity_and_shards(per_host_session_capacity, DEFAULT_SHARD_COUNT)
    }

    fn with_capacity_and_shards(
        per_host_session_capacity: usize,
        shard_count: usize,
    ) -> SessionCache<T> {
        let shard_count = shard_count.max(1);
        let shards = (0..shard_count)
            .map(|_| Mutex::new(SessionCacheShard::new(per_host_session_capacity)))
            .collect();

        SessionCache {
            shards,
            shard_count,
        }
    }

    fn shard_count(&self) -> usize {
        self.shard_count
    }

    fn shard_index(&self, key: &SessionKey<T>) -> usize {
        (HASHER.hash_one(key) as usize) % self.shard_count
    }

    pub fn insert(&self, key: SessionKey<T>, session: SslSession) {
        let shard_index = self.shard_index(&key);
        self.shards[shard_index].lock().insert(key, session);
    }

    pub fn get(&self, key: &SessionKey<T>) -> Option<SslSession> {
        let shard_index = self.shard_index(key);
        self.shards[shard_index].lock().get(key)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use boring::ssl::{
        SslAcceptor, SslConnector, SslFiletype, SslMethod, SslSessionCacheMode, SslVerifyMode,
        SslVersion,
    };
    use tokio::{io::AsyncReadExt, net::TcpListener};

    use super::{SessionCache, SessionKey};

    const SERVER_CERT_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.crt");
    const SERVER_KEY_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mtls/server.key");

    fn tls_acceptor(version: SslVersion) -> SslAcceptor {
        let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        acceptor
            .set_certificate_chain_file(SERVER_CERT_PATH)
            .unwrap();
        acceptor
            .set_private_key_file(SERVER_KEY_PATH, SslFiletype::PEM)
            .unwrap();
        acceptor.set_min_proto_version(Some(version)).unwrap();
        acceptor.set_max_proto_version(Some(version)).unwrap();
        acceptor
            .set_session_id_context(b"session-cache-tests")
            .unwrap();
        acceptor.check_private_key().unwrap();
        acceptor.build()
    }

    async fn capture_session(version: SslVersion) -> boring::ssl::SslSession {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tls_acceptor(version);

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _stream = tokio_boring::accept(&acceptor, stream).await.unwrap();
        });

        let sender = Arc::new(Mutex::new(None));
        let (tx, rx) = tokio::sync::oneshot::channel();
        sender.lock().unwrap().replace(tx);

        let mut connector = SslConnector::builder(SslMethod::tls()).unwrap();
        connector.set_verify(SslVerifyMode::NONE);
        connector.set_min_proto_version(Some(version)).unwrap();
        connector.set_max_proto_version(Some(version)).unwrap();
        connector
            .set_session_cache_mode(SslSessionCacheMode::CLIENT | SslSessionCacheMode::NO_INTERNAL);
        connector.set_new_session_callback({
            let sender = Arc::clone(&sender);
            move |_, session| {
                if let Some(tx) = sender.lock().unwrap().take() {
                    let _ = tx.send(session);
                }
            }
        });

        let config = connector.build().configure().unwrap();
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut stream = tokio_boring::connect(config, "localhost", stream)
            .await
            .unwrap();

        if version == SslVersion::TLS1_3 {
            let mut buf = [0_u8; 1];
            let _ = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut buf)).await;
        }

        let session = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .or_else(|| stream.ssl().session().map(|session| session.to_owned()))
            .unwrap();

        server.await.unwrap();
        session
    }

    async fn capture_distinct_session(
        version: SslVersion,
        existing_id: &[u8],
    ) -> boring::ssl::SslSession {
        for _ in 0..4 {
            let session = capture_session(version).await;
            if session.id() != existing_id {
                return session;
            }
        }

        panic!("failed to capture a distinct TLS session")
    }

    #[test]
    fn shard_count_is_never_zero() {
        let cache = SessionCache::<u64>::with_capacity_and_shards(2, 0);

        assert_eq!(cache.shard_count(), 1);
        assert_eq!(cache.shard_index(&SessionKey(7)), 0);
    }

    #[test]
    fn shard_routing_is_stable_for_a_key() {
        let cache = SessionCache::<u64>::with_capacity_and_shards(2, 4);
        let key = SessionKey(7_u64);
        let index = cache.shard_index(&key);

        assert_eq!(cache.shard_index(&key), index);

        let other = (0_u64..256)
            .map(SessionKey)
            .find(|candidate| *candidate != key && cache.shard_index(candidate) != index)
            .unwrap();

        assert_ne!(cache.shard_index(&other), index);
    }

    #[tokio::test]
    async fn tls12_eviction_cleans_reverse_lookup() {
        let cache = SessionCache::<u64>::with_capacity_and_shards(1, 4);
        let key = SessionKey(1_u64);
        let first = capture_session(SslVersion::TLS1_2).await;
        let second = capture_distinct_session(SslVersion::TLS1_2, first.id()).await;
        let shard_index = cache.shard_index(&key);

        cache.insert(key.clone(), first.clone());
        cache.insert(key.clone(), second.clone());

        {
            let shard = cache.shards[shard_index].lock();
            assert!(!shard.reverse.contains_key(first.id()));
            assert!(shard.reverse.contains_key(second.id()));
        }

        let resumed = cache.get(&key).unwrap();
        assert_eq!(resumed.id(), second.id());
    }

    #[tokio::test]
    async fn tls13_sessions_are_single_use() {
        let cache = SessionCache::<u64>::with_capacity_and_shards(2, 4);
        let key = SessionKey(1_u64);
        let session = capture_session(SslVersion::TLS1_3).await;
        let shard_index = cache.shard_index(&key);

        cache.insert(key.clone(), session.clone());

        let first = cache.get(&key).unwrap();
        assert_eq!(first.id(), session.id());
        assert!(cache.get(&key).is_none());

        let shard = cache.shards[shard_index].lock();
        assert!(!shard.reverse.contains_key(session.id()));
        assert!(!shard.per_host_sessions.contains_key(&key));
    }
}
