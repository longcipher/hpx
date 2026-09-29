use system_configuration::{
    core_foundation::{
        array::CFArray,
        base::{CFType, TCFType},
        dictionary::CFDictionary,
        number::CFNumber,
        string::{CFString, CFStringRef},
    },
    dynamic_store::SCDynamicStoreBuilder,
    sys::schema_definitions::{
        kSCPropNetProxiesExceptionsList, kSCPropNetProxiesExcludeSimpleHostnames,
        kSCPropNetProxiesHTTPEnable, kSCPropNetProxiesHTTPPort, kSCPropNetProxiesHTTPProxy,
        kSCPropNetProxiesHTTPSEnable, kSCPropNetProxiesHTTPSPort, kSCPropNetProxiesHTTPSProxy,
    },
};

#[expect(unsafe_code)]
pub(super) fn with_system(builder: &mut super::matcher::Builder) {
    let Some(proxies_map) = SCDynamicStoreBuilder::new("")
        .build()
        .and_then(|store| store.get_proxies())
    else {
        return;
    };

    if builder.http.is_empty() {
        let http_proxy_config = parse_setting_from_dynamic_store(
            &proxies_map,
            // SAFETY: `kSCPropNetProxiesHTTP*` are immutable `CFStringRef` extern
            // statics provided by the system Configuration framework. They point
            // to statically-allocated, never-freed string constants and are safe
            // to read at any time.
            unsafe { kSCPropNetProxiesHTTPEnable },
            unsafe { kSCPropNetProxiesHTTPProxy },
            unsafe { kSCPropNetProxiesHTTPPort },
        );
        if let Some(http) = http_proxy_config {
            builder.http = http;
        }
    }

    if builder.https.is_empty() {
        let https_proxy_config = parse_setting_from_dynamic_store(
            &proxies_map,
            // SAFETY: same as above — `kSCPropNetProxiesHTTPS*` are immutable
            // system-provided `CFStringRef` extern statics, safe to read.
            unsafe { kSCPropNetProxiesHTTPSEnable },
            unsafe { kSCPropNetProxiesHTTPSProxy },
            unsafe { kSCPropNetProxiesHTTPSPort },
        );

        if let Some(https) = https_proxy_config {
            builder.https = https;
        }
    }

    // Import the system proxy bypass list. Without this, loopback and LAN
    // destinations listed in System Settings → Exceptions are still proxied.
    if builder.no.is_empty()
        && let Some(no_proxy) = parse_exceptions_from_dynamic_store(&proxies_map)
    {
        builder.no = no_proxy;
    }
}

/// Read `ExceptionsList` (and `ExcludeSimpleHostnames`) into a `NO_PROXY` string.
#[expect(unsafe_code)]
fn parse_exceptions_from_dynamic_store(
    proxies_map: &CFDictionary<CFString, CFType>,
) -> Option<String> {
    // SAFETY: `kSCPropNetProxies*` keys are immutable system CFString constants.
    let exclude_simple = proxies_map
        .find(unsafe { kSCPropNetProxiesExcludeSimpleHostnames })
        .and_then(|flag| flag.downcast::<CFNumber>())
        .and_then(|flag| flag.to_i32())
        .unwrap_or(0)
        == 1;

    // SAFETY: same — read-only system dictionary lookups.
    let mut parts: Vec<String> = proxies_map
        .find(unsafe { kSCPropNetProxiesExceptionsList })
        .and_then(|list| list.downcast::<CFArray>())
        .map_or_default(|arr| {
            arr.iter()
                .filter_map(|item| {
                    // Item is an untyped CF pointer; wrap as CFType then CFString.
                    let ty = unsafe { CFType::wrap_under_get_rule(*item) };
                    ty.downcast::<CFString>().map(|s| s.to_string())
                })
                .filter(|s| !s.is_empty())
                .collect()
        });

    if exclude_simple {
        // Approximate "exclude simple hostnames": always bypass loopback and
        // the macOS `<local>` pseudo-entry from the exceptions list.
        for name in ["localhost", "127.0.0.1", "::1", "<local>"] {
            if !parts.iter().any(|p| p.eq_ignore_ascii_case(name)) {
                parts.push(name.to_string());
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

fn parse_setting_from_dynamic_store(
    proxies_map: &CFDictionary<CFString, CFType>,
    enabled_key: CFStringRef,
    host_key: CFStringRef,
    port_key: CFStringRef,
) -> Option<String> {
    let proxy_enabled = proxies_map
        .find(enabled_key)
        .and_then(|flag| flag.downcast::<CFNumber>())
        .and_then(|flag| flag.to_i32())
        .unwrap_or(0)
        == 1;

    if proxy_enabled {
        let proxy_host = proxies_map
            .find(host_key)
            .and_then(|host| host.downcast::<CFString>())
            .map(|host| host.to_string());
        let proxy_port = proxies_map
            .find(port_key)
            .and_then(|port| port.downcast::<CFNumber>())
            .and_then(|port| port.to_i32());

        return match (proxy_host, proxy_port) {
            (Some(proxy_host), Some(proxy_port)) => Some(format!("{proxy_host}:{proxy_port}")),
            (Some(proxy_host), None) => Some(proxy_host),
            (None, Some(_)) => None,
            (None, None) => None,
        };
    }

    None
}
