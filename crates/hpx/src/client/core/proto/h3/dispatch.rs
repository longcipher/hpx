//! Request dispatch helpers for HTTP/3.
//!
//! Extracted from [`super::client::drive_request`] to keep the per-request
//! lifecycle readable. Each function handles one phase of the h3 request/
//! response lifecycle:
//!
//! - [`send_request_body`] — polls body frames and sends via `send_data`,
//!   then calls `finish`.
//! - [`handle_response`] — calls `recv_response`, handles graceful
//!   STOP_SENDING (H3_NO_ERROR) and non-graceful stream errors.
//! - [`collect_response_body`] — drains `recv_data` chunks and delivers
//!   them through an [`IncomingBody`] channel.

use std::pin::Pin;

use bytes::{Buf, Bytes};
use http_body::Body;

use super::client::{stream_error_to_error, stream_error_to_h3_error};
use crate::{
    client::core::{
        Error,
        body::{DecodedLength, Incoming as IncomingBody},
        error::BoxError,
    },
    error::H3Error,
};

/// Result of [`handle_response`]: either `recv_response` succeeded or an
/// early-return response was constructed.
pub(crate) enum RecvResponseResult {
    /// `recv_response` succeeded; the caller should collect the body via
    /// [`collect_response_body`].
    Success(http::Response<()>),
    /// An early-return response was constructed (H3_NO_ERROR STOP_SENDING
    /// or a non-graceful stream error). The caller should return this
    /// response immediately.
    EarlyReturn(http::Response<IncomingBody>),
}

/// Send the request body via `send_data` in a loop, then call `finish`.
///
/// Polls the body for data frames via [`Body::poll_frame`] and sends each
/// chunk via [`h3::client::RequestStream::send_data`]. Trailers are
/// ignored (Phase 1 scope). After all data frames are consumed, calls
/// [`h3::client::RequestStream::finish`] to half-close the send direction.
#[allow(clippy::result_large_err)]
pub(crate) async fn send_request_body<B>(
    stream: &mut h3::client::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    body: &mut B,
) -> Result<(), (Error, Option<http::Request<B>>)>
where
    B: Body + Send + Unpin,
    B::Data: Into<Bytes>,
    B::Error: Into<BoxError>,
{
    loop {
        let frame = match std::future::poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)).await {
            Some(Ok(frame)) => frame,
            Some(Err(e)) => {
                return Err((Error::new_body(e.into()), None::<http::Request<B>>));
            }
            None => break,
        };

        if let Ok(data) = frame.into_data() {
            stream
                .send_data(data.into())
                .await
                .map_err(|e| (stream_error_to_error(e), None::<http::Request<B>>))?;
        }
        // Trailers are ignored for Phase 1 scope.
    }

    stream
        .finish()
        .await
        .map_err(|e| (stream_error_to_error(e), None::<http::Request<B>>))?;

    Ok(())
}

/// Receive the response headers via `recv_response`.
///
/// Handles two error cases:
/// - **Graceful STOP_SENDING** (H3_NO_ERROR / 0x0100): per RFC 9114 §8.1,
///   the server is cleanly canceling the request stream. Returns an empty
///   200 OK response with no body.
/// - **Non-graceful stream errors** (e.g., H3_INTERNAL_ERROR 0x0102):
///   per RFC 9114 §4.4, creates a body channel whose first chunk is the
///   error, so the caller observes the failure when reading the response
///   body rather than during `send_request`.
///
/// On success, returns [`RecvResponseResult::Success`] with the
/// `Response<()>` from h3 so the caller can collect the body.
#[allow(clippy::result_large_err)]
pub(crate) async fn handle_response<B>(
    stream: &mut h3::client::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
) -> Result<RecvResponseResult, (Error, Option<http::Request<B>>)> {
    let response = match stream.recv_response().await {
        Ok(response) => response,
        Err(e) => {
            // H3_NO_ERROR (0x0100) STOP_SENDING is graceful EOF (RFC 9114
            // §8.1). The server is cleanly canceling the request stream
            // without error. Return an empty 200 OK response so the caller
            // observes a graceful shutdown rather than a stream error.
            if matches!(
                &e,
                h3::error::StreamError::RemoteTerminate { code, .. }
                    if code.value() == h3::error::Code::H3_NO_ERROR.value()
            ) {
                let (body_tx, body_rx) =
                    IncomingBody::new_channel(DecodedLength::CHUNKED, /* wanter = */ false);
                drop(body_tx); // Empty body — no data chunks.
                let response = http::Response::builder()
                    .status(http::StatusCode::OK)
                    .body(body_rx)
                    .map_err(|_| {
                        (
                            Error::new_body(H3Error::Other(Box::new(e))),
                            None::<http::Request<B>>,
                        )
                    })?;
                return Ok(RecvResponseResult::EarlyReturn(response));
            }

            // Non-H3_NO_ERROR stream errors (e.g., H3_INTERNAL_ERROR
            // 0x0102) are NOT graceful (RFC 9114 §4.4). Create a body
            // channel whose first (and only) chunk is the error, so the
            // caller observes the failure when reading the response body
            // rather than during `send_request()`. This ensures the error
            // satisfies `is_body()`.
            let h3_err = stream_error_to_h3_error(e);
            let (mut body_tx, body_rx) =
                IncomingBody::new_channel(DecodedLength::CHUNKED, /* wanter = */ false);
            body_tx.send_error(Error::new_body(h3_err));
            let response = http::Response::builder()
                .status(http::StatusCode::OK)
                .body(body_rx)
                .map_err(|_| {
                    (
                        Error::new_body(H3Error::StreamReset {
                            code: 0x0102,
                            stream_id: 0,
                        }),
                        None::<http::Request<B>>,
                    )
                })?;
            return Ok(RecvResponseResult::EarlyReturn(response));
        }
    };

    Ok(RecvResponseResult::Success(response))
}

/// Collect the response body via `recv_data` and deliver chunks through
/// an [`IncomingBody`] channel.
///
/// Drains all `recv_data` chunks into a `Vec<Bytes>`, then spawns a task
/// that delivers them through the channel so the caller can read the body
/// through the standard [`Body`] API.
#[allow(clippy::result_large_err)]
pub(crate) async fn collect_response_body<B>(
    stream: &mut h3::client::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    response: http::Response<()>,
) -> Result<http::Response<IncomingBody>, (Error, Option<http::Request<B>>)> {
    let mut chunks: Vec<Bytes> = Vec::new();
    while let Some(mut data) = stream
        .recv_data()
        .await
        .map_err(|e| (stream_error_to_error(e), None::<http::Request<B>>))?
    {
        let len = data.remaining();
        chunks.push(data.copy_to_bytes(len));
    }

    let (mut body_tx, body_rx) =
        IncomingBody::new_channel(DecodedLength::CHUNKED, /* wanter = */ false);
    let send_task = tokio::spawn(async move {
        for chunk in chunks {
            if std::future::poll_fn(|cx| body_tx.poll_ready(cx))
                .await
                .is_err()
            {
                return;
            }
            if body_tx.try_send_data(chunk).is_err() {
                return;
            }
        }
        drop(body_tx);
    });
    drop(send_task);

    let response = response.map(|_| body_rx);
    Ok(response)
}
