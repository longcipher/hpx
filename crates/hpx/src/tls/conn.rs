#[cfg(feature = "boring")]
pub use tokio_boring::SslStream as TlsStream;
#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
pub use tokio_rustls::client::TlsStream;

#[cfg(all(feature = "rustls-tls", not(feature = "boring")))]
pub use super::rustls::*;
#[cfg(feature = "boring")]
pub use crate::tls::boring::*;
