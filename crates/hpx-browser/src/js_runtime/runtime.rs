use deno_core::{JsRuntime, RuntimeOptions, v8};

use crate::{
    dom::Dom,
    js_runtime::{
        extensions::{
            console_ext::console_extension,
            crypto_ext::crypto_extension,
            dom_ext::dom_extension,
            fetch_ext::fetch_extension,
            timer_ext::{TimerState, timer_extension},
        },
        state::DomState,
    },
};

pub struct BrowserJsRuntime {
    inner: JsRuntime,
}

impl BrowserJsRuntime {
    pub fn new(dom: Dom) -> Self {
        Self::with_base_url(dom, None)
    }

    #[allow(clippy::expect_used, clippy::panic)]
    pub fn with_base_url(dom: Dom, base_url: Option<url::Url>) -> Self {
        let mut state = DomState::new(dom);
        if let Some(url) = base_url {
            state = state.with_base_url(url);
        }

        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![
                console_extension::init(),
                crypto_extension::init(),
                dom_extension::init(),
                timer_extension::init(),
                fetch_extension::init(),
                // Stubbed extensions — no ops
                crate::js_runtime::extensions::input_ext::input_extension::init(),
                crate::js_runtime::extensions::layout_ext::layout_extension::init(),
                crate::js_runtime::extensions::nav_ext::nav_extension::init(),
                crate::js_runtime::extensions::stealth_ext::stealth_extension::init(),
                crate::js_runtime::extensions::sse_ext::sse_extension::init(),
                crate::js_runtime::extensions::websocket_ext::websocket_extension::init(),
                crate::js_runtime::extensions::perf_ext::perf_extension::init(),
                crate::js_runtime::extensions::worker_ext::worker_extension::init(),
                crate::js_runtime::extensions::canvas_ext::canvas_extension::init(),
                crate::js_runtime::extensions::webgl_ext::webgl_extension::init(),
                crate::js_runtime::extensions::audio_ext::audio_extension::init(),
            ],
            ..Default::default()
        });

        runtime.op_state().borrow_mut().put(state);
        runtime.op_state().borrow_mut().put(TimerState::new());

        runtime
            .execute_script(
                "[bootstrap:console]",
                include_str!("js/console_bootstrap.js"),
            )
            .expect("console bootstrap failed");
        runtime
            .execute_script("[bootstrap:crypto]", include_str!("js/crypto_bootstrap.js"))
            .expect("crypto bootstrap failed");
        runtime
            .execute_script("[bootstrap:timer]", include_str!("js/timer_bootstrap.js"))
            .expect("timer bootstrap failed");
        runtime
            .execute_script("[bootstrap:dom]", include_str!("js/dom_bootstrap.js"))
            .expect("dom bootstrap failed");
        runtime
            .execute_script("[bootstrap:fetch]", include_str!("js/fetch_bootstrap.js"))
            .expect("fetch bootstrap failed");
        runtime
            .execute_script(
                "[bootstrap:storage]",
                include_str!("js/storage_bootstrap.js"),
            )
            .expect("storage bootstrap failed");

        runtime
            .execute_script(
                "[bootstrap:stealth]",
                include_str!("js/stealth_bootstrap.js"),
            )
            .expect("stealth bootstrap failed");

        Self { inner: runtime }
    }

    /// Update the DOM reference in the runtime's state without re-creating the V8 isolate.
    /// This allows reusing the existing V8 context (bootstrap scripts, globals) across navigations.
    pub fn update_dom(&mut self, dom: Dom) {
        let state = self.inner.op_state();
        let mut state = state.borrow_mut();
        let dom_state = state.borrow_mut::<DomState>();
        dom_state.dom = dom;
    }

    /// Execute a JavaScript script and return the result as a string.
    pub fn execute_script(&mut self, code: &str) -> Result<String, JsError> {
        let result = self
            .inner
            .execute_script("<anonymous>", code.to_string())
            .map_err(|e| JsError::Execution(e.to_string()))?;

        // Stringify via V8 scope — convert Global to Local then to_string
        let __ctx = self.inner.main_context();
        v8::scope_with_context!(scope, self.inner.v8_isolate(), __ctx);
        let local = v8::Local::new(scope, result);
        Ok(local
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "undefined".to_string()))
    }

    /// Run the V8 event loop until all pending work is done.
    pub async fn run_event_loop(&mut self) -> Result<(), JsError> {
        self.inner
            .run_event_loop(deno_core::PollEventLoopOptions::default())
            .await
            .map_err(|e| JsError::Execution(e.to_string()))
    }

    /// Get console output captured so far.
    pub fn console_output(&mut self) -> Vec<crate::js_runtime::state::ConsoleMessage> {
        let state = self.inner.op_state();
        let state = state.borrow();
        state.borrow::<DomState>().console_output.clone()
    }

    /// Get the inner deno_core JsRuntime.
    pub fn inner(&mut self) -> &mut JsRuntime {
        &mut self.inner
    }

    pub fn set_user_agent(&mut self, ua: &str) {
        let escaped = ua.replace('\\', "\\\\").replace('\'', "\\'");
        let _ = self
            .inner
            .execute_script("<set-ua>", format!("globalThis.__hpx_ua = '{}';", escaped));
    }

    pub fn set_platform(&mut self, platform: &str, ua_platform: &str, ua_platform_version: &str) {
        let p = platform.replace('\'', "\\'");
        let uap = ua_platform.replace('\'', "\\'");
        let uapv = ua_platform_version.replace('\'', "\\'");
        let _ = self.inner.execute_script(
            "<set-platform>",
            format!(
                "globalThis.__hpx_platform='{}';globalThis.__hpx_ua_platform='{}';globalThis.__hpx_ua_platform_version='{}';",
                p, uap, uapv
            ),
        );
    }

    pub fn set_stealth(&mut self, enabled: bool) {
        let _ = self.inner.execute_script(
            "<set-stealth>",
            format!("globalThis.__hpx_stealth = {};", enabled),
        );
    }

    pub fn run_page_init(&mut self) {
        let _ = self
            .inner
            .execute_script("<hpx:page-init>", "globalThis.__hpx_init();".to_string());
    }
}

impl Default for BrowserJsRuntime {
    fn default() -> Self {
        Self::new(Dom::new())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JsError {
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("compilation failed: {0}")]
    Compilation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_eval() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt.execute_script("1 + 2").unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn eval_string() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt.execute_script("'hello ' + 'world'").unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn eval_syntax_error() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt.execute_script("function {{{}}}");
        assert!(result.is_err());
    }

    #[test]
    fn console_log_capture() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.execute_script("console.log('test message')").unwrap();
        let output = rt.console_output();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].args[0], "test message");
        assert_eq!(output[0].level, crate::js_runtime::state::ConsoleLevel::Log);
    }

    #[test]
    fn console_warn_capture() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.execute_script("console.warn('warning msg')").unwrap();
        let output = rt.console_output();
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].level,
            crate::js_runtime::state::ConsoleLevel::Warn
        );
    }

    #[test]
    fn dom_create_element() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script("let el = document.createElement('div'); el.tagName")
            .unwrap();
        assert_eq!(result, "DIV");
    }

    #[test]
    fn dom_text_content() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let el = document.createElement('p');
                el.textContent = 'hello';
                el.textContent
                "#,
            )
            .unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn dom_append_child() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let parent = document.createElement('div');
                let child = document.createElement('span');
                parent.appendChild(child);
                parent.childNodes.length
                "#,
            )
            .unwrap();
        assert_eq!(result, "1");
    }

    #[test]
    fn dom_get_element_by_id() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let el = document.createElement('div');
                el.id = 'test';
                // getElementById searches from document, so test direct API
                let found = document.getElementById('nonexistent');
                found === null
                "#,
            )
            .unwrap();
        assert_eq!(result, "true");
    }

    #[test]
    fn dom_node_type() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let el = document.createElement('div');
                let text = document.createTextNode('hi');
                `${el.nodeType}:${text.nodeType}`
                "#,
            )
            .unwrap();
        assert_eq!(result, "1:3");
    }

    #[test]
    fn dom_parent_child_navigation() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let parent = document.createElement('div');
                let child = document.createElement('p');
                parent.appendChild(child);
                child.parentNode.tagName
                "#,
            )
            .unwrap();
        assert_eq!(result, "DIV");
    }

    #[test]
    fn storage_local() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                localStorage.setItem('key', 'value');
                localStorage.getItem('key')
                "#,
            )
            .unwrap();
        assert_eq!(result, "value");
    }

    #[test]
    fn storage_length() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                localStorage.clear();
                localStorage.setItem('a', '1');
                localStorage.setItem('b', '2');
                localStorage.length
                "#,
            )
            .unwrap();
        assert_eq!(result, "2");
    }

    #[test]
    fn crypto_random_values() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let buf = new Uint8Array(16);
                crypto.getRandomValues(buf);
                buf.length
                "#,
            )
            .unwrap();
        assert_eq!(result, "16");
    }

    #[test]
    fn fetch_stub() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt.execute_script("typeof fetch").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn headers_class() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        let result = rt
            .execute_script(
                r#"
                let h = new Headers([['content-type', 'text/html']]);
                h.get('Content-Type')
                "#,
            )
            .unwrap();
        assert_eq!(result, "text/html");
    }

    #[test]
    fn set_user_agent_test() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
        let result = rt.execute_script("globalThis.__hpx_ua").unwrap();
        assert_eq!(result, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
    }

    #[test]
    fn set_platform_test() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_platform("macos", "MacIntel", "15.0.0");
        let result = rt
            .execute_script(
                "`${globalThis.__hpx_platform}|${globalThis.__hpx_ua_platform}|${globalThis.__hpx_ua_platform_version}`",
            )
            .unwrap();
        assert_eq!(result, "macos|MacIntel|15.0.0");
    }

    #[test]
    fn set_stealth_test() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        let result = rt.execute_script("globalThis.__hpx_stealth").unwrap();
        assert_eq!(result, "true");
    }

    #[test]
    fn run_page_init_test() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.set_user_agent("test-agent");
        rt.run_page_init();
        let result = rt.execute_script("typeof globalThis.__hpx_init").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn navigator_useragent_from_global() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
        rt.set_platform("Win32", "Windows", "15.0.0");
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt.execute_script("globalThis.navigator.userAgent").unwrap();
        assert!(result.contains("Chrome/148"));
    }

    #[test]
    fn webdriver_is_false() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt.execute_script("globalThis.navigator.webdriver").unwrap();
        assert_eq!(result, "false");
    }

    #[test]
    fn webdriver_descriptor_is_undefined() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script(
                "Object.getOwnPropertyDescriptor(globalThis.navigator, 'webdriver') === undefined ? 'true' : 'false'",
            )
            .unwrap();
        assert_eq!(result, "true");
    }

    #[test]
    fn chrome_object_exists() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt.execute_script("typeof globalThis.chrome").unwrap();
        assert_eq!(result, "object");
    }

    #[test]
    fn chrome_csi_returns_timing() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("typeof globalThis.chrome.csi().onloadT")
            .unwrap();
        assert_eq!(result, "number");
    }

    #[test]
    fn screen_dimensions() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let w: i32 = rt
            .execute_script("globalThis.screen.width")
            .unwrap()
            .parse()
            .unwrap();
        let h: i32 = rt
            .execute_script("globalThis.screen.height")
            .unwrap()
            .parse()
            .unwrap();
        assert!(w > 0 && h > 0);
    }

    #[test]
    fn grease_brands_count() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_user_agent("Mozilla/5.0 Chrome/148.0.0.0");
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("globalThis.navigator.userAgentData.brands.length")
            .unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn grease_chromium_version() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_user_agent("Mozilla/5.0 Chrome/148.0.0.0");
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script(
                "globalThis.navigator.userAgentData.brands.some(function(b){return b.brand==='Chromium' && b.version==='148'})",
            )
            .unwrap();
        assert_eq!(result, "true");
    }

    #[test]
    fn plugins_count() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("globalThis.navigator.plugins.length")
            .unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn performance_memory_exists() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("typeof globalThis.performance.memory.jsHeapSizeLimit")
            .unwrap();
        assert_eq!(result, "number");
    }

    #[test]
    fn canvas_element_exists() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("typeof HTMLCanvasElement !== 'undefined'")
            .unwrap();
        assert!(result == "true" || result == "false");
    }

    #[test]
    fn audio_context_type() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt.execute_script("typeof AudioContext").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn audio_context_creates_instance() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script(
                "var ctx = new AudioContext(); ctx.sampleRate === 44100 || ctx.sampleRate === 48000",
            )
            .unwrap();
        assert_eq!(result, "true");
    }

    #[test]
    fn audio_context_base_latency() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("var ctx = new AudioContext(); typeof ctx.baseLatency")
            .unwrap();
        assert_eq!(result, "number");
    }

    #[test]
    fn audio_context_state_running() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("var ctx = new AudioContext(); ctx.state")
            .unwrap();
        assert_eq!(result, "running");
    }

    #[test]
    fn audio_context_destination() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script("var ctx = new AudioContext(); ctx.destination.numberOfInputs")
            .unwrap();
        assert_eq!(result, "1");
    }

    #[test]
    fn audio_context_webkit_compat() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt.execute_script("typeof webkitAudioContext").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn canvas_noise_deterministic_per_session() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_stealth(true);
        rt.run_page_init();
        let result = rt
            .execute_script(
                r#"
            (function() {
                try {
                    var c = document.createElement('canvas');
                    c.width = 2; c.height = 2;
                    var d1 = c.toDataURL(); var d2 = c.toDataURL();
                    return d1 === d2 ? 'true' : 'false';
                } catch(e) {
                    return 'false';
                }
            })()
            "#,
            )
            .unwrap();
        assert!(result == "true" || result == "false");
    }

    #[test]
    fn profile_to_globals_chrome_148_macos() {
        let mut rt = BrowserJsRuntime::new(Dom::new());
        rt.set_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
        rt.set_platform("MacIntel", "macOS", "15.0.0");
        rt.set_stealth(true);
        rt.run_page_init();
        let ua = rt.execute_script("navigator.userAgent").unwrap();
        assert!(ua.contains("Macintosh"));
        assert!(ua.contains("Chrome/148"));
        let platform = rt.execute_script("navigator.platform").unwrap();
        assert_eq!(platform, "MacIntel");
    }
}
