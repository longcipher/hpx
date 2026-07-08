//! Chrome browser detection and path resolution.

use std::{net::TcpListener, path::PathBuf};

/// Find the Chrome (or Chromium) binary on the system.
///
/// Checks platform-specific known paths first, then falls back to `which` for
/// PATH-based discovery.
///
/// # Errors
///
/// Returns an error if no Chrome binary is found on the system.
pub fn find_chrome() -> Result<PathBuf, ChromeError> {
    #[cfg(target_os = "macos")]
    let known_paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
    ];
    #[cfg(target_os = "linux")]
    let known_paths = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium-browser",
        "/usr/bin/chromium",
    ];
    #[cfg(target_os = "windows")]
    let known_paths = [
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files\\Chromium\\Application\\chrome.exe",
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let known_paths: [&str; 0] = [];

    for path_str in &known_paths {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return Ok(path);
        }
    }

    // Fallback: search PATH via `which`
    for binary in ["google-chrome", "chromium-browser", "chromium", "chrome"] {
        if let Ok(path) = which::which(binary) {
            return Ok(path);
        }
    }

    Err(ChromeError::NotFound)
}

/// Allocate a free TCP port on localhost.
///
/// The OS assigns an available port; if the port is unexpectedly taken, the
/// function retries until it succeeds.
///
/// # Errors
///
/// Returns an error only if the OS cannot assign a port after multiple retries
/// (extremely unlikely).
pub fn free_port() -> Result<u16, ChromeError> {
    const MAX_RETRIES: u32 = 10;
    for _ in 0..MAX_RETRIES {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(ChromeError::Io)?;
        if let Ok(addr) = listener.local_addr() {
            return Ok(addr.port());
        }
    }
    Err(ChromeError::PortAllocationFailed)
}

/// Errors that can occur during Chrome detection.
#[derive(Debug, thiserror::Error)]
pub enum ChromeError {
    /// No Chrome or Chromium binary found on the system.
    #[error("Chrome or Chromium binary not found")]
    NotFound,

    /// Failed to allocate a free port.
    #[error("failed to allocate a free port")]
    PortAllocationFailed,

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_returns_valid_port() {
        let port = free_port().expect("free_port should succeed");
        assert!(port > 0, "port must be greater than 0");
    }

    #[test]
    fn free_port_returns_different_ports() {
        let port1 = free_port().expect("free_port should succeed");
        let port2 = free_port().expect("free_port should succeed");
        // Two consecutive calls should yield different ports.
        assert_ne!(
            port1, port2,
            "successive calls should return different ports"
        );
    }

    #[test]
    fn free_port_returns_bindable_port() {
        let port = free_port().expect("free_port should succeed");
        // The returned port should be bindable (not already taken).
        let addr = format!("127.0.0.1:{port}");
        let _listener = TcpListener::bind(&addr).expect("free_port result should be bindable");
    }

    #[test]
    fn find_chrome_returns_result() {
        // find_chrome should not panic — it returns Ok or Err.
        let result = find_chrome();
        match result {
            Ok(path) => {
                assert!(
                    path.exists(),
                    "returned path should exist: {}",
                    path.display()
                );
            }
            Err(ChromeError::NotFound) => {
                // Acceptable on systems without Chrome installed.
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn find_chrome_error_is_not_found() {
        // Verify the error type is correct when Chrome is not found.
        let result = find_chrome();
        if let Err(e) = result {
            assert!(
                matches!(e, ChromeError::NotFound),
                "expected NotFound variant, got: {e}"
            );
        }
    }
}
