//! Page pool for concurrent browsing.
//!
//! Reusing pages skips V8 isolate creation and bootstrap JS execution.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{page::Page, stealth::StealthProfile};

/// A pool of warm Page instances.
pub struct PagePool {
    idle_pages: Arc<Mutex<VecDeque<Page>>>,
    max_size: usize,
}

impl PagePool {
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded page pool; Arc shares idle queue within one thread"
    )]
    pub fn new(max_size: usize) -> Self {
        Self {
            idle_pages: Arc::new(Mutex::new(VecDeque::with_capacity(max_size))),
            max_size,
        }
    }

    /// Acquire a page from the pool or create a new one.
    #[allow(
        clippy::await_holding_lock,
        reason = "std Mutex guard dropped before any await; single-threaded executor"
    )]
    pub async fn acquire(
        &self,
        profile: Option<StealthProfile>,
    ) -> Result<Page, crate::page::PageError> {
        let mut pages = self.idle_pages.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut page) = pages.pop_front() {
            page.reload_html("<html><head></head><body></body></html>", "about:blank");
            return Ok(page);
        }
        Page::from_html("<html><head></head><body></body></html>", profile.is_some()).await
    }

    /// Return a page to the pool.
    pub fn release(&self, page: Page) {
        let mut pages = self.idle_pages.lock().unwrap_or_else(|e| e.into_inner());
        if pages.len() < self.max_size {
            pages.push_back(page);
        }
    }

    /// Acquire a warm Page and navigate it to `url`.
    pub async fn navigate(
        &self,
        url: &str,
        profile: StealthProfile,
    ) -> Result<Page, crate::page::PageError> {
        let mut page = self.acquire(Some(profile)).await?;
        page.navigate_warm(url).await?;
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_acquire_creates_page() {
        let pool = PagePool::new(4);
        let page = pool.acquire(None).await;
        assert!(page.is_ok());
    }

    #[tokio::test]
    async fn pool_release_and_reacquire() {
        let pool = PagePool::new(4);
        let page = pool.acquire(None).await.unwrap();
        pool.release(page);
        let page2 = pool.acquire(None).await;
        assert!(page2.is_ok());
    }

    #[tokio::test]
    async fn pool_respects_max_size() {
        let pool = PagePool::new(1);
        let p1 = pool.acquire(None).await.unwrap();
        let p2 = pool.acquire(None).await.unwrap();
        pool.release(p1);
        pool.release(p2); // second release should be dropped (pool full)
        let count = pool.idle_pages.lock().unwrap().len();
        assert_eq!(count, 1);
    }
}
