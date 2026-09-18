//! Runtime-pinned DNS resolver.
//!
//! [`PinnedDns`] wraps a fallback [`Resolve`] implementation with a concurrent
//! map of hostname-to-address pins that can be installed and released while
//! the client is in use. A pinned hostname always resolves to its pinned
//! addresses; every other name delegates to the fallback resolver.
//!
//! # Motivation
//!
//! Static per-client overrides ([`ClientBuilder::resolve`]) cannot express
//! "dial exactly the address that was just validated" when the address is
//! only known at request time. The canonical example is an SSRF guard:
//!
//! 1. Validate the destination URL; the validator resolves the host and
//!    checks every address against an allowlist, yielding one pinned IP.
//! 2. `pin(host, [pinned_ip])`, send the request through the shared client,
//!    then `unpin_if(host, [pinned_ip])`.
//! 3. The connector dials the validated IP even if DNS changes between
//!    validation and send, while TLS SNI and the `Host` header still carry
//!    the original hostname.
//!
//! Because pins live behind the shared resolver, no per-request plumbing is
//! needed in the connector: any [`Client`] built with
//! [`ClientBuilder::dns_resolver`] gains runtime pinning.
//!
//! # Port convention
//!
//! Like [`ClientBuilder::resolve`], ports carried by pinned [`SocketAddr`]s
//! are ignored: an explicitly specified URI port overrides them, otherwise
//! the conventional port for the scheme is used. Pin port `0` unless a test
//! needs a specific port.
//!
//! # Concurrency
//!
//! Pins are protected by a [`parking_lot::RwLock`]; resolution takes the read
//! lock and never blocks on I/O. Concurrent deliveries to the same host each
//! validate before pinning, so last-write-wins is safe (every pinned address
//! passed validation). Use [`PinnedDns::unpin_if`] to release a pin without
//! clearing a newer pin installed by a concurrent request.
//!
//! [`Client`]: crate::Client
//! [`ClientBuilder::resolve`]: crate::ClientBuilder::resolve
//! [`ClientBuilder::dns_resolver`]: crate::ClientBuilder::dns_resolver

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use parking_lot::RwLock;

use super::{
    gai::GaiResolver,
    resolve::{Addrs, IntoResolve, Name, Resolve, Resolving},
};

/// DNS resolver with runtime-installable hostname pins.
///
/// Cheap to clone: the pin map and fallback are shared through `Arc`s.
/// Pass an instance (or a clone) to [`ClientBuilder::dns_resolver`].
///
/// [`ClientBuilder::dns_resolver`]: crate::ClientBuilder::dns_resolver
#[derive(Clone)]
pub struct PinnedDns {
    fallback: Arc<dyn Resolve>,
    pins: Arc<RwLock<HashMap<String, Vec<SocketAddr>>>>,
}

impl std::fmt::Debug for PinnedDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinnedDns")
            .field("pins", &self.pins.read().len())
            .finish_non_exhaustive()
    }
}

impl PinnedDns {
    /// Creates a resolver that pins on top of `fallback`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use hpx::{Client, dns::PinnedDns};
    ///
    /// let pinned = PinnedDns::with_system();
    /// let client = Client::builder()
    ///     .dns_resolver(pinned.clone())
    ///     .build()
    ///     .unwrap();
    /// ```
    #[must_use]
    pub fn new(fallback: impl IntoResolve) -> Self {
        Self {
            fallback: fallback.into_resolve(),
            pins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a resolver with the system (`getaddrinfo`) fallback.
    #[must_use]
    pub fn with_system() -> Self {
        Self::new(GaiResolver::new())
    }

    /// Pins `host` to `addrs`, replacing any previous pin.
    ///
    /// The host should be lowercase (URL parsers already normalize it).
    /// Resolution returns the addresses in the given order.
    pub fn pin(&self, host: &str, addrs: Vec<SocketAddr>) {
        self.pins.write().insert(host.to_string(), addrs);
    }

    /// Removes the pin for `host`, if any.
    pub fn unpin(&self, host: &str) {
        self.pins.write().remove(host);
    }

    /// Removes the pin for `host` only when it equals `expected`.
    ///
    /// Returns `true` when a matching pin was removed. Use this to release a
    /// pin installed by the caller without clearing a newer pin installed
    /// concurrently for the same host.
    pub fn unpin_if(&self, host: &str, expected: &[SocketAddr]) -> bool {
        let mut pins = self.pins.write();
        let matches = pins.get(host).is_some_and(|current| current == expected);
        if matches {
            pins.remove(host);
        }
        matches
    }

    /// Removes all pins.
    pub fn clear(&self) {
        self.pins.write().clear();
    }

    /// Returns the currently pinned addresses for `host`, if any.
    #[must_use]
    pub fn pinned(&self, host: &str) -> Option<Vec<SocketAddr>> {
        self.pins.read().get(host).cloned()
    }
}

impl Resolve for PinnedDns {
    fn resolve(&self, name: Name) -> Resolving {
        if let Some(addrs) = self.pins.read().get(name.as_str()).cloned() {
            let addrs: Addrs = Box::new(addrs.into_iter());
            return Box::pin(std::future::ready(Ok(addrs)));
        }
        self.fallback.resolve(name)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    fn addr(ip: [u8; 4]) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), 0)
    }

    #[derive(Debug)]
    struct LoopbackFallback;

    impl Resolve for LoopbackFallback {
        fn resolve(&self, name: Name) -> Resolving {
            let ip = if name.as_str() == "example.com" {
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))
            } else {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            };
            let addrs: Addrs = Box::new(std::iter::once(SocketAddr::new(ip, 0)));
            Box::pin(std::future::ready(Ok(addrs)))
        }
    }

    #[tokio::test]
    async fn miss_delegates_to_fallback() {
        let pinned = PinnedDns::new(LoopbackFallback);
        let mut addrs = pinned
            .resolve(Name::from("example.com"))
            .await
            .expect("fallback resolves");
        assert_eq!(
            addrs.next(),
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                0
            ))
        );
    }

    #[tokio::test]
    async fn pin_overrides_fallback() {
        let pinned = PinnedDns::new(LoopbackFallback);
        pinned.pin("example.com", vec![addr([203, 0, 113, 7])]);
        let mut addrs = pinned
            .resolve(Name::from("example.com"))
            .await
            .expect("pin resolves");
        assert_eq!(addrs.next(), Some(addr([203, 0, 113, 7])));
        assert!(addrs.next().is_none());
    }

    #[tokio::test]
    async fn unpin_restores_fallback() {
        let pinned = PinnedDns::new(LoopbackFallback);
        pinned.pin("example.com", vec![addr([203, 0, 113, 7])]);
        pinned.unpin("example.com");
        assert!(pinned.pinned("example.com").is_none());
        let mut addrs = pinned
            .resolve(Name::from("example.com"))
            .await
            .expect("resolves");
        assert_eq!(
            addrs.next(),
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                0
            ))
        );
    }

    #[tokio::test]
    async fn unpin_if_only_removes_matching_pin() {
        let pinned = PinnedDns::new(LoopbackFallback);
        pinned.pin("example.com", vec![addr([203, 0, 113, 7])]);
        assert!(!pinned.unpin_if("example.com", &[addr([198, 51, 100, 9])]));
        assert!(pinned.pinned("example.com").is_some());
        assert!(pinned.unpin_if("example.com", &[addr([203, 0, 113, 7])]));
        assert!(pinned.pinned("example.com").is_none());
    }

    #[test]
    fn clones_share_pins() {
        let pinned = PinnedDns::new(LoopbackFallback);
        let clone = pinned.clone();
        pinned.pin("example.com", vec![addr([203, 0, 113, 7])]);
        assert_eq!(
            clone.pinned("example.com"),
            Some(vec![addr([203, 0, 113, 7])])
        );
    }
}
