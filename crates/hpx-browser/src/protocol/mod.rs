//! Chrome DevTools Protocol (CDP) server for hpx-browser.
//!
//! Provides a WebSocket server that speaks CDP JSON-RPC, enabling
//! Puppeteer and Playwright to drive hpx-browser as a drop-in
//! replacement for headless Chrome.

pub mod server;
pub mod session;
pub mod types;

pub use server::CdpServer;
pub use session::CdpSession;
pub use types::*;
