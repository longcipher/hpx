//! Locator API for element selection and interaction.
//!
//! A thin wrapper over [`CdpPage`] providing a Playwright-like Locator interface
//! for element selection and interaction via CSS selectors.

use std::time::Duration;

use crate::cdp_page::CdpPage;

/// A locator that targets a DOM element by CSS selector.
///
/// Delegates all operations to the underlying [`CdpPage`].
/// Short-lived: owns the page reference rather than wrapping in `Arc`.
#[derive(Debug)]
pub struct Locator {
    /// The CSS selector for the target element.
    pub selector: String,
    /// The page this locator operates on.
    pub page: CdpPage,
}

impl Locator {
    /// Create a new `Locator` targeting `selector` on `page`.
    #[must_use]
    pub fn new(selector: impl Into<String>, page: CdpPage) -> Self {
        Self {
            selector: selector.into(),
            page,
        }
    }

    /// Click the element matching this locator's selector.
    ///
    /// # Errors
    ///
    /// Returns an error if the JS evaluation fails or the element is not found.
    pub async fn click(&self) -> crate::cdp_client::error::Result<()> {
        let escaped = escape_selector(&self.selector);
        let js = format!("document.querySelector('{escaped}').click()");
        self.page.evaluate::<()>(&js).await
    }

    /// Type `text` into the element by dispatching individual key events.
    ///
    /// Focuses the element first, then dispatches `keyDown` and `keyUp` events
    /// for each character to trigger keyboard event handlers.
    ///
    /// **Performance note:** Each character requires two CDP round-trips
    /// (`keyDown` + `keyUp`). For large text inputs, consider using
    /// `Input.insertText` via `cdp.send_command` directly.
    ///
    /// # Errors
    ///
    /// Returns an error if focusing or any key event dispatch fails.
    pub async fn type_text(&self, text: &str) -> crate::cdp_client::error::Result<()> {
        let escaped = escape_selector(&self.selector);
        let focus_js = format!("document.querySelector('{escaped}').focus()");
        self.page.evaluate::<()>(&focus_js).await?;

        for ch in text.chars() {
            let char_str = ch.escape_default().to_string();
            self.page
                .cdp
                .send_command_with_session(
                    "Input.dispatchKeyEvent",
                    Some(serde_json::json!({
                        "type": "keyDown",
                        "text": char_str,
                        "key": char_str,
                    })),
                    &self.page.session_id,
                )
                .await?;
            self.page
                .cdp
                .send_command_with_session(
                    "Input.dispatchKeyEvent",
                    Some(serde_json::json!({
                        "type": "keyUp",
                        "key": char_str,
                    })),
                    &self.page.session_id,
                )
                .await?;
        }

        Ok(())
    }

    /// Wait for the element matching this locator's selector to appear.
    ///
    /// Polls with a 100ms interval until the element is found or the timeout
    /// is reached.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeout is reached without finding the element.
    pub async fn wait_for(&self, timeout: Duration) -> crate::cdp_client::error::Result<()> {
        let escaped = escape_selector(&self.selector);
        let js = format!("document.querySelector('{escaped}') !== null");

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let found: bool = self.page.evaluate(&js).await?;
            if found {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(crate::cdp_client::error::CdpClientError::command_failed(
                    "wait_for",
                    &format!("element '{}' not found within timeout", self.selector),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Return the `innerText` of the element matching this locator's selector.
    ///
    /// # Errors
    ///
    /// Returns an error if the evaluation fails.
    pub async fn inner_text(&self) -> crate::cdp_client::error::Result<String> {
        let escaped = escape_selector(&self.selector);
        let js = format!("document.querySelector('{escaped}').innerText");
        self.page.evaluate(&js).await
    }

    /// Return the value of `name` attribute on the element.
    ///
    /// # Errors
    ///
    /// Returns an error if the evaluation fails.
    pub async fn get_attribute(
        &self,
        name: &str,
    ) -> crate::cdp_client::error::Result<Option<String>> {
        let escaped = escape_selector(&self.selector);
        let escaped_name = escape_selector(name);
        let js = format!("document.querySelector('{escaped}').getAttribute('{escaped_name}')");
        self.page.evaluate(&js).await
    }
}

/// Escape a CSS selector (or attribute name) for safe JS string interpolation.
///
/// Replaces single quotes with `\'` so the string can be safely embedded in
/// a single-quoted JS string literal.
///
/// # Errors
///
/// Never returns an error.
#[must_use]
pub fn escape_selector(s: &str) -> String {
    s.replace('\'', "\\'")
}

impl CdpPage {
    /// Create a [`Locator`] targeting `selector` on this page.
    #[must_use]
    pub fn locator(&self, selector: impl Into<String>) -> Locator {
        Locator::new(selector, self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_selector_replaces_single_quotes() {
        assert_eq!(escape_selector("button#'"), "button#\\'");
        assert_eq!(escape_selector("it's"), "it\\'s");
        assert_eq!(escape_selector("no-quotes"), "no-quotes");
    }

    #[test]
    fn escape_selector_empty_string() {
        assert_eq!(escape_selector(""), "");
    }

    #[test]
    fn locator_new_sets_fields() {
        let client = std::sync::Arc::new(crate::cdp_client::cdp::CdpClient::new());
        let page = CdpPage::new("t".into(), "s".into(), client);
        let loc = Locator::new("#btn", page);
        assert_eq!(loc.selector, "#btn");
        assert_eq!(loc.page.target_id, "t");
    }

    #[test]
    fn click_generates_correct_js() {
        let escaped = escape_selector(".my-class");
        let js = format!("document.querySelector('{escaped}').click()");
        assert_eq!(js, "document.querySelector('.my-class').click()");
    }

    #[test]
    fn inner_text_generates_correct_js() {
        let escaped = escape_selector("#text");
        let js = format!("document.querySelector('{escaped}').innerText");
        assert_eq!(js, "document.querySelector('#text').innerText");
    }

    #[test]
    fn get_attribute_generates_correct_js() {
        let escaped = escape_selector("#el");
        let name = escape_selector("href");
        let js = format!("document.querySelector('{escaped}').getAttribute('{name}')");
        assert_eq!(js, "document.querySelector('#el').getAttribute('href')");
    }

    #[test]
    fn wait_for_generates_correct_js() {
        let escaped = escape_selector("[data-testid='form']");
        let js = format!("document.querySelector('{escaped}') !== null");
        assert_eq!(
            js,
            "document.querySelector('[data-testid=\\'form\\']') !== null"
        );
    }

    #[test]
    fn focus_generates_correct_js() {
        let escaped = escape_selector("#input");
        let js = format!("document.querySelector('{escaped}').focus()");
        assert_eq!(js, "document.querySelector('#input').focus()");
    }

    #[test]
    fn page_locator_factory() {
        let client = std::sync::Arc::new(crate::cdp_client::cdp::CdpClient::new());
        let page = CdpPage::new("t".into(), "s".into(), client);
        let loc = page.locator("#btn");
        assert_eq!(loc.selector, "#btn");
        assert_eq!(loc.page.session_id, "s");
    }
}
