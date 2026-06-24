//! Iframe support for hpx-browser.
//!
//! Each iframe with `srcdoc` gets its own DOM tree, V8 runtime, and event loop.
//! Communication between parent and child is via serialized postMessage.

use crate::dom::{Dom, NodeData, NodeId};

/// Info about an iframe found in the DOM.
pub struct IframeInfo {
    pub node_id: NodeId,
    pub srcdoc: Option<String>,
    pub src: Option<String>,
}

/// A child iframe with its own DOM.
#[cfg(feature = "v8")]
pub struct ChildIframe {
    pub node_id: NodeId,
    pub event_loop: crate::event_loop::BrowserEventLoop,
}

#[cfg(feature = "v8")]
impl ChildIframe {
    /// Create a child iframe from srcdoc HTML.
    pub async fn from_srcdoc(
        node_id: NodeId,
        html: &str,
        _profile: &crate::stealth::StealthProfile,
    ) -> Result<Self, crate::event_loop::EventLoopError> {
        let dom = crate::html_parser::parse_html(html);
        let runtime = crate::js_runtime::BrowserJsRuntime::new(dom);
        let mut event_loop = crate::event_loop::BrowserEventLoop::with_runtime(runtime);

        // Run child event loop
        let _ = event_loop
            .run_until_idle(std::time::Duration::from_secs(5))
            .await;

        Ok(Self {
            node_id,
            event_loop,
        })
    }

    /// Create a child iframe by fetching src URL via HTTP client.
    pub async fn from_url(
        node_id: NodeId,
        url: &str,
        client: &crate::net::HttpClient,
        stealth_profile: Option<&crate::stealth::StealthProfile>,
    ) -> Result<Self, crate::event_loop::EventLoopError> {
        let resp = client.get(url).await.map_err(|e| {
            crate::event_loop::EventLoopError::Other(format!("iframe fetch error: {}", e))
        })?;

        if !resp.ok() {
            return Err(crate::event_loop::EventLoopError::Other(format!(
                "iframe fetch {} returned {}",
                url, resp.status
            )));
        }

        let html = resp.text();
        if html.trim().is_empty() {
            return Self::from_srcdoc(
                node_id,
                "<html><body></body></html>",
                stealth_profile.unwrap_or(&crate::stealth::StealthProfile::default()),
            )
            .await;
        }

        let dom = crate::html_parser::parse_html(&html);
        let runtime = crate::js_runtime::BrowserJsRuntime::new(dom);
        let mut event_loop = crate::event_loop::BrowserEventLoop::with_runtime(runtime);

        // Set location
        let url_js = url.replace('\\', "\\\\").replace('\'', "\\'");
        let _ = event_loop.execute_script(&format!("location.href = '{}';", url_js));

        // Run child event loop
        let _ = event_loop
            .run_until_idle(std::time::Duration::from_secs(10))
            .await;

        Ok(Self {
            node_id,
            event_loop,
        })
    }

    /// Evaluate JS in the child's V8 context.
    pub fn evaluate(&mut self, js: &str) -> Result<String, crate::event_loop::EventLoopError> {
        self.event_loop.execute_script(js)
    }

    /// Query the child's DOM for text content of a selector match.
    pub fn query_text(&mut self, selector: &str) -> Option<String> {
        self.evaluate(&format!(
            r#"(() => {{ const el = document.querySelector("{}"); return el ? el.textContent : ""; }})()"#,
            selector.replace('"', "\\\"")
        ))
        .ok()
        .filter(|s| !s.is_empty())
    }
}

/// Find all `<iframe>` elements in the DOM.
pub fn find_iframes(dom: &Dom) -> Vec<IframeInfo> {
    let mut iframes = Vec::new();
    collect_iframes(dom, NodeId::DOCUMENT, &mut iframes);
    iframes
}

fn collect_iframes(dom: &Dom, node_id: NodeId, iframes: &mut Vec<IframeInfo>) {
    let children = dom.children(node_id);
    for child_id in children {
        if let Some(node) = dom.get(child_id) {
            if let NodeData::Element(elem) = &node.data {
                if elem.name.local.eq_ignore_ascii_case("iframe") {
                    let srcdoc = elem
                        .attrs
                        .iter()
                        .find(|a| a.name.local == "srcdoc")
                        .map(|a| a.value.clone());
                    let src = elem
                        .attrs
                        .iter()
                        .find(|a| a.name.local == "src")
                        .map(|a| a.value.clone());
                    iframes.push(IframeInfo {
                        node_id: child_id,
                        srcdoc,
                        src,
                    });
                }
            }
            collect_iframes(dom, child_id, iframes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_no_iframes_in_empty_doc() {
        let dom = Dom::new();
        let iframes = find_iframes(&dom);
        assert!(iframes.is_empty());
    }

    #[test]
    fn find_iframe_with_srcdoc() {
        let dom = crate::html_parser::parse_html(
            r#"<html><body><iframe srcdoc="<p>Hello</p>"></iframe></body></html>"#,
        );
        let iframes = find_iframes(&dom);
        assert_eq!(iframes.len(), 1);
        assert!(iframes[0].srcdoc.is_some());
        assert!(iframes[0].srcdoc.as_ref().unwrap().contains("Hello"));
    }

    #[test]
    fn find_iframe_with_src() {
        let dom = crate::html_parser::parse_html(
            r#"<html><body><iframe src="https://example.com"></iframe></body></html>"#,
        );
        let iframes = find_iframes(&dom);
        assert_eq!(iframes.len(), 1);
        assert_eq!(iframes[0].src.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn find_multiple_iframes() {
        let dom = crate::html_parser::parse_html(
            r#"<html><body>
                <iframe src="first.html"></iframe>
                <iframe src="second.html"></iframe>
            </body></html>"#,
        );
        let iframes = find_iframes(&dom);
        assert_eq!(iframes.len(), 2);
    }
}
