#![allow(clippy::items_after_test_module)]

use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll, ready},
};

use bytes::Bytes;
use http_body::{Body as HttpBody, SizeHint};
use http_body_util::{BodyExt, combinators::BoxBody};
use pin_project_lite::pin_project;
#[cfg(feature = "stream")]
use {tokio::fs::File, tokio_util::io::ReaderStream};

use super::http::InnerResponseBody;
use crate::error::{BoxError, Error};

pin_project! {
    /// An request body.
    pub struct Body {
        #[pin]
        inner: BodyInner,
    }
}

pin_project! {
    /// A stable, erased response body type for client middleware boundaries.
    pub struct ClientResponseBody {
        #[pin]
        inner: ClientResponseBodyInner,
    }
}

pin_project! {
    #[project = BodyInnerProj]
    enum BodyInner {
        Reusable {
            bytes: Bytes,
        },
        Client {
            #[pin]
            body: Pin<Box<ClientResponseBody>>,
        },
        Boxed {
            #[pin]
            body: BoxBody<Bytes, BoxError>,
        },
    }
}

pin_project! {
    #[project = ClientResponseBodyInnerProj]
    enum ClientResponseBodyInner {
        Reusable {
            bytes: Bytes,
        },
        Inner {
            #[pin]
            body: InnerResponseBody,
        },
        Boxed {
            #[pin]
            body: BoxBody<Bytes, BoxError>,
        },
    }
}

pin_project! {
    /// We can't use `map_frame()` because that loses the hint data (for good reason).
    /// But we aren't transforming the data.
    struct IntoBytesBody<B> {
        #[pin]
        inner: B,
    }
}

// ===== impl Body =====

impl Body {
    /// Returns a reference to the internal data of the `Body`.
    ///
    /// `None` is returned, if the underlying data is a stream.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.inner {
            BodyInner::Reusable { bytes } => Some(bytes.as_ref()),
            BodyInner::Client { body } => body.as_ref().get_ref().as_bytes(),
            BodyInner::Boxed { .. } => None,
        }
    }

    /// Wrap a [`HttpBody`] in a box inside `Body`.
    ///
    /// # Example
    ///
    /// ```
    /// # use hpx::Body;
    /// # use futures_util;
    /// # fn main() {
    /// let content = "hello,world!".to_string();
    ///
    /// let body = Body::wrap(content);
    /// # }
    /// ```
    pub fn wrap<B>(inner: B) -> Body
    where
        B: HttpBody + Send + Sync + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BoxError>,
    {
        Body::boxed(IntoBytesBody { inner }.map_err(Into::into).boxed())
    }

    /// Wrap a futures `Stream` in a box inside `Body`.
    ///
    /// # Example
    ///
    /// ```
    /// # use hpx::Body;
    /// # use futures_util;
    /// # fn main() {
    /// let chunks: Vec<Result<_, ::std::io::Error>> = vec![Ok("hello"), Ok(" "), Ok("world")];
    ///
    /// let stream = futures_util::stream::iter(chunks);
    ///
    /// let body = Body::wrap_stream(stream);
    /// # }
    /// ```
    ///
    /// # Optional
    ///
    /// This requires the `stream` feature to be enabled.
    #[cfg(feature = "stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "stream")))]
    pub fn wrap_stream<S>(stream: S) -> Body
    where
        S: futures_util::stream::TryStream + Send + 'static,
        S::Error: Into<BoxError>,
        Bytes: From<S::Ok>,
    {
        Body::stream(stream)
    }

    #[cfg(any(feature = "stream", feature = "multipart"))]
    pub(crate) fn stream<S>(stream: S) -> Body
    where
        S: futures_util::stream::TryStream + Send + 'static,
        S::Error: Into<BoxError>,
        Bytes: From<S::Ok>,
    {
        use futures_util::TryStreamExt;
        use http_body::Frame;
        use http_body_util::StreamBody;
        use sync_wrapper::SyncStream;

        let body = StreamBody::new(SyncStream::new(
            stream
                .map_ok(Bytes::from)
                .map_ok(Frame::data)
                .map_err(Into::into),
        ));
        Body::boxed(body.boxed())
    }

    #[inline]
    pub(crate) fn empty() -> Body {
        Body::reusable(Bytes::new())
    }

    #[inline]
    pub(crate) fn reusable(chunk: Bytes) -> Body {
        Body {
            inner: BodyInner::Reusable { bytes: chunk },
        }
    }

    #[inline]
    fn boxed(body: BoxBody<Bytes, BoxError>) -> Body {
        Body {
            inner: BodyInner::Boxed { body },
        }
    }

    #[cfg(feature = "multipart")]
    pub(crate) fn content_length(&self) -> Option<u64> {
        match &self.inner {
            BodyInner::Reusable { bytes } => Some(bytes.len() as u64),
            BodyInner::Client { body } => body.size_hint().exact(),
            BodyInner::Boxed { body } => body.size_hint().exact(),
        }
    }

    pub(crate) fn try_clone(&self) -> Option<Body> {
        match &self.inner {
            BodyInner::Reusable { bytes } => Some(Body::reusable(bytes.clone())),
            BodyInner::Client { body } => body.as_ref().get_ref().try_clone().map(Body::from),
            BodyInner::Boxed { .. } => None,
        }
    }

    /// Convert the body into a mutable [`bytes::BytesMut`] if possible.
    ///
    /// This is useful for zero-copy operations where you need a mutable buffer,
    /// such as with SIMD JSON parsing.
    ///
    /// Returns `Some(BytesMut)` if the body is a reusable `Bytes` chunk,
    /// `None` if the body is a streaming body.
    ///
    /// # Example
    ///
    /// ```
    /// use bytes::BytesMut;
    /// use hpx::Body;
    ///
    /// let body = Body::from("hello world");
    /// if let Some(bytes_mut) = body.into_bytes_mut() {
    ///     // Use mutable bytes for zero-copy operations
    ///     println!("Got {} bytes", bytes_mut.len());
    /// }
    /// ```
    pub fn into_bytes_mut(self) -> Option<bytes::BytesMut> {
        match self.inner {
            BodyInner::Reusable { bytes } => {
                // Try to convert without copying if possible
                match bytes.try_into_mut() {
                    Ok(bytes_mut) => Some(bytes_mut),
                    Err(bytes) => Some(bytes::BytesMut::from(bytes.as_ref())),
                }
            }
            BodyInner::Client { .. } => None,
            BodyInner::Boxed { .. } => None,
        }
    }

    /// Get an [`tokio::io::AsyncRead`] adapter for the body.
    ///
    /// This enables efficient streaming of the body using standard async I/O traits.
    ///
    /// # Example
    ///
    /// ```
    /// use hpx::Body;
    /// use tokio::io::AsyncReadExt;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let body = Body::from("hello world");
    /// let mut reader = body.reader();
    ///
    /// let mut buffer = Vec::new();
    /// reader.read_to_end(&mut buffer).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Optional
    ///
    /// This requires the optional `stream` feature to be enabled.
    #[cfg(feature = "stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "stream")))]
    pub fn reader(self) -> impl tokio::io::AsyncRead {
        use futures_util::TryStreamExt;

        tokio_util::io::StreamReader::new(
            http_body_util::BodyDataStream::new(self).map_err(std::io::Error::other),
        )
    }
}

impl Default for Body {
    #[inline]
    fn default() -> Body {
        Body::empty()
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            BodyInner::Reusable { bytes } => f.debug_tuple("Body").field(bytes).finish(),
            BodyInner::Client { .. } => f.write_str("Body(ClientResponseBody(..))"),
            BodyInner::Boxed { .. } => f.write_str("Body(..)"),
        }
    }
}

impl ClientResponseBody {
    /// Wrap any compatible HTTP body into the stable client response body type.
    pub fn wrap<B>(inner: B) -> Self
    where
        B: HttpBody + Send + Sync + 'static,
        B::Data: Into<Bytes>,
        B::Error: Into<BoxError>,
    {
        Self::boxed(IntoBytesBody { inner }.map_err(Into::into).boxed())
    }

    #[inline]
    pub(crate) fn from_inner_response(body: InnerResponseBody) -> Self {
        Self {
            inner: ClientResponseBodyInner::Inner { body },
        }
    }

    #[inline]
    fn boxed(body: BoxBody<Bytes, BoxError>) -> Self {
        Self {
            inner: ClientResponseBodyInner::Boxed { body },
        }
    }

    #[inline]
    fn reusable(bytes: Bytes) -> Self {
        Self {
            inner: ClientResponseBodyInner::Reusable { bytes },
        }
    }

    #[inline]
    fn as_bytes(&self) -> Option<&[u8]> {
        match &self.inner {
            ClientResponseBodyInner::Reusable { bytes } => Some(bytes.as_ref()),
            ClientResponseBodyInner::Inner { .. } | ClientResponseBodyInner::Boxed { .. } => None,
        }
    }

    #[inline]
    fn into_bytes_mut(self) -> Option<bytes::BytesMut> {
        match self.inner {
            ClientResponseBodyInner::Reusable { bytes } => match bytes.try_into_mut() {
                Ok(bytes_mut) => Some(bytes_mut),
                Err(bytes) => Some(bytes::BytesMut::from(bytes.as_ref())),
            },
            ClientResponseBodyInner::Inner { .. } | ClientResponseBodyInner::Boxed { .. } => None,
        }
    }

    #[inline]
    fn try_clone(&self) -> Option<Self> {
        match &self.inner {
            ClientResponseBodyInner::Reusable { bytes } => Some(Self::reusable(bytes.clone())),
            ClientResponseBodyInner::Inner { .. } | ClientResponseBodyInner::Boxed { .. } => None,
        }
    }
}

impl fmt::Debug for ClientResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientResponseBody(..)")
    }
}

impl Default for ClientResponseBody {
    fn default() -> Self {
        Self::from(Bytes::new())
    }
}

impl From<BoxBody<Bytes, BoxError>> for ClientResponseBody {
    fn from(body: BoxBody<Bytes, BoxError>) -> Self {
        Self::boxed(body)
    }
}

impl From<Bytes> for ClientResponseBody {
    fn from(bytes: Bytes) -> Self {
        Self::reusable(bytes)
    }
}

impl From<Vec<u8>> for ClientResponseBody {
    fn from(vec: Vec<u8>) -> Self {
        Self::from(Bytes::from(vec))
    }
}

impl From<String> for ClientResponseBody {
    fn from(string: String) -> Self {
        Self::from(Bytes::from(string))
    }
}

impl From<&'static [u8]> for ClientResponseBody {
    fn from(bytes: &'static [u8]) -> Self {
        Self::from(Bytes::from_static(bytes))
    }
}

impl From<&'static str> for ClientResponseBody {
    fn from(string: &'static str) -> Self {
        Self::from(string.as_bytes())
    }
}

impl From<BoxBody<Bytes, BoxError>> for Body {
    #[inline]
    fn from(body: BoxBody<Bytes, BoxError>) -> Self {
        Self::boxed(body)
    }
}

impl From<ClientResponseBody> for Body {
    #[inline]
    fn from(body: ClientResponseBody) -> Self {
        Self {
            inner: BodyInner::Client {
                body: Box::pin(body),
            },
        }
    }
}

impl From<Bytes> for Body {
    #[inline]
    fn from(bytes: Bytes) -> Body {
        Body::reusable(bytes)
    }
}

impl From<Vec<u8>> for Body {
    #[inline]
    fn from(vec: Vec<u8>) -> Body {
        Body::reusable(vec.into())
    }
}

impl From<&'static [u8]> for Body {
    #[inline]
    fn from(s: &'static [u8]) -> Body {
        Body::reusable(Bytes::from_static(s))
    }
}

impl From<String> for Body {
    #[inline]
    fn from(s: String) -> Body {
        Body::reusable(s.into())
    }
}

impl From<&'static str> for Body {
    #[inline]
    fn from(s: &'static str) -> Body {
        s.as_bytes().into()
    }
}

#[cfg(feature = "stream")]
impl From<File> for Body {
    #[inline]
    fn from(file: File) -> Body {
        Body::wrap_stream(ReaderStream::new(file))
    }
}

impl HttpBody for Body {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.as_mut().project().inner.project() {
            BodyInnerProj::Reusable { bytes } => {
                let out = bytes.split_off(0);
                if out.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(http_body::Frame::data(out))))
                }
            }
            BodyInnerProj::Client { body } => {
                let mut body = body.get_mut().as_mut();
                Poll::Ready(ready!(body.as_mut().poll_frame(cx)).map(|opt_chunk| {
                    opt_chunk.map_err(|err| match err.downcast::<Error>() {
                        Ok(err) => *err,
                        Err(err) => Error::body(err),
                    })
                }))
            }
            BodyInnerProj::Boxed { body } => {
                Poll::Ready(ready!(body.poll_frame(cx)).map(|opt_chunk| {
                    opt_chunk.map_err(|err| match err.downcast::<Error>() {
                        Ok(err) => *err,
                        Err(err) => Error::body(err),
                    })
                }))
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        match &self.inner {
            BodyInner::Reusable { bytes } => SizeHint::with_exact(bytes.len() as u64),
            BodyInner::Client { body } => body.size_hint(),
            BodyInner::Boxed { body } => body.size_hint(),
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        match &self.inner {
            BodyInner::Reusable { bytes } => bytes.is_empty(),
            BodyInner::Client { body } => body.is_end_stream(),
            BodyInner::Boxed { body } => body.is_end_stream(),
        }
    }
}

impl HttpBody for ClientResponseBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.as_mut().project().inner.project() {
            ClientResponseBodyInnerProj::Reusable { bytes } => {
                let out = bytes.split_off(0);
                if out.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(http_body::Frame::data(out))))
                }
            }
            ClientResponseBodyInnerProj::Inner { body } => body.poll_frame(cx),
            ClientResponseBodyInnerProj::Boxed { body } => body.poll_frame(cx),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match &self.inner {
            ClientResponseBodyInner::Reusable { bytes } => SizeHint::with_exact(bytes.len() as u64),
            ClientResponseBodyInner::Inner { body } => body.size_hint(),
            ClientResponseBodyInner::Boxed { body } => body.size_hint(),
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.inner {
            ClientResponseBodyInner::Reusable { bytes } => bytes.is_empty(),
            ClientResponseBodyInner::Inner { body } => body.is_end_stream(),
            ClientResponseBodyInner::Boxed { body } => body.is_end_stream(),
        }
    }
}

// ===== impl IntoBytesBody =====

impl<B> HttpBody for IntoBytesBody<B>
where
    B: HttpBody,
    B::Data: Into<Bytes>,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match ready!(self.project().inner.poll_frame(cx)) {
            Some(Ok(f)) => Poll::Ready(Some(Ok(f.map_data(Into::into)))),
            Some(Err(e)) => Poll::Ready(Some(Err(e))),
            None => Poll::Ready(None),
        }
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
}

#[cfg(test)]
mod tests {
    use http_body::Body as _;

    use super::Body;

    #[test]
    fn test_as_bytes() {
        let test_data = b"Test body";
        let body = Body::from(&test_data[..]);
        assert_eq!(body.as_bytes(), Some(&test_data[..]));
    }

    #[test]
    fn body_exact_length() {
        let empty_body = Body::empty();
        assert!(empty_body.is_end_stream());
        assert_eq!(empty_body.size_hint().exact(), Some(0));

        let bytes_body = Body::reusable("abc".into());
        assert!(!bytes_body.is_end_stream());
        assert_eq!(bytes_body.size_hint().exact(), Some(3));

        // can delegate even when wrapped
        let stream_body = Body::wrap(empty_body);
        assert!(stream_body.is_end_stream());
        assert_eq!(stream_body.size_hint().exact(), Some(0));
    }
}

// ===== AsSendBody trait =====

/// A trait for types that can be converted into an HTTP request body
/// with optional size information.
///
/// Implementations return `Some(len)` when the body size is known
/// ahead of time (allowing `Content-Length` to be set), or `None` when
/// the size is unknown (requiring `Transfer-Encoding: chunked`).
pub trait AsSendBody {
    /// Convert this value into a [`Body`].
    fn into_send_body(self) -> Body;

    /// Returns the content length if known ahead of time.
    fn content_length(&self) -> Option<u64> {
        None
    }
}

impl AsSendBody for Body {
    #[inline]
    fn into_send_body(self) -> Body {
        self
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        self.size_hint().exact()
    }
}

impl AsSendBody for Bytes {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::from(self)
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(self.len() as u64)
    }
}

impl AsSendBody for Vec<u8> {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::from(self)
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(self.len() as u64)
    }
}

impl AsSendBody for String {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::from(self)
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(self.len() as u64)
    }
}

impl AsSendBody for &'static str {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::from(self)
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(self.len() as u64)
    }
}

impl AsSendBody for &'static [u8] {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::from(self)
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(self.len() as u64)
    }
}

impl AsSendBody for () {
    #[inline]
    fn into_send_body(self) -> Body {
        Body::empty()
    }

    #[inline]
    fn content_length(&self) -> Option<u64> {
        Some(0)
    }
}

// Note: &str and &[u8] impls are intentionally omitted to avoid
// conflicting implementations with &'static str and &'static [u8].
// Use .to_owned() or Body::from() directly for non-static references.
