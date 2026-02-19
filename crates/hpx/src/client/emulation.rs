use http::{HeaderMap, HeaderValue};

#[cfg(feature = "http1")]
use super::core::http1::Http1Options;
#[cfg(feature = "http2")]
use super::core::http2::Http2Options;
use super::layer::config::TransportOptions;
use crate::{
    header::{self, OrigHeaderMap},
    tls::TlsOptions,
};

/// Factory trait for creating emulation configurations.
///
/// This trait allows different types (enums, structs, etc.) to provide
/// their own emulation configurations. It's particularly useful for:
/// - Predefined browser profiles
/// - Dynamic configuration based on runtime conditions
/// - User-defined custom emulation strategies
pub trait EmulationFactory {
    /// Creates an [`Emulation`] instance from this factory.
    fn emulation(self) -> Emulation;
}

/// Built-in browser-style emulation profiles provided by `hpx`.
///
/// These profiles provide convenient header-level defaults for common client
/// personas while keeping transport options configurable through
/// [`EmulationBuilder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BrowserProfile {
    /// Chrome-like request headers.
    #[default]
    Chrome,
    /// Firefox-like request headers.
    Firefox,
    /// Safari-like request headers.
    Safari,
    /// Edge-like request headers.
    Edge,
    /// OkHttp-like request headers.
    OkHttp,
}

impl BrowserProfile {
    #[inline]
    const fn user_agent(self) -> &'static str {
        match self {
            Self::Chrome => {
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36"
            }
            Self::Firefox => {
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.0; rv:146.0) Gecko/20100101 Firefox/146.0"
            }
            Self::Safari => {
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
            }
            Self::Edge => {
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0"
            }
            Self::OkHttp => "okhttp/5.0.0",
        }
    }

    #[inline]
    const fn accept(self) -> &'static str {
        match self {
            Self::Firefox => "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            Self::OkHttp => "*/*",
            _ => {
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"
            }
        }
    }
}

/// Builder for creating an [`Emulation`] configuration.
#[derive(Debug)]
#[must_use]
pub struct EmulationBuilder {
    emulation: Emulation,
}

/// HTTP emulation configuration for mimicking different HTTP clients.
///
/// This struct combines transport-layer options (HTTP/1, HTTP/2, TLS) with
/// request-level settings (headers, header case preservation) to provide
/// a complete emulation profile for web browsers, mobile applications,
/// API clients, and other HTTP implementations.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Emulation {
    headers: HeaderMap,
    orig_headers: OrigHeaderMap,
    transport: TransportOptions,
}

// ==== impl EmulationBuilder ====

impl EmulationBuilder {
    /// Sets the  HTTP/1 options configuration.
    #[cfg(feature = "http1")]
    #[inline]
    pub fn http1_options(mut self, opts: Http1Options) -> Self {
        *self.emulation.http1_options_mut() = Some(opts);
        self
    }

    /// Sets the HTTP/2 options configuration.
    #[cfg(feature = "http2")]
    #[inline]
    pub fn http2_options(mut self, opts: Http2Options) -> Self {
        *self.emulation.http2_options_mut() = Some(opts);
        self
    }

    /// Sets the  TLS options configuration.
    #[inline]
    pub fn tls_options(mut self, opts: TlsOptions) -> Self {
        *self.emulation.tls_options_mut() = Some(opts);
        self
    }

    /// Sets the default headers.
    #[inline]
    pub fn headers(mut self, src: HeaderMap) -> Self {
        crate::util::replace_headers(&mut self.emulation.headers, src);
        self
    }

    /// Sets the original headers.
    #[inline]
    pub fn orig_headers(mut self, src: OrigHeaderMap) -> Self {
        self.emulation.orig_headers.extend(src);
        self
    }

    /// Builds the [`Emulation`] instance.
    #[inline]
    pub fn build(self) -> Emulation {
        self.emulation
    }
}

// ==== impl Emulation ====

impl Emulation {
    /// Creates a new [`EmulationBuilder`].
    #[inline]
    pub fn builder() -> EmulationBuilder {
        EmulationBuilder {
            emulation: Emulation::default(),
        }
    }

    /// Returns a mutable reference to the TLS options, if set.
    #[inline]
    pub fn tls_options_mut(&mut self) -> &mut Option<TlsOptions> {
        self.transport.tls_options_mut()
    }

    /// Returns a mutable reference to the HTTP/1 options, if set.
    #[cfg(feature = "http1")]
    #[inline]
    pub fn http1_options_mut(&mut self) -> &mut Option<Http1Options> {
        self.transport.http1_options_mut()
    }

    /// Returns a mutable reference to the HTTP/2 options, if set.
    #[cfg(feature = "http2")]
    #[inline]
    pub fn http2_options_mut(&mut self) -> &mut Option<Http2Options> {
        self.transport.http2_options_mut()
    }

    /// Returns a mutable reference to the emulation headers, if set.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Returns a mutable reference to the original headers, if set.
    #[inline]
    pub fn orig_headers_mut(&mut self) -> &mut OrigHeaderMap {
        &mut self.orig_headers
    }

    /// Decomposes the [`Emulation`] into its components.
    #[inline]
    pub(crate) fn into_parts(self) -> (TransportOptions, HeaderMap, OrigHeaderMap) {
        (self.transport, self.headers, self.orig_headers)
    }
}

impl EmulationFactory for Emulation {
    #[inline]
    fn emulation(self) -> Emulation {
        self
    }
}

impl EmulationFactory for BrowserProfile {
    #[inline]
    fn emulation(self) -> Emulation {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(self.user_agent()),
        );
        headers.insert(header::ACCEPT, HeaderValue::from_static(self.accept()));
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9"),
        );

        match self {
            BrowserProfile::Chrome | BrowserProfile::Edge | BrowserProfile::Safari => {
                headers.insert("sec-fetch-dest", HeaderValue::from_static("document"));
                headers.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
                headers.insert("sec-fetch-site", HeaderValue::from_static("none"));
            }
            BrowserProfile::Firefox | BrowserProfile::OkHttp => {}
        }

        Emulation::builder().headers(headers).build()
    }
}

#[cfg(feature = "http1")]
impl EmulationFactory for Http1Options {
    #[inline]
    fn emulation(self) -> Emulation {
        Emulation::builder().http1_options(self).build()
    }
}

#[cfg(feature = "http2")]
impl EmulationFactory for Http2Options {
    #[inline]
    fn emulation(self) -> Emulation {
        Emulation::builder().http2_options(self).build()
    }
}

impl EmulationFactory for TlsOptions {
    #[inline]
    fn emulation(self) -> Emulation {
        Emulation::builder().tls_options(self).build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_profile_sets_user_agent_header() {
        let emulation = BrowserProfile::Firefox.emulation();
        let headers = emulation.headers;
        assert!(headers.contains_key(header::USER_AGENT));
    }
}
