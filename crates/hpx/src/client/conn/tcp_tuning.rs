//! Low-latency TCP socket tuning for HFT-style clients.
//!
//! The knobs exposed here are deliberately Linux-specific in effect:
//!
//! - `SO_BUSY_POLL` lets the kernel poll the NIC receive queue for a bounded
//!   number of microseconds before sleeping, trading CPU for wake-up latency.
//! - `SO_INCOMING_CPU` pins the receive path of a socket to one CPU so it stays
//!   on the same core as the consumer (RPS/RFS alignment).
//! - `SO_PRIORITY` selects the kernel egress queue for the socket.
//! - `TCP_QUICKACK` disables delayed ACK for the *current* ACK state. Linux does
//!   **not** persist this flag: the kernel clears it after each interaction, so
//!   it has to be re-armed after every read. [`QuickAckStream`] does exactly
//!   that for the sockets this client opens, and [`QuickAckRead`] does it for
//!   user-supplied transports.
//!
//! All options are opt-in and conservative: an unset field is never written to
//! the socket, and applying them on a non-Linux target is a validated no-op so
//! the same `ClientBuilder` configuration is portable.
//!
//! # Example
//!
//! ```text
//! use hpx::{Client, TcpTuning};
//!
//! let tuning = TcpTuning::new()
//!     .with_busy_poll_us(50)
//!     .with_incoming_cpu(2)
//!     .with_quickack(true);
//! let client = Client::builder().tcp_low_latency(tuning).build()?;
//! ```
//!
//! The runnable version lives on the public
//! [`ClientBuilder::tcp_low_latency`](crate::ClientBuilder::tcp_low_latency)
//! documentation. This module is crate-private, so a doctest here would trip
//! `rustdoc::private_doc_tests` under `--document-private-items`.

#[cfg(unix)]
use std::os::fd::AsFd;
use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

use crate::{
    client::conn::{Connected, Connection, TlsInfoFactory},
    tls::TlsInfo,
};

/// Validation errors for [`TcpTuning`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpTuningError {
    /// `SO_BUSY_POLL` must be a non-zero microsecond budget.
    ZeroBusyPoll,
    /// `SO_PRIORITY` values above 7 require `CAP_NET_ADMIN` on Linux.
    PriorityOutOfRange,
}

impl fmt::Display for TcpTuningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroBusyPoll => {
                f.write_str("SO_BUSY_POLL budget must be greater than zero microseconds")
            }
            Self::PriorityOutOfRange => {
                f.write_str("SO_PRIORITY must be between 0 and 7 for unprivileged processes")
            }
        }
    }
}

impl std::error::Error for TcpTuningError {}

/// Low-latency socket options applied by the connector at connect time.
///
/// Every field is optional; `Default` applies nothing. On non-Linux targets the
/// options are still validated but not written to the socket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TcpTuning {
    busy_poll_us: Option<u32>,
    incoming_cpu: Option<u32>,
    priority: Option<i32>,
    quickack: bool,
}

impl TcpTuning {
    /// Create an empty (no-op) tuning profile.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            busy_poll_us: None,
            incoming_cpu: None,
            priority: None,
            quickack: false,
        }
    }

    /// Set the `SO_BUSY_POLL` budget in microseconds.
    ///
    /// A value of zero is rejected by [`TcpTuning::validate`].
    #[inline]
    #[must_use]
    pub const fn with_busy_poll_us(mut self, micros: u32) -> Self {
        self.busy_poll_us = Some(micros);
        self
    }

    /// Pin the socket's receive path to a CPU (`SO_INCOMING_CPU`).
    #[inline]
    #[must_use]
    pub const fn with_incoming_cpu(mut self, cpu: u32) -> Self {
        self.incoming_cpu = Some(cpu);
        self
    }

    /// Set the kernel egress queue priority (`SO_PRIORITY`, 0..=7).
    #[inline]
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Enable `TCP_QUICKACK` on connect and after every read via
    /// [`QuickAckRead`].
    #[inline]
    #[must_use]
    pub const fn with_quickack(mut self, enabled: bool) -> Self {
        self.quickack = enabled;
        self
    }

    /// `SO_BUSY_POLL` budget in microseconds.
    #[inline]
    #[must_use]
    pub const fn busy_poll_us(&self) -> Option<u32> {
        self.busy_poll_us
    }

    /// `SO_INCOMING_CPU` target.
    #[inline]
    #[must_use]
    pub const fn incoming_cpu(&self) -> Option<u32> {
        self.incoming_cpu
    }

    /// `SO_PRIORITY` value.
    #[inline]
    #[must_use]
    pub const fn priority(&self) -> Option<i32> {
        self.priority
    }

    /// Whether `TCP_QUICKACK` is enabled.
    #[inline]
    #[must_use]
    pub const fn quickack(&self) -> bool {
        self.quickack
    }

    /// Whether any option is set.
    #[inline]
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.busy_poll_us.is_some()
            || self.incoming_cpu.is_some()
            || self.priority.is_some()
            || self.quickack
    }

    /// Validate the profile before it is applied to a socket.
    ///
    /// # Errors
    ///
    /// Returns [`TcpTuningError`] for a zero busy-poll budget or a priority
    /// outside the unprivileged 0..=7 range.
    #[inline]
    pub const fn validate(&self) -> Result<(), TcpTuningError> {
        if matches!(self.busy_poll_us, Some(0)) {
            return Err(TcpTuningError::ZeroBusyPoll);
        }
        if let Some(priority) = self.priority
            && (priority < 0 || priority > 7)
        {
            return Err(TcpTuningError::PriorityOutOfRange);
        }
        Ok(())
    }

    /// Apply every configured option to `socket`.
    ///
    /// This is called before `connect(2)` on the `socket2::Socket` created by
    /// the HTTP connector. Options that return `ENOPROTOOPT`/`EOPNOTSUPP` are
    /// skipped: a client must still be able to connect on kernels or
    /// containers that lack a given feature. Every other option is still
    /// attempted; the first real socket error is returned afterwards. Invalid
    /// profiles are rejected before any socket call.
    ///
    /// # Errors
    ///
    /// Returns an error when the profile is invalid, or the first non-optional
    /// `setsockopt` failure after every configured option was attempted.
    #[cfg(unix)]
    #[inline]
    pub fn apply<S: std::os::fd::AsFd>(&self, socket: &S) -> io::Result<()> {
        self.validate().map_err(io::Error::other)?;
        #[cfg(target_os = "linux")]
        self.apply_linux(socket.as_fd())?;
        #[cfg(not(target_os = "linux"))]
        let _ = socket;
        Ok(())
    }

    /// Validate the profile on targets without a POSIX socket descriptor.
    #[cfg(not(unix))]
    #[inline]
    pub fn apply<S>(&self, _socket: &S) -> io::Result<()> {
        self.validate().map_err(io::Error::other)
    }

    #[cfg(target_os = "linux")]
    fn apply_linux(&self, fd: std::os::fd::BorrowedFd<'_>) -> io::Result<()> {
        // Apply every configured option even when one is unsupported: a
        // kernel or container without `SO_BUSY_POLL` must not prevent
        // `TCP_QUICKACK` and the remaining options from being set.
        let mut first_error = None;

        if let Some(micros) = self.busy_poll_us {
            let micros = i32::try_from(micros).unwrap_or(i32::MAX);
            record_option(
                &mut first_error,
                set_socket_option(fd, libc::SOL_SOCKET, libc::SO_BUSY_POLL, micros),
            );
        }
        if let Some(cpu) = self.incoming_cpu {
            let cpu = i32::try_from(cpu).unwrap_or(i32::MAX);
            record_option(
                &mut first_error,
                set_socket_option(fd, libc::SOL_SOCKET, libc::SO_INCOMING_CPU, cpu),
            );
        }
        if let Some(priority) = self.priority {
            record_option(
                &mut first_error,
                set_socket_option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, priority),
            );
        }
        if self.quickack {
            record_option(&mut first_error, set_quickack(fd, 1));
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Re-arm `TCP_QUICKACK` on a connected socket.
///
/// Linux clears this flag after each ACK interaction, so this must be called
/// after every read that consumes data. Returns `Ok(())` on non-Linux targets.
#[cfg(unix)]
pub fn tcp_quickack(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    set_quickack(fd, 1)?;
    #[cfg(not(target_os = "linux"))]
    let _ = fd;
    Ok(())
}

/// A TCP stream that re-arms `TCP_QUICKACK` after every read.
///
/// The flag is cleared by the kernel after each ACK interaction, so a
/// long-lived connection has to set it again once per read. The re-arm happens
/// on the socket, below the TLS layer, so every encrypted record read also
/// refreshes the ACK state.
///
/// The client wraps every socket it opens in this type when
/// [`TcpTuning::with_quickack`] is enabled. It is public so that custom
/// connectors can apply the same policy to transports the client does not own.
///
/// The wrapper is transparent on non-Linux targets, where the option does not
/// exist and the re-arm is a no-op.
#[derive(Debug)]
pub struct QuickAckStream {
    inner: TcpStream,
    enabled: bool,
}

impl QuickAckStream {
    /// Wrap `inner`, re-arming `TCP_QUICKACK` after each read when `enabled`.
    #[inline]
    #[must_use]
    pub fn new(inner: TcpStream, enabled: bool) -> Self {
        Self { inner, enabled }
    }

    /// Borrow the wrapped socket.
    #[inline]
    #[must_use]
    pub fn get_ref(&self) -> &TcpStream {
        &self.inner
    }

    /// Unwrap the stream, returning the original socket.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> TcpStream {
        self.inner
    }

    /// Set the `TCP_NODELAY` option on the socket.
    #[inline]
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.inner.set_nodelay(nodelay)
    }

    /// Whether the stream re-arms `TCP_QUICKACK` after each read.
    #[inline]
    #[must_use]
    pub const fn quickack_enabled(&self) -> bool {
        self.enabled
    }

    /// Re-arm `TCP_QUICKACK` on the socket.
    ///
    /// Best-effort: a failed `setsockopt` only costs ACK latency and is never
    /// turned into a read error.
    #[cfg(unix)]
    fn rearm(&self) {
        if let Err(error) = tcp_quickack(self.inner.as_fd()) {
            tracing::trace!(%error, "failed to re-arm TCP_QUICKACK");
        }
    }

    /// Re-arm `TCP_QUICKACK` on the socket.
    ///
    /// The option does not exist outside Linux, so this is a no-op.
    #[cfg(not(unix))]
    fn rearm(&self) {}
}

impl AsyncRead for QuickAckStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if this.enabled && matches!(&result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            // Best-effort: a failed re-arm only costs ACK latency, and must
            // never turn a successful read into an error.
            this.rearm();
        }
        result
    }
}

impl AsyncWrite for QuickAckStream {
    #[inline]
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl Connection for QuickAckStream {
    fn connected(&self) -> Connected {
        self.inner.connected()
    }
}

impl TlsInfoFactory for QuickAckStream {
    fn tls_info(&self) -> Option<TlsInfo> {
        None
    }
}

#[cfg(target_os = "linux")]
fn set_quickack(fd: std::os::fd::BorrowedFd<'_>, value: i32) -> io::Result<()> {
    set_socket_option(fd, libc::IPPROTO_TCP, libc::TCP_QUICKACK, value)
}

#[cfg(target_os = "linux")]
fn record_option(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    let Err(error) = result else {
        return;
    };
    if is_unsupported_option(&error) {
        return;
    }
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(target_os = "linux")]
fn is_unsupported_option(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOPROTOOPT) | Some(libc::EOPNOTSUPP)
    )
}

#[cfg(target_os = "linux")]
fn set_socket_option(
    fd: std::os::fd::BorrowedFd<'_>,
    level: i32,
    option: i32,
    value: i32,
) -> io::Result<()> {
    // SAFETY: `fd` is a borrowed live socket descriptor; `value` is an `i32`
    // whose size matches the kernel's expectation for all four options.
    let rc = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            level,
            option,
            (&raw const value).cast(),
            u32::try_from(size_of::<i32>()).unwrap_or(4),
        )
    };
    if rc == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// An `AsyncRead` adapter that re-arms `TCP_QUICKACK` after each read.
///
/// Wrap the raw TCP stream *below* the TLS layer so that every encrypted-record
/// read also refreshes the ACK state. Re-arming is best-effort: a failed
/// `setsockopt` is traced but never turns a successful read into an error.
#[cfg(unix)]
#[derive(Debug)]
pub struct QuickAckRead<S> {
    inner: S,
    enabled: bool,
}

#[cfg(unix)]
impl<S> QuickAckRead<S> {
    /// Wrap `inner`, enabling or disabling the per-read re-arm.
    #[inline]
    #[must_use]
    pub const fn new(inner: S, enabled: bool) -> Self {
        Self { inner, enabled }
    }

    /// Borrow the wrapped stream.
    #[inline]
    #[must_use]
    pub const fn get_ref(&self) -> &S {
        &self.inner
    }

    /// Mutably borrow the wrapped stream.
    #[inline]
    #[must_use]
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Unwrap the adapter, returning the original stream.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> S {
        self.inner
    }
}

#[cfg(unix)]
impl<S> tokio::io::AsyncRead for QuickAckRead<S>
where
    S: tokio::io::AsyncRead + Unpin + std::os::fd::AsFd,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut this.inner).poll_read(cx, &mut *buf);
        if this.enabled
            && matches!(&result, std::task::Poll::Ready(Ok(())))
            && buf.filled().len() > before
            && let Err(error) = tcp_quickack(this.inner.as_fd())
        {
            tracing::trace!(%error, "failed to re-arm TCP_QUICKACK");
        }
        result
    }
}

#[cfg(unix)]
impl<S> tokio::io::AsyncWrite for QuickAckRead<S>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
    };

    use super::*;

    #[test]
    fn default_profile_is_a_no_op() {
        let tuning = TcpTuning::default();
        assert!(!tuning.is_enabled());
        assert_eq!(tuning.busy_poll_us(), None);
        assert_eq!(tuning.incoming_cpu(), None);
        assert_eq!(tuning.priority(), None);
        assert!(!tuning.quickack());
        assert_eq!(tuning.validate(), Ok(()));
    }

    #[test]
    fn builder_sets_every_knob() {
        let tuning = TcpTuning::new()
            .with_busy_poll_us(50)
            .with_incoming_cpu(3)
            .with_priority(6)
            .with_quickack(true);
        assert_eq!(tuning.busy_poll_us(), Some(50));
        assert_eq!(tuning.incoming_cpu(), Some(3));
        assert_eq!(tuning.priority(), Some(6));
        assert!(tuning.quickack());
        assert!(tuning.is_enabled());
        assert_eq!(tuning.validate(), Ok(()));
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        assert_eq!(
            TcpTuning::new().with_busy_poll_us(0).validate(),
            Err(TcpTuningError::ZeroBusyPoll)
        );
        for priority in [-1, 8, i32::MAX] {
            assert_eq!(
                TcpTuning::new().with_priority(priority).validate(),
                Err(TcpTuningError::PriorityOutOfRange)
            );
        }
        for priority in [0, 7] {
            assert_eq!(TcpTuning::new().with_priority(priority).validate(), Ok(()));
        }
    }

    #[test]
    fn apply_rejects_invalid_profiles_everywhere() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let tuning = TcpTuning::new().with_busy_poll_us(0);
        assert!(tuning.apply(&listener).is_err());
        assert!(TcpTuning::new().apply(&listener).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unsupported_options_are_ignored_but_other_errors_are_kept() {
        assert!(is_unsupported_option(&io::Error::from_raw_os_error(
            libc::ENOPROTOOPT
        )));
        assert!(is_unsupported_option(&io::Error::from_raw_os_error(
            libc::EOPNOTSUPP
        )));
        assert!(!is_unsupported_option(&io::Error::from_raw_os_error(
            libc::EPERM
        )));

        let mut first_error = None;
        record_option(&mut first_error, Ok(()));
        record_option(
            &mut first_error,
            Err(io::Error::from_raw_os_error(libc::ENOPROTOOPT)),
        );
        assert!(first_error.is_none());

        record_option(
            &mut first_error,
            Err(io::Error::from_raw_os_error(libc::EPERM)),
        );
        record_option(
            &mut first_error,
            Err(io::Error::from_raw_os_error(libc::EINVAL)),
        );
        assert_eq!(
            first_error.and_then(|error| error.raw_os_error()),
            Some(libc::EPERM),
            "the first real error is reported after the remaining options run"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn quick_ack_read_passes_bytes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            stream.write_all(b"ping-pong").await.expect("write");
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut wrapped = QuickAckRead::new(stream, true);
        let mut buf = [0u8; 9];
        wrapped.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ping-pong");
        server.await.expect("server");
        let _ = wrapped.into_inner();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn disabled_quick_ack_read_still_passes_bytes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            stream.write_all(b"ok").await.expect("write");
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut wrapped = QuickAckRead::new(stream, false);
        let mut buf = [0u8; 2];
        wrapped.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ok");
        server.await.expect("server");
    }

    #[tokio::test]
    async fn quick_ack_stream_passes_bytes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            stream.write_all(b"ping-pong").await.expect("write");
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut wrapped = QuickAckStream::new(stream, true);
        assert!(wrapped.quickack_enabled());
        let mut buf = [0u8; 9];
        wrapped.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ping-pong");
        server.await.expect("server");
        let _ = wrapped.into_inner();
    }

    #[tokio::test]
    async fn disabled_quick_ack_stream_still_passes_bytes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            stream.write_all(b"ok").await.expect("write");
        });

        let stream = TcpStream::connect(addr).await.expect("connect");
        let mut wrapped = QuickAckStream::new(stream, false);
        assert!(!wrapped.quickack_enabled());
        let mut buf = [0u8; 2];
        wrapped.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ok");
        server.await.expect("server");
    }
}
