#![allow(unused)]
//! TLS Key Log Management
//!
//! This module provides utilities for managing TLS key logging, allowing session keys to be
//! written to a file for debugging or analysis (e.g., with Wireshark).
//!
//! The [`KeyLog`] enum lets you control key log behavior, either by respecting the
//! `SSLKEYLOGFILE` environment variable or by specifying a custom file path. Handles are cached
//! globally to avoid duplicate file access.
//!
//! Use [`KeyLog::handle`] to obtain a [`Handle`] for writing keys.
//!
//! ## Feature Gate
//!
//! The keylog module is gated behind the `keylog` Cargo feature (opt-in).
//! Without the feature, `KeyLog::from_env()` returns a no-op and no file I/O occurs.
//! This prevents accidental TLS session key leakage in production.

#[cfg(feature = "keylog")]
mod handle;

use std::{
    borrow::Cow,
    io::{Error, ErrorKind, Result},
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

#[cfg(feature = "keylog")]
pub(crate) use handle::Handle;
#[cfg(feature = "keylog")]
use scc::HashMap as SccHashMap;

/// Specifies the intent for a (TLS) keylogger.
#[cfg(feature = "keylog")]
#[derive(Debug, Clone)]
pub struct KeyLog(Option<Arc<Path>>);

#[cfg(feature = "keylog")]
impl KeyLog {
    /// Creates a [`KeyLog`] based on the `SSLKEYLOGFILE` environment variable.
    pub fn from_env() -> KeyLog {
        match std::env::var("SSLKEYLOGFILE") {
            Ok(ref s) if !s.trim().is_empty() => {
                KeyLog(Some(Arc::from(normalize_path(Path::new(s)))))
            }
            _ => KeyLog(None),
        }
    }

    /// Creates a [`KeyLog`] that writes to the specified file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> KeyLog {
        KeyLog(Some(Arc::from(normalize_path(path.as_ref()))))
    }

    /// Creates a new key log file [`Handle`] based on the policy.
    pub(crate) fn handle(self) -> Result<Handle> {
        static GLOBAL_KEYLOG_CACHE: OnceLock<SccHashMap<Arc<Path>, Handle>> = OnceLock::new();

        let path = self
            .0
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "KeyLog: file path is not specified"))?;

        let cache = GLOBAL_KEYLOG_CACHE.get_or_init(SccHashMap::new);

        // Fast path: check if already cached
        if let Some(entry) = cache.get_sync(path.as_ref()) {
            return Ok(entry.get().clone());
        }

        // Slow path: insert if absent
        let handle = Handle::new(path.clone())?;
        match cache.entry_sync(path) {
            scc::hash_map::Entry::Occupied(entry) => Ok(entry.get().clone()),
            scc::hash_map::Entry::Vacant(entry) => {
                entry.insert_entry(handle.clone());
                Ok(handle)
            }
        }
    }
}

/// No-op [`KeyLog`] when the `keylog` Cargo feature is not enabled.
///
/// All methods return no-op values — `SSLKEYLOGFILE` is never read and
/// no file I/O occurs.
#[cfg(not(feature = "keylog"))]
#[derive(Debug, Clone)]
pub struct KeyLog;

#[cfg(not(feature = "keylog"))]
impl KeyLog {
    /// Returns a no-op [`KeyLog`]. Does not read `SSLKEYLOGFILE`.
    pub fn from_env() -> KeyLog {
        KeyLog
    }

    /// Returns a no-op [`KeyLog`]. The path is ignored.
    pub fn from_file<P: AsRef<Path>>(_path: P) -> KeyLog {
        KeyLog
    }

    /// Returns a no-op [`Handle`].
    pub(crate) fn handle(self) -> Result<Handle> {
        Ok(Handle)
    }
}

/// No-op key log handle when the `keylog` Cargo feature is not enabled.
///
/// All writes are silently discarded.
#[cfg(not(feature = "keylog"))]
#[derive(Debug, Clone)]
pub(crate) struct Handle;

#[cfg(not(feature = "keylog"))]
impl Handle {
    /// No-op: discards the line.
    pub(crate) fn write(&self, _line: &str) {}
}

#[cfg(feature = "keylog")]
fn normalize_path<'a, P>(path: P) -> PathBuf
where
    P: Into<Cow<'a, Path>>,
{
    let path = path.into();
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}
