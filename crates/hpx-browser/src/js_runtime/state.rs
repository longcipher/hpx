use std::{collections::HashMap, sync::Arc};

use crate::{
    dom::Dom,
    net::{HttpClient, NetConfig, SharedNetConfig, csp::PolicySet},
};

/// Shared state stored in deno_core's OpState, accessible by all ops.
pub struct DomState {
    pub dom: Dom,
    pub base_url: Option<url::Url>,
    pub console_output: Vec<ConsoleMessage>,
    /// localStorage ("local") / sessionStorage ("session")
    pub storage: HashMap<String, HashMap<String, String>>,
}

/// Network state for JS-initiated requests (`fetch`).
///
/// Holds the page's shared [`SharedNetConfig`] so `op_fetch` routes through
/// the same proxy (and SSRF policy) as navigations and subresource loads. The
/// handle is cloned out of OpState before any `.await`, so no borrow is held
/// across yield points.
///
/// The [`HttpClient`] is cached so cookies, Accept-CH, and H1-only host memory
/// persist across multiple `fetch()` calls within the same page. It is
/// rebuilt only when the shared [`NetConfig`] is swapped (proxy change).
pub struct FetchState {
    pub net_config: SharedNetConfig,
    pub client: HttpClient,
    pub(crate) last_config: Arc<NetConfig>,
    /// Content-Security-Policy governing JS-initiated requests
    /// (`connect-src`). Empty when the page has no policy or when CSP
    /// enforcement is disabled.
    pub(crate) csp: PolicySet,
    /// Origin of the document that issues the requests, used as the base for
    /// `'self'` comparisons.
    pub(crate) page_origin: Option<url::Url>,
}

impl FetchState {
    /// Create a new [`FetchState`] with an initial client built from the
    /// current config.
    pub fn new(net_config: SharedNetConfig, client: HttpClient) -> Self {
        let last_config = (&*net_config.load()).clone();
        Self {
            net_config,
            client,
            last_config,
            csp: PolicySet::default(),
            page_origin: None,
        }
    }

    /// Apply the document's Content-Security-Policy to JS-initiated requests.
    pub fn set_csp(&mut self, csp: PolicySet, page_origin: Option<url::Url>) {
        self.csp = csp;
        self.page_origin = page_origin;
    }
}

#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    pub level: ConsoleLevel,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    Log,
    Warn,
    Error,
    Info,
    Debug,
}

impl DomState {
    pub fn new(dom: Dom) -> Self {
        let mut storage = HashMap::new();
        storage.insert("local".to_string(), HashMap::new());
        storage.insert("session".to_string(), HashMap::new());
        Self {
            dom,
            base_url: None,
            console_output: Vec::new(),
            storage,
        }
    }

    pub fn with_base_url(mut self, url: url::Url) -> Self {
        self.base_url = Some(url);
        self
    }
}
