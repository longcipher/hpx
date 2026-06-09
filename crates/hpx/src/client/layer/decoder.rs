//! Middleware for decoding

use std::task::{Context, Poll};

use http::{Request, Response};
use http_body::Body;
use tower::{Layer, Service};
use tower_http::decompression::{self, DecompressionBody, ResponseFuture};

use crate::config::RequestConfig;

/// Configuration for supported content-encoding algorithms.
///
/// `AcceptEncoding` controls which compression formats are enabled for decoding
/// response bodies. Each field corresponds to a specific algorithm and is only
/// available if the corresponding feature is enabled.
#[derive(Clone)]
pub(crate) struct AcceptEncoding {
    #[cfg(feature = "gzip")]
    pub(crate) gzip: bool,
    #[cfg(feature = "brotli")]
    pub(crate) brotli: bool,
    #[cfg(feature = "zstd")]
    pub(crate) zstd: bool,
    #[cfg(feature = "deflate")]
    pub(crate) deflate: bool,
}

/// Layer that adds response body decompression to a service.
#[derive(Clone)]
pub struct DecompressionLayer {
    accept: AcceptEncoding,
}

/// Service that decompresses response bodies based on the [`AcceptEncoding`] configuration.
///
/// Uses `Option` internally to support per-request reconfiguration via
/// `take()` + `replace()`. The invariant (`Some` before every `call`/`poll_ready`)
/// is maintained by the implementation and guarded by `debug_assert`.
#[derive(Clone)]
pub struct Decompression<S>(Option<decompression::Decompression<S>>);

// ===== AcceptEncoding =====

impl Default for AcceptEncoding {
    fn default() -> AcceptEncoding {
        AcceptEncoding {
            #[cfg(feature = "gzip")]
            gzip: true,
            #[cfg(feature = "brotli")]
            brotli: true,
            #[cfg(feature = "zstd")]
            zstd: true,
            #[cfg(feature = "deflate")]
            deflate: true,
        }
    }
}

impl_request_config_value!(AcceptEncoding);

// ===== impl DecompressionLayer =====

impl DecompressionLayer {
    /// Creates a new [`DecompressionLayer`] with the specified [`AcceptEncoding`].
    #[inline(always)]
    pub const fn new(accept: AcceptEncoding) -> Self {
        Self { accept }
    }
}

impl<S> Layer<S> for DecompressionLayer {
    type Service = Decompression<S>;

    #[inline(always)]
    fn layer(&self, service: S) -> Self::Service {
        Decompression(Some(Decompression::<S>::apply_accept(
            decompression::Decompression::new(service),
            &self.accept,
        )))
    }
}

// ===== impl Decompression =====

impl<S> Decompression<S> {
    fn apply_accept(
        mut decoder: decompression::Decompression<S>,
        accept: &AcceptEncoding,
    ) -> decompression::Decompression<S> {
        #[cfg(feature = "gzip")]
        {
            decoder = decoder.gzip(accept.gzip);
        }

        #[cfg(feature = "deflate")]
        {
            decoder = decoder.deflate(accept.deflate);
        }

        #[cfg(feature = "brotli")]
        {
            decoder = decoder.br(accept.brotli);
        }

        #[cfg(feature = "zstd")]
        {
            decoder = decoder.zstd(accept.zstd);
        }

        decoder
    }

    #[inline]
    fn get_inner(&mut self) -> &mut decompression::Decompression<S> {
        self.0
            .as_mut()
            .expect("[BUG] Decompression service not initialized")
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for Decompression<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ReqBody: Body,
    ResBody: Body,
{
    type Response = Response<DecompressionBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    #[inline(always)]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_inner().poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        if let Some(accept) = RequestConfig::<AcceptEncoding>::get(req.extensions()) {
            if let Some(decoder) = self.0.take() {
                self.0.replace(Decompression::apply_accept(decoder, accept));
            }
            debug_assert!(self.0.is_some());
        }

        self.get_inner().call(req)
    }
}
