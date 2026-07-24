//! The `config` module provides a generic mechanism for loading and managing
//! request-scoped configuration.
//!
//! # Design Overview
//!
//! This module is centered around two abstractions:
//!
//! - The [`RequestConfigValue`] trait, used to associate a config key type with its value type.
//! - The [`RequestConfig`] struct, which wraps an optional value of the type linked via
//!   [`RequestConfigValue`].
//!
//! Under the hood, the [`RequestConfig`] struct holds a single value for the associated config
//! type. This value can be conveniently accessed, inserted, or mutated using [`http::Extensions`],
//! enabling type-safe configuration storage and retrieval on a per-request basis.
//!
//! # Motivation
//!
//! The key design benefit is the ability to store multiple config types—potentially even with the
//! same value type (e.g., [`std::time::Duration`])—without code duplication or ambiguity. By
//! leveraging trait association, each config key is distinct at the type level, while code for
//! storage and access remains totally generic.
//!
//! # Usage
//!
//! Implement [`RequestConfigValue`] for any marker type you wish to use as a config key,
//! specifying the associated value type. Then use [`RequestConfig<T>`] in [`Extensions`]
//! to set or retrieve config values for each key type in a uniform way.

use http::Extensions;

/// Associate a marker key type with its associated value type stored in [`http::Extensions`].
/// Implement this trait for unit/marker types to declare the concrete `Value` used for that key.
pub(crate) trait RequestConfigValue: Clone + 'static {
    type Value: Clone + Send + Sync + 'static;
}

/// Typed wrapper that holds an optional configuration value for a given marker key `T`.
/// Instances of [`RequestConfig<T>`] are intended to be inserted into [`http::Extensions`].
#[derive(Clone, Copy)]
pub(crate) struct RequestConfig<T: RequestConfigValue>(Option<T::Value>);

impl<T: RequestConfigValue> Default for RequestConfig<T> {
    #[inline]
    fn default() -> Self {
        RequestConfig(None)
    }
}

impl<T> RequestConfig<T>
where
    T: RequestConfigValue,
{
    /// Creates a new `RequestConfig` with the provided value.
    #[inline]
    pub(crate) const fn new(v: Option<T::Value>) -> Self {
        RequestConfig(v)
    }

    /// Returns a reference to the inner value of this request-scoped configuration.
    #[inline]
    pub(crate) const fn as_ref(&self) -> Option<&T::Value> {
        self.0.as_ref()
    }

    /// Retrieve the value from the request-scoped configuration.
    ///
    /// If the request specifies a value, use that value; otherwise, attempt to retrieve it from the
    /// current instance (typically a client instance).
    #[inline]
    pub(crate) fn fetch<'a>(&'a self, ext: &'a Extensions) -> Option<&'a T::Value> {
        ext.get::<RequestConfig<T>>()
            .and_then(Self::as_ref)
            .or(self.as_ref())
    }

    /// Stores this value into the given [`http::Extensions`], if a value of the same type is not
    /// already present.
    ///
    /// This method checks whether the provided [`http::Extensions`] contains a
    /// [`RequestConfig<T>`]. If not, it clones the current value and inserts it into the
    /// extensions. If a value already exists, the method does nothing.
    #[inline]
    pub(crate) fn store<'a>(&'a self, ext: &'a mut Extensions) -> &'a mut Option<T::Value> {
        &mut ext.get_or_insert_with(|| self.clone()).0
    }

    /// Loads the internal value from the provided [`http::Extensions`], if present.
    ///
    /// This method attempts to remove a value of type [`RequestConfig<T>`] from the provided
    /// [`http::Extensions`]. If such a value exists, the current internal value is replaced with
    /// the removed value. If not, the internal value remains unchanged.
    #[inline]
    pub(crate) fn load(&mut self, ext: &mut Extensions) -> Option<&T::Value> {
        if let Some(value) = RequestConfig::<T>::remove(ext) {
            self.0.replace(value);
        }
        self.as_ref()
    }

    /// Returns an immutable reference to the stored value from the given [`http::Extensions`], if
    /// present.
    ///
    /// Internally fetches [`RequestConfig<T>`] and returns a reference to its inner value, if set.
    #[inline]
    pub(crate) fn get(ext: &Extensions) -> Option<&T::Value> {
        ext.get::<RequestConfig<T>>()?.0.as_ref()
    }

    /// Returns a mutable reference to the inner value in [`http::Extensions`], inserting a default
    /// if missing.
    ///
    /// This ensures a [`RequestConfig<T>`] exists and returns a mutable reference to its inner
    /// `Option<T::Value>`.
    #[inline]
    pub(crate) fn get_mut(ext: &mut Extensions) -> &mut Option<T::Value> {
        &mut ext.get_or_insert_default::<RequestConfig<T>>().0
    }

    /// Removes and returns the stored value from the given [`http::Extensions`], if present.
    ///
    /// This consumes the [`RequestConfig<T>`] entry and extracts its inner value.
    #[inline]
    pub(crate) fn remove(ext: &mut Extensions) -> Option<T::Value> {
        ext.remove::<RequestConfig<T>>()?.0
    }
}

/// Implements [`RequestConfigValue`] for a given type.
#[allow(unused_macro_rules)]
macro_rules! impl_request_config_value {
    ($type:ty) => {
        impl crate::config::RequestConfigValue for $type {
            type Value = Self;
        }
    };
    ($type:ty, $value:ty) => {
        impl crate::config::RequestConfigValue for $type {
            type Value = $value;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // Define test marker types
    #[derive(Clone, Debug, PartialEq)]
    struct TimeoutConfig(std::time::Duration);

    impl RequestConfigValue for TimeoutConfig {
        type Value = std::time::Duration;
    }

    #[derive(Clone, Debug, PartialEq)]
    struct MaxRetries;

    impl RequestConfigValue for MaxRetries {
        type Value = usize;
    }

    #[test]
    fn request_config_default_is_none() {
        let config = RequestConfig::<TimeoutConfig>::default();
        assert!(config.as_ref().is_none());
    }

    #[test]
    fn request_config_new_with_value() {
        let config = RequestConfig::<TimeoutConfig>::new(Some(std::time::Duration::from_secs(5)));
        assert!(config.as_ref().is_some());
        assert_eq!(config.as_ref().unwrap(), &std::time::Duration::from_secs(5));
    }

    #[test]
    fn request_config_new_with_none() {
        let config = RequestConfig::<TimeoutConfig>::new(None);
        assert!(config.as_ref().is_none());
    }

    #[test]
    fn fetch_returns_client_value_when_request_empty() {
        let client = RequestConfig::<TimeoutConfig>::new(Some(std::time::Duration::from_secs(10)));
        let ext = Extensions::new();
        let result = client.fetch(&ext);
        assert_eq!(result, Some(&std::time::Duration::from_secs(10)));
    }

    #[test]
    fn fetch_returns_request_value_over_client() {
        let client = RequestConfig::<TimeoutConfig>::new(Some(std::time::Duration::from_secs(10)));
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<TimeoutConfig>::new(Some(
            std::time::Duration::from_secs(5),
        )));
        let result = client.fetch(&ext);
        assert_eq!(result, Some(&std::time::Duration::from_secs(5)));
    }

    #[test]
    fn fetch_returns_none_when_both_empty() {
        let client = RequestConfig::<TimeoutConfig>::new(None);
        let ext = Extensions::new();
        assert!(client.fetch(&ext).is_none());
    }

    #[test]
    fn store_inserts_when_not_present() {
        let config = RequestConfig::<MaxRetries>::new(Some(3));
        let mut ext = Extensions::new();
        config.store(&mut ext);
        assert_eq!(RequestConfig::<MaxRetries>::get(&ext), Some(&3));
    }

    #[test]
    fn store_does_not_overwrite_existing() {
        let config = RequestConfig::<MaxRetries>::new(Some(3));
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<MaxRetries>::new(Some(5)));
        config.store(&mut ext);
        // The existing value (5) should remain
        assert_eq!(RequestConfig::<MaxRetries>::get(&ext), Some(&5));
    }

    #[test]
    fn get_returns_value_when_present() {
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<MaxRetries>::new(Some(7)));
        assert_eq!(RequestConfig::<MaxRetries>::get(&ext), Some(&7));
    }

    #[test]
    fn get_returns_none_when_absent() {
        let ext = Extensions::new();
        assert!(RequestConfig::<MaxRetries>::get(&ext).is_none());
    }

    #[test]
    fn get_returns_none_when_inner_value_is_none() {
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<MaxRetries>::new(None));
        assert!(RequestConfig::<MaxRetries>::get(&ext).is_none());
    }

    #[test]
    fn remove_extracts_value() {
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<MaxRetries>::new(Some(42)));
        let removed = RequestConfig::<MaxRetries>::remove(&mut ext);
        assert_eq!(removed, Some(42));
        assert!(RequestConfig::<MaxRetries>::get(&ext).is_none());
    }

    #[test]
    fn remove_returns_none_when_absent() {
        let mut ext = Extensions::new();
        assert!(RequestConfig::<MaxRetries>::remove(&mut ext).is_none());
    }

    #[test]
    fn load_replaces_inner_value() {
        let mut config = RequestConfig::<MaxRetries>::new(Some(1));
        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<MaxRetries>::new(Some(99)));
        let result = config.load(&mut ext);
        assert_eq!(result, Some(&99));
        assert_eq!(config.as_ref(), Some(&99));
    }

    #[test]
    fn load_keeps_existing_when_ext_empty() {
        let mut config = RequestConfig::<MaxRetries>::new(Some(5));
        let mut ext = Extensions::new();
        let result = config.load(&mut ext);
        assert_eq!(result, Some(&5));
    }

    #[test]
    fn get_mut_provides_mutable_access() {
        let mut ext = Extensions::new();
        *RequestConfig::<MaxRetries>::get_mut(&mut ext) = Some(10);
        assert_eq!(RequestConfig::<MaxRetries>::get(&ext), Some(&10));
    }

    #[test]
    fn type_safety_different_configs_dont_interfere() {
        #[derive(Clone, Debug, PartialEq)]
        struct TagA;
        #[derive(Clone, Debug, PartialEq)]
        struct TagB;
        impl RequestConfigValue for TagA {
            type Value = String;
        }
        impl RequestConfigValue for TagB {
            type Value = String;
        }

        let mut ext = Extensions::new();
        ext.insert(RequestConfig::<TagA>::new(Some("hello".into())));
        // TagB should not see TagA's value
        assert!(RequestConfig::<TagB>::get(&ext).is_none());
        assert_eq!(RequestConfig::<TagA>::get(&ext), Some(&"hello".to_string()));
    }
}
