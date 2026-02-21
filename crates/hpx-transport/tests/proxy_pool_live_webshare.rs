use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use hpx::{Client, Proxy, ProxyPool, ProxyPoolStrategy};
use hpx_transport::{
    auth::NoAuth,
    exchange::{ExchangeClient, RestClient, RestConfig},
};
use serde_json::Value;

const IP_ECHO_URL: &str = "https://api.ipify.org/?format=json";
const FAILURE_URL: &str = "https://this-domain-should-not-exist-hpx-test.invalid/";
const DEFAULT_PROXY_LIST_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/webshare_proxy_list.txt"
);

#[derive(Clone, Debug)]
struct ProxyEntry {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl ProxyEntry {
    fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.split(':');
        let host = parts.next()?.trim().to_string();
        let port = parts.next()?.trim().parse().ok()?;
        let username = parts.next()?.trim().to_string();
        let password = parts.next()?.trim().to_string();

        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            host,
            port,
            username,
            password,
        })
    }

    fn to_proxy_with_password(&self, password: &str) -> hpx::Result<Proxy> {
        Proxy::all(format!(
            "http://{}:{}@{}:{}",
            self.username, password, self.host, self.port
        ))
    }

    fn good_proxy(&self) -> hpx::Result<Proxy> {
        self.to_proxy_with_password(&self.password)
    }

    fn bad_proxy(&self) -> hpx::Result<Proxy> {
        self.to_proxy_with_password(&format!("{}-invalid", self.password))
    }

    fn redacted(&self) -> String {
        format!("{}:{}:{}:***", self.host, self.port, self.username)
    }
}

fn proxy_list_path() -> PathBuf {
    env::var("HPX_PROXY_LIST_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_PROXY_LIST_PATH))
}

fn load_proxy_entries() -> (Vec<ProxyEntry>, PathBuf) {
    let path = proxy_list_path();
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read proxy list file {}: {}", path.display(), e));

    let entries = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            ProxyEntry::from_line(trimmed)
        })
        .collect();

    (entries, path)
}

async fn request_ip_with_hpx(client: &Client) -> Result<String, String> {
    let response = client
        .get(IP_ECHO_URL)
        .send()
        .await
        .map_err(|e| format!("request error: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("unexpected status: {}", response.status()));
    }

    let value: Value = response
        .json()
        .await
        .map_err(|e| format!("json decode error: {e}"))?;

    value
        .get("ip")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing ip field in response: {value}"))
}

async fn request_ip_with_rest(client: &RestClient<NoAuth>) -> Result<String, String> {
    let response = client
        .get::<Value>(IP_ECHO_URL)
        .await
        .map_err(|e| format!("request error: {e}"))?;

    response
        .data
        .get("ip")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing ip field in response: {}", response.data))
}

async fn request_failure_with_hpx(client: &Client) -> bool {
    match client.get(FAILURE_URL).send().await {
        Ok(response) => !response.status().is_success(),
        Err(_) => true,
    }
}

async fn request_failure_with_rest(client: &RestClient<NoAuth>) -> bool {
    client.get::<Value>(FAILURE_URL).await.is_err()
}

async fn find_first_working_entry(entries: &[ProxyEntry], proxy_path: &Path) -> ProxyEntry {
    for entry in entries {
        let proxy = entry.good_proxy().expect("proxy must parse");
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .proxy(proxy)
            .build()
            .expect("client should build");

        match request_ip_with_hpx(&client).await {
            Ok(ip) => {
                eprintln!("working proxy {} => {}", entry.redacted(), ip);
                return entry.clone();
            }
            Err(err) => {
                eprintln!("probe failed {} => {}", entry.redacted(), err);
            }
        }
    }

    panic!(
        "no working proxy found in {} entries from {}",
        entries.len(),
        proxy_path.display()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live test using real webshare proxies"]
async fn live_hpx_proxy_pool_strategies_with_webshare_list() {
    let (entries, proxy_path) = load_proxy_entries();
    assert!(!entries.is_empty(), "proxy list should not be empty");

    let working = find_first_working_entry(&entries, &proxy_path).await;
    let good_proxy = working.good_proxy().expect("good proxy must parse");
    let bad_proxy = working.bad_proxy().expect("bad proxy must parse");

    let bad_direct = Client::builder()
        .timeout(Duration::from_secs(20))
        .proxy(bad_proxy.clone())
        .build()
        .expect("client should build");
    let bad_should_fail = request_ip_with_hpx(&bad_direct).await;
    assert!(
        bad_should_fail.is_err(),
        "bad auth proxy unexpectedly succeeded for {}",
        working.redacted()
    );

    let sticky_pool = ProxyPool::with_strategy(
        vec![good_proxy.clone(), bad_proxy.clone()],
        ProxyPoolStrategy::StickyFailover,
    )
    .expect("sticky pool should build");

    let sticky_client = Client::builder()
        .timeout(Duration::from_secs(20))
        .proxy_pool(sticky_pool)
        .build()
        .expect("client should build");

    let first = request_ip_with_hpx(&sticky_client)
        .await
        .expect("first request via sticky pool should succeed");
    let second = request_ip_with_hpx(&sticky_client)
        .await
        .expect("second request via sticky pool should still succeed");
    assert_eq!(
        first, second,
        "sticky strategy should keep using same proxy after success"
    );

    assert!(
        request_failure_with_hpx(&sticky_client).await,
        "failure trigger request should fail"
    );

    let after_trigger = request_ip_with_hpx(&sticky_client).await;
    assert!(
        after_trigger.is_err(),
        "after trigger failure, next request should switch to bad proxy and fail"
    );

    let recovered = request_ip_with_hpx(&sticky_client)
        .await
        .expect("after bad proxy failure, pool should switch back to good proxy");
    let stable = request_ip_with_hpx(&sticky_client)
        .await
        .expect("success after recovery should remain on good proxy");
    assert_eq!(
        recovered, stable,
        "sticky strategy should stay on recovered good proxy after success"
    );

    let random_pool = ProxyPool::with_strategy(
        vec![good_proxy, bad_proxy],
        ProxyPoolStrategy::RandomPerRequest,
    )
    .expect("random pool should build");

    let random_client = Client::builder()
        .timeout(Duration::from_secs(20))
        .proxy_pool(random_pool)
        .build()
        .expect("client should build");

    let mut success_count = 0_u32;
    let mut failure_count = 0_u32;

    for _ in 0..24 {
        if request_ip_with_hpx(&random_client).await.is_ok() {
            success_count += 1;
        } else {
            failure_count += 1;
        }
    }

    assert!(
        success_count > 0,
        "random strategy should hit the good proxy at least once"
    );
    assert!(
        failure_count > 0,
        "random strategy should hit the bad proxy at least once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live test using real webshare proxies"]
async fn live_hpx_each_webshare_proxy_connectivity() {
    let (entries, _) = load_proxy_entries();
    assert!(!entries.is_empty(), "proxy list should not be empty");

    let mut failures = Vec::new();

    for entry in entries {
        let proxy = entry.good_proxy().expect("proxy must parse");
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .proxy(proxy)
            .build()
            .expect("client should build");

        if let Err(err) = request_ip_with_hpx(&client).await {
            failures.push(format!("{} => {}", entry.redacted(), err));
        }
    }

    assert!(
        failures.is_empty(),
        "some proxies failed connectivity: {}",
        failures.join(" | ")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live test using real webshare proxies"]
async fn live_rest_client_proxy_pool_strategies_with_webshare_list() {
    let (entries, proxy_path) = load_proxy_entries();
    assert!(!entries.is_empty(), "proxy list should not be empty");

    let working = find_first_working_entry(&entries, &proxy_path).await;
    let good_proxy = working.good_proxy().expect("good proxy must parse");
    let bad_proxy = working.bad_proxy().expect("bad proxy must parse");

    let sticky_pool = ProxyPool::with_strategy(
        vec![good_proxy.clone(), bad_proxy.clone()],
        ProxyPoolStrategy::StickyFailover,
    )
    .expect("sticky pool should build");

    let sticky_client = RestClient::new(
        RestConfig::new("https://api.ipify.org")
            .timeout(Duration::from_secs(20))
            .proxy_pool(sticky_pool),
        NoAuth,
    )
    .expect("rest client should build");

    let first = request_ip_with_rest(&sticky_client)
        .await
        .expect("first rest request should succeed");
    let second = request_ip_with_rest(&sticky_client)
        .await
        .expect("second rest request should still succeed");
    assert_eq!(
        first, second,
        "sticky strategy should keep using same proxy after success"
    );

    assert!(
        request_failure_with_rest(&sticky_client).await,
        "rest failure trigger should fail"
    );

    let after_trigger = request_ip_with_rest(&sticky_client).await;
    assert!(
        after_trigger.is_err(),
        "after trigger failure, next rest request should switch to bad proxy and fail"
    );

    let recovered = request_ip_with_rest(&sticky_client)
        .await
        .expect("after bad proxy failure, pool should switch back to good proxy");
    let stable = request_ip_with_rest(&sticky_client)
        .await
        .expect("success after recovery should remain on good proxy");
    assert_eq!(
        recovered, stable,
        "sticky strategy should stay on recovered good proxy after success"
    );

    let random_pool = ProxyPool::with_strategy(
        vec![good_proxy, bad_proxy],
        ProxyPoolStrategy::RandomPerRequest,
    )
    .expect("random pool should build");

    let random_client = RestClient::new(
        RestConfig::new("https://api.ipify.org")
            .timeout(Duration::from_secs(20))
            .proxy_pool(random_pool),
        NoAuth,
    )
    .expect("rest client should build");

    let mut success_count = 0_u32;
    let mut failure_count = 0_u32;

    for _ in 0..24 {
        if request_ip_with_rest(&random_client).await.is_ok() {
            success_count += 1;
        } else {
            failure_count += 1;
        }
    }

    assert!(
        success_count > 0,
        "random strategy should hit the good proxy at least once"
    );
    assert!(
        failure_count > 0,
        "random strategy should hit the bad proxy at least once"
    );
}
