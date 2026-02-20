//! Non-poisoning [`Mutex`] wrapper.
//!
//! This type exposes an API identical to [`std::sync::Mutex`],
//! but **does not return** [`std::sync::PoisonError`] even if a thread panics
//! while holding the lock.
//!
//! **Preferred for new code:** use `parking_lot::Mutex` (faster, no poisoning) or lock-free
//! structures such as `scc::HashMap` on hot paths. This wrapper remains for compatibility
//! and cases where matching the `std::sync` API is helpful.

use std::{
    ops::{Deref, DerefMut},
    sync,
};

/// A [`Mutex`] that never poisons and has the same interface as [`std::sync::Mutex`].
pub struct Mutex<T: ?Sized>(sync::Mutex<T>);

impl<T> Mutex<T> {
    /// Like [`std::sync::Mutex::new`].
    #[inline]
    pub fn new(t: T) -> Mutex<T> {
        Mutex(sync::Mutex::new(t))
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Like [`std::sync::Mutex::lock`].
    #[inline]
    pub fn lock(&self) -> MutexGuard<'_, T> {
        MutexGuard(self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl<T> Default for Mutex<T>
where
    T: Default,
{
    #[inline]
    fn default() -> Self {
        Mutex::new(T::default())
    }
}

/// Like [`std::sync::MutexGuard`].
#[must_use]
pub struct MutexGuard<'a, T: ?Sized + 'a>(sync::MutexGuard<'a, T>);

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.0.deref()
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.0.deref_mut()
    }
}
