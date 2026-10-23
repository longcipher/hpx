//! Page pool for concurrent browsing.
//!
//! Reusing pages skips V8 isolate creation and bootstrap JS execution.

use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use crate::{page::Page, stealth::StealthProfile};

/// A guard that returns the wrapped [`Page`] to its pool on drop.
pub struct PooledPage {
    page: Option<Page>,
    pool: Arc<PagePoolInner>,
}

impl PooledPage {
    /// Unwrap the page, detaching it from the pool (caller owns it).
    pub fn into_inner(mut self) -> Page {
        #[expect(clippy::expect_used, reason = "invariant")]
        let page = self.page.take().expect("pooled page already taken");
        // Detached: do not return to pool.
        self.page = None;
        page
    }

    /// Access the underlying page.
    pub fn page(&mut self) -> &mut Page {
        #[expect(clippy::expect_used, reason = "invariant")]
        let p = self.page.as_mut().expect("pooled page already taken");
        p
    }
}

impl std::ops::Deref for PooledPage {
    type Target = Page;
    fn deref(&self) -> &Self::Target {
        #[expect(clippy::expect_used, reason = "invariant: page present until drop")]
        let p = self.page.as_ref().expect("pooled page already taken");
        p
    }
}

impl std::ops::DerefMut for PooledPage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[expect(clippy::expect_used, reason = "invariant: page present until drop")]
        let p = self.page.as_mut().expect("pooled page already taken");
        p
    }
}

impl Drop for PooledPage {
    fn drop(&mut self) {
        if let Some(page) = self.page.take() {
            let mut pages = self.pool.idle_pages.lock();
            if pages.len() < self.pool.max_size {
                pages.push_back(page);
            }
        }
    }
}

struct PagePoolInner {
    idle_pages: Mutex<VecDeque<Page>>,
    max_size: usize,
}

/// A pool of warm Page instances.
pub struct PagePool {
    inner: Arc<PagePoolInner>,
}

impl PagePool {
    #[expect(clippy::arc_with_non_send_sync, reason = "single-threaded page pool")]
    pub fn new(max_size: usize) -> Self {
        Self {
            inner: Arc::new(PagePoolInner {
                idle_pages: Mutex::new(VecDeque::with_capacity(max_size)),
                max_size,
            }),
        }
    }

    /// Acquire a page from the pool or create a new one.
    pub async fn acquire(
        &self,
        profile: Option<StealthProfile>,
    ) -> Result<PooledPage, crate::page::PageError> {
        // Try to reuse a pooled page (brief sync lock).
        if let Some(mut page) = self.inner.idle_pages.lock().pop_front() {
            page.reload_html("<html><head></head><body></body></html>", "about:blank");
            return Ok(PooledPage {
                page: Some(page),
                pool: self.inner.clone(),
            });
        }
        // No pooled page available — create a new one (async, no lock held).
        let page =
            Page::from_html("<html><head></head><body></body></html>", profile.is_some()).await?;
        Ok(PooledPage {
            page: Some(page),
            pool: self.inner.clone(),
        })
    }

    /// Return a page to the pool.
    pub fn release(&self, page: Page) {
        let mut pages = self.inner.idle_pages.lock();
        if pages.len() < self.inner.max_size {
            pages.push_back(page);
        }
    }

    /// Acquire a warm Page and navigate it to `url`.
    pub async fn navigate(
        &self,
        url: &str,
        profile: StealthProfile,
    ) -> Result<PooledPage, crate::page::PageError> {
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
        drop(page);
        let page2 = pool.acquire(None).await;
        assert!(page2.is_ok());
    }

    #[tokio::test]
    async fn pool_respects_max_size() {
        let pool = PagePool::new(1);
        let p1 = pool.acquire(None).await.unwrap();
        let p2 = pool.acquire(None).await.unwrap();
        drop(p1);
        drop(p2);
        let count = pool.inner.idle_pages.lock().len();
        assert_eq!(count, 1);
    }
}
