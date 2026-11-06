use std::{
    borrow::Borrow,
    collections::{VecDeque, hash_map::Entry},
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use openssl::ssl::{SslSession, SslSessionRef, SslVersion};
use parking_lot::Mutex;

use crate::hash::{HASHER, HashMap};

const DEFAULT_SHARD_COUNT: usize = 16;

/// The most hosts (session keys) one shard keeps sessions for.
///
/// Sessions are capped per host, but every distinct host or connection
/// identity adds a key, so without this cap the cache would grow for the
/// whole life of the client. With [`DEFAULT_SHARD_COUNT`] shards the cache
/// holds about 256 hosts; a full shard evicts its least recently used host.
const DEFAULT_HOST_CAPACITY_PER_SHARD: usize = 16;

/// A typed key for indexing TLS sessions in the cache.
///
/// This wrapper provides type safety and allows different key types
/// (e.g., hostname, connection parameters) to be used for session lookup.
#[derive(Hash, PartialEq, Eq, Clone)]
pub(crate) struct SessionKey<T>(pub(crate) T);

/// A hashable wrapper around `SslSession` for use in hash-based collections.
///
/// Uses the session ID for hashing and equality, enabling efficient
/// storage and lookup in HashMap/HashSet while maintaining session semantics.
#[derive(Clone)]
struct HashSession(SslSession);

impl PartialEq for HashSession {
    fn eq(&self, other: &Self) -> bool {
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
/// for efficient session storage, retrieval, and cleanup operations. Each
/// shard keeps at most [`DEFAULT_HOST_CAPACITY_PER_SHARD`] hosts, evicting
/// the least recently used one, and the per-host capacity given at
/// construction, evicting the oldest session.
pub(crate) struct SessionCache<T> {
    shards: Vec<Mutex<SessionCacheShard<T>>>,
    shard_count: usize,
}

struct SessionCacheShard<T> {
    reverse: HashMap<HashSession, SessionKey<T>>,
    per_host_sessions: HashMap<SessionKey<T>, HostSessions>,
    per_host_session_capacity: usize,
    host_capacity: usize,
    /// Advances on every insert and lookup, to order hosts by recency.
    clock: u64,
}

/// The sessions cached for one host, oldest first.
#[derive(Default)]
struct HostSessions {
    sessions: VecDeque<HashSession>,
    /// The shard clock at the host's last insert or lookup.
    last_used: u64,
}

impl<T> SessionCacheShard<T>
where
    T: Hash + Eq + Clone,
{
    fn new(per_host_session_capacity: usize, host_capacity: usize) -> Self {
        Self {
            per_host_sessions: HashMap::with_hasher(HASHER),
            reverse: HashMap::with_hasher(HASHER),
            per_host_session_capacity,
            host_capacity,
            clock: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    fn insert(&mut self, key: SessionKey<T>, session: SslSession) {
        if self.per_host_sessions.len() >= self.host_capacity
            && !self.per_host_sessions.contains_key(&key)
        {
            self.evict_least_recently_used_host();
        }

        let last_used = self.tick();
        let host = self.per_host_sessions.entry(key.clone()).or_default();
        host.last_used = last_used;
        let per_host_sessions = &mut host.sessions;

        if per_host_sessions.len() >= self.per_host_session_capacity
            && let Some(evicted_session) = per_host_sessions.pop_front()
        {
            self.reverse.remove(&evicted_session);
        }

        let session = HashSession(session);
        per_host_sessions.push_back(session.clone());
        self.reverse.insert(session, key);
    }

    /// Returns the session to resume for `key`, as of `now` in seconds since
    /// the Unix epoch.
    fn get(&mut self, key: &SessionKey<T>, now: u64) -> Option<SslSession> {
        let last_used = self.tick();
        let host = self.per_host_sessions.get_mut(key)?;
        host.last_used = last_used;

        // An expired session can only fail to resume, so drop it here rather
        // than offer it until newer sessions push it out.
        let reverse = &mut self.reverse;
        host.sessions.retain(|session| {
            let live = !is_expired(&session.0, now);
            if !live {
                reverse.remove(session);
            }
            live
        });

        // Offer the newest session: the oldest is the first to expire or to
        // be invalidated by a server-side ticket key rotation.
        let Some(session) = host.sessions.back().map(|session| session.0.clone()) else {
            self.per_host_sessions.remove(key);
            return None;
        };

        if session.protocol_version() == SslVersion::TLS1_3 {
            self.remove(&session);
        }

        Some(session)
    }

    /// Drops every session of the least recently used host.
    fn evict_least_recently_used_host(&mut self) {
        let Some(oldest) = self
            .per_host_sessions
            .iter()
            .min_by_key(|(_, host)| host.last_used)
            .map(|(key, _)| key.clone())
        else {
            return;
        };

        if let Some(host) = self.per_host_sessions.remove(&oldest) {
            for session in &host.sessions {
                self.reverse.remove(session);
            }
        }
    }

    fn remove(&mut self, session: &SslSessionRef) {
        let key = match self.reverse.remove(session.id()) {
            Some(key) => key,
            None => return,
        };

        if let Entry::Occupied(mut per_host_sessions) = self.per_host_sessions.entry(key) {
            let sessions = &mut per_host_sessions.get_mut().sessions;
            let target_id = session.id();
            sessions.retain(|s| s.0.id() != target_id);
            if sessions.is_empty() {
                per_host_sessions.remove();
            }
        }
    }
}

impl<T> SessionCache<T>
where
    T: Hash + Eq + Clone,
{
    pub(crate) fn with_capacity(per_host_session_capacity: usize) -> Self {
        Self::with_capacity_and_shards(per_host_session_capacity, DEFAULT_SHARD_COUNT)
    }

    fn with_capacity_and_shards(per_host_session_capacity: usize, shard_count: usize) -> Self {
        let shard_count = shard_count.max(1);
        let shards = (0..shard_count)
            .map(|_| {
                Mutex::new(SessionCacheShard::new(
                    per_host_session_capacity,
                    DEFAULT_HOST_CAPACITY_PER_SHARD,
                ))
            })
            .collect();

        Self {
            shards,
            shard_count,
        }
    }

    #[cfg_attr(not(test), expect(dead_code))] // used in tests
    fn shard_count(&self) -> usize {
        self.shard_count
    }

    fn shard_index(&self, key: &SessionKey<T>) -> usize {
        (HASHER.hash_one(key) as usize) % self.shard_count
    }

    pub(crate) fn insert(&self, key: SessionKey<T>, session: SslSession) {
        let shard_index = self.shard_index(&key);
        self.shards[shard_index].lock().insert(key, session);
    }

    pub(crate) fn get(&self, key: &SessionKey<T>) -> Option<SslSession> {
        let shard_index = self.shard_index(key);
        let now = unix_now();
        self.shards[shard_index].lock().get(key, now)
    }
}

/// Seconds since the Unix epoch, the clock session lifetimes are kept on.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Returns whether `session` is past the lifetime its server granted, as of
/// `now` in seconds since the Unix epoch.
fn is_expired(session: &SslSessionRef, now: u64) -> bool {
    // `time()` is a C `long` here, so widen everything to add and compare
    // without overflow or sign conversions.
    i128::from(session.time()) + i128::from(session.timeout()) <= i128::from(now)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use openssl::ssl::{
        SslAcceptor, SslConnector, SslFiletype, SslMethod, SslSessionCacheMode, SslVerifyMode,
        SslVersion,
    };
    use tokio::{io::AsyncReadExt, net::TcpListener};

    use super::{SessionCache, SessionCacheShard, SessionKey, unix_now};

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

    async fn capture_session(version: SslVersion) -> openssl::ssl::SslSession {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tls_acceptor(version);

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ssl = openssl::ssl::Ssl::new(acceptor.context()).unwrap();
            let stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
            let mut stream = stream;
            std::pin::Pin::new(&mut stream).accept().await.unwrap();
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
        let ssl = config.into_ssl("localhost").unwrap();
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let stream = tokio_openssl::SslStream::new(ssl, stream).unwrap();
        let mut stream = stream;
        std::pin::Pin::new(&mut stream).connect().await.unwrap();

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
    ) -> openssl::ssl::SslSession {
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

    #[tokio::test]
    async fn tls12_get_offers_the_newest_session() {
        let cache = SessionCache::<u64>::with_capacity_and_shards(2, 4);
        let key = SessionKey(1_u64);
        let older = capture_session(SslVersion::TLS1_2).await;
        let newer = capture_distinct_session(SslVersion::TLS1_2, older.id()).await;

        cache.insert(key.clone(), older);
        cache.insert(key.clone(), newer.clone());

        // Twice: a TLS 1.2 session stays cached once offered.
        assert_eq!(cache.get(&key).unwrap().id(), newer.id());
        assert_eq!(cache.get(&key).unwrap().id(), newer.id());
    }

    #[tokio::test]
    async fn expired_sessions_are_dropped() {
        let mut shard = SessionCacheShard::<u64>::new(2, 4);
        let key = SessionKey(1_u64);
        let session = capture_session(SslVersion::TLS1_2).await;
        let expiry =
            u64::try_from(session.time()).unwrap() + u64::try_from(session.timeout()).unwrap();

        shard.insert(key.clone(), session.clone());
        assert_eq!(shard.get(&key, expiry - 1).unwrap().id(), session.id());

        assert!(shard.get(&key, expiry).is_none());
        assert!(!shard.reverse.contains_key(session.id()));
        assert!(!shard.per_host_sessions.contains_key(&key));
    }

    #[tokio::test]
    async fn least_recently_used_host_is_evicted_at_capacity() {
        let mut shard = SessionCacheShard::<u64>::new(1, 2);
        let first = capture_session(SslVersion::TLS1_2).await;
        let second = capture_distinct_session(SslVersion::TLS1_2, first.id()).await;
        let third = capture_distinct_session(SslVersion::TLS1_2, second.id()).await;
        let now = unix_now();

        shard.insert(SessionKey(1), first.clone());
        shard.insert(SessionKey(2), second.clone());
        // Looking host 1 up leaves host 2 as the least recently used.
        assert!(shard.get(&SessionKey(1), now).is_some());
        shard.insert(SessionKey(3), third.clone());

        assert_eq!(shard.per_host_sessions.len(), 2);
        assert!(shard.per_host_sessions.contains_key(&SessionKey(1)));
        assert!(!shard.per_host_sessions.contains_key(&SessionKey(2)));
        assert!(shard.per_host_sessions.contains_key(&SessionKey(3)));
        assert!(shard.reverse.contains_key(first.id()));
        assert!(!shard.reverse.contains_key(second.id()));
        assert!(shard.reverse.contains_key(third.id()));
    }
}
