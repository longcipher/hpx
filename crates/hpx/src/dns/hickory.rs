//! DNS resolution via the [hickory-resolver](https://github.com/hickory-dns/hickory-dns) crate

use std::net::SocketAddr;

use hickory_resolver::{
    TokioResolver,
    config::{LookupIpStrategy, ResolverConfig},
    net::runtime::TokioRuntimeProvider,
};

use super::{Addrs, Name, Resolve, Resolving};

/// Wrapper around an [`TokioResolver`], which implements the `Resolve` trait.
#[derive(Debug, Clone)]
pub struct HickoryDnsResolver {
    /// Tokio-based DNS resolver.
    ///
    /// On initialization, it attempts to load the system's DNS configuration;
    /// if unavailable, it falls back to sensible default settings.
    resolver: TokioResolver,
}

impl HickoryDnsResolver {
    /// Create a new resolver with the default configuration,
    /// which reads from `/etc/resolve.conf`. The options are
    /// overridden to look up for both IPv4 and IPv6 addresses
    /// to work with "happy eyeballs" algorithm.
    pub fn new() -> crate::Result<HickoryDnsResolver> {
        let mut builder = match TokioResolver::builder_tokio() {
            Ok(resolver) => {
                debug!("using system DNS configuration");
                resolver
            }
            Err(_err) => {
                debug!("error reading DNS system conf: {_err}, using defaults");
                TokioResolver::builder_with_config(
                    ResolverConfig::default(),
                    TokioRuntimeProvider::default(),
                )
            }
        };

        builder.options_mut().ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        let resolver = builder.build().map_err(crate::Error::builder)?;

        Ok(HickoryDnsResolver { resolver })
    }
}

impl Resolve for HickoryDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolver = self.clone();
        Box::pin(async move {
            let lookup = resolver.resolver.lookup_ip(name.as_str()).await?;
            let addrs: Addrs = Box::new(
                lookup
                    .iter()
                    .map(|ip_addr| SocketAddr::new(ip_addr, 0))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Ok(addrs)
        })
    }
}
