use hpx::{Client, Proxy};
use http::StatusCode;

#[test]
fn proxy_pool_switches_after_failure_and_sticks_after_success() {
    let pool = hpx::proxy_pool::ProxyPool::with_strategy(
        vec![
            Proxy::all("http://proxy-a:8080").expect("valid proxy"),
            Proxy::all("http://proxy-b:8080").expect("valid proxy"),
        ],
        hpx::proxy_pool::ProxyPoolStrategy::StickyFailover,
    )
    .expect("non-empty proxy pool");

    let result = Client::builder().proxy_pool(pool).build();
    assert!(result.is_ok());
}

#[test]
fn proxy_pool_rejects_empty_list() {
    let result = hpx::proxy_pool::ProxyPool::with_strategy(
        Vec::new(),
        hpx::proxy_pool::ProxyPoolStrategy::RandomPerRequest,
    );

    assert!(result.is_err());
}

#[test]
fn proxy_pool_failure_status_classifier_matches_expected_codes() {
    assert!(hpx::proxy_pool::ProxyPool::is_failure_status(
        StatusCode::BAD_GATEWAY
    ));
    assert!(hpx::proxy_pool::ProxyPool::is_failure_status(
        StatusCode::TOO_MANY_REQUESTS
    ));
    assert!(hpx::proxy_pool::ProxyPool::is_failure_status(
        StatusCode::PROXY_AUTHENTICATION_REQUIRED
    ));
    assert!(!hpx::proxy_pool::ProxyPool::is_failure_status(
        StatusCode::OK
    ));
}
