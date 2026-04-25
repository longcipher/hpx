//! BDD test harness for hpx-dl using cucumber-rs.
//!
//! Run with: `cargo test -p hpx-dl --test cucumber`

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use cucumber::{World, given, then, when};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct MockFile {
    data: Vec<u8>,
    supports_range: bool,
    etag: Option<String>,
    last_modified: Option<String>,
    chunk_size: usize,
    chunk_delay: Duration,
}

type FileStore = Arc<RwLock<HashMap<String, MockFile>>>;

#[derive(Debug)]
struct MockResponse {
    status: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    chunk_size: usize,
    chunk_delay: Duration,
}

#[derive(Debug, Default)]
struct EventLog {
    events: Vec<hpx_dl::DownloadEvent>,
    failures: HashMap<hpx_dl::DownloadId, String>,
    completed_order: Vec<hpx_dl::DownloadId>,
}

fn request_path(target: &str) -> String {
    let path = if let Some(rest) = target.strip_prefix("http://") {
        match rest.find('/') {
            Some(index) => &rest[index..],
            None => "/",
        }
    } else {
        target
    };

    if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn status_text(status: hpx::StatusCode) -> &'static str {
    status.canonical_reason().unwrap_or("OK")
}

async fn build_mock_response(store: FileStore, request: &str) -> MockResponse {
    let lines: Vec<&str> = request.split("\r\n").collect();
    let parts: Vec<&str> = lines
        .first()
        .map(|line| line.split_whitespace().collect())
        .unwrap_or_default();
    if parts.len() < 2 {
        return MockResponse {
            status: "400 Bad Request".to_string(),
            headers: vec![("Content-Length".to_string(), "0".to_string())],
            body: Vec::new(),
            chunk_size: 0,
            chunk_delay: Duration::ZERO,
        };
    }

    let method = parts[0];
    let path = request_path(parts[1]);

    let mut range_start = None;
    let mut range_end = None;
    for line in &lines[1..] {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            let value = value.trim();
            if let Some(value) = value.strip_prefix("bytes=") {
                let mut bounds = value.splitn(2, '-');
                range_start = bounds.next().and_then(|start| start.parse::<u64>().ok());
                range_end = bounds.next().and_then(|end| {
                    if end.is_empty() {
                        None
                    } else {
                        end.parse::<u64>().ok()
                    }
                });
            }
        }
    }

    let files = store.read().await;
    let Some(file) = files.get(&path) else {
        return MockResponse {
            status: "404 Not Found".to_string(),
            headers: vec![("Content-Length".to_string(), "0".to_string())],
            body: Vec::new(),
            chunk_size: 0,
            chunk_delay: Duration::ZERO,
        };
    };

    let total = file.data.len() as u64;
    let mut headers = vec![
        (
            "Accept-Ranges".to_string(),
            if file.supports_range { "bytes" } else { "none" }.to_string(),
        ),
        ("Content-Length".to_string(), total.to_string()),
    ];
    if let Some(etag) = &file.etag {
        headers.push(("ETag".to_string(), etag.clone()));
    }
    if let Some(last_modified) = &file.last_modified {
        headers.push(("Last-Modified".to_string(), last_modified.clone()));
    }

    if method.eq_ignore_ascii_case("HEAD") {
        return MockResponse {
            status: "200 OK".to_string(),
            headers,
            body: Vec::new(),
            chunk_size: 0,
            chunk_delay: Duration::ZERO,
        };
    }

    match range_start {
        Some(start) if file.supports_range && start < total => {
            let end = range_end.unwrap_or(total - 1).min(total - 1);
            let body = file.data[start as usize..=end as usize].to_vec();
            headers.retain(|(name, _)| name != "Content-Length");
            headers.push(("Content-Length".to_string(), body.len().to_string()));
            headers.push((
                "Content-Range".to_string(),
                format!("bytes {start}-{end}/{total}"),
            ));
            MockResponse {
                status: "206 Partial Content".to_string(),
                headers,
                body,
                chunk_size: file.chunk_size,
                chunk_delay: file.chunk_delay,
            }
        }
        _ => MockResponse {
            status: "200 OK".to_string(),
            headers,
            body: file.data.clone(),
            chunk_size: file.chunk_size,
            chunk_delay: file.chunk_delay,
        },
    }
}

async fn handle_mock_connection(stream: tokio::net::TcpStream, store: FileStore) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = stream;
    let mut buf = vec![0u8; 8192];
    let n = match stream.read(&mut buf).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };

    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    let response = build_mock_response(store, &request).await;

    let mut header = format!("HTTP/1.1 {}\r\n", response.status);
    for (name, value) in &response.headers {
        header.push_str(&format!("{name}: {value}\r\n"));
    }
    header.push_str("\r\n");

    let _ = stream.write_all(header.as_bytes()).await;
    if !response.body.is_empty() {
        let chunk_size = response.chunk_size.max(response.body.len()).max(1);
        for chunk in response.body.chunks(chunk_size) {
            let _ = stream.write_all(chunk).await;
            if response.chunk_delay > Duration::ZERO {
                tokio::time::sleep(response.chunk_delay).await;
            }
        }
    }
    let _ = stream.flush().await;
}

async fn start_mock_server(port: u16) -> (SocketAddr, FileStore) {
    let store: FileStore = Arc::new(RwLock::new(HashMap::new()));
    let store_clone = Arc::clone(&store);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let store = Arc::clone(&store_clone);
            tokio::spawn(async move {
                handle_mock_connection(stream, store).await;
            });
        }
    });

    (addr, store)
}

async fn proxy_response(request: &str) -> (String, Vec<u8>) {
    let lines: Vec<&str> = request.split("\r\n").collect();
    let parts: Vec<&str> = lines
        .first()
        .map(|line| line.split_whitespace().collect())
        .unwrap_or_default();
    if parts.len() < 2 {
        return (
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n".to_string(),
            Vec::new(),
        );
    }

    let method = parts[0];
    let target = parts[1];
    let client = hpx::Client::new();
    let mut outbound = match method {
        "HEAD" => client.head(target),
        _ => client.get(target),
    };

    for line in &lines[1..] {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "connection" | "proxy-connection"
        ) {
            continue;
        }
        outbound = outbound.header(name.trim(), value.trim());
    }

    let response = match outbound.send().await {
        Ok(response) => response,
        Err(_) => {
            return (
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n".to_string(),
                Vec::new(),
            );
        }
    };

    let status = response.status();
    let headers = response.headers().clone();
    let body = if method.eq_ignore_ascii_case("HEAD") {
        Vec::new()
    } else {
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default()
    };

    let mut header = format!("HTTP/1.1 {} {}\r\n", status.as_u16(), status_text(status));
    let mut has_length = false;
    for (name, value) in &headers {
        if name == hpx::header::TRANSFER_ENCODING {
            continue;
        }
        if name == hpx::header::CONTENT_LENGTH {
            has_length = true;
        }
        if let Ok(value) = value.to_str() {
            header.push_str(&format!("{}: {}\r\n", name.as_str(), value));
        }
    }
    if !has_length {
        header.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    header.push_str("\r\n");

    (header, body)
}

async fn handle_proxy_connection(stream: tokio::net::TcpStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = stream;
    let mut buf = vec![0u8; 8192];
    let n = match stream.read(&mut buf).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    let (header, body) = proxy_response(&request).await;
    let _ = stream.write_all(header.as_bytes()).await;
    if !body.is_empty() {
        let _ = stream.write_all(&body).await;
    }
    let _ = stream.flush().await;
}

async fn start_proxy_server(port: u16) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind proxy server");
    let addr = listener.local_addr().expect("proxy addr");

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                handle_proxy_connection(stream).await;
            });
        }
    });

    addr
}

/// Shared state for end-to-end Cucumber scenarios.
#[derive(Debug, cucumber::World)]
pub struct DownloadWorld {
    engine: Option<hpx_dl::DownloadEngine>,
    mock_addr: SocketAddr,
    mock_store: FileStore,
    event_log: Arc<tokio::sync::Mutex<EventLog>>,
    storage_path: PathBuf,
    last_download_id: Option<hpx_dl::DownloadId>,
    downloads_by_name: HashMap<String, hpx_dl::DownloadId>,
    last_error: Option<String>,
    originals: HashMap<String, Vec<u8>>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    proxy_url: Option<String>,
    default_proxy: Option<hpx_dl::ProxyConfig>,
    temp_dir: tempfile::TempDir,
}

impl Default for DownloadWorld {
    fn default() -> Self {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let (mock_addr, mock_store) = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(start_mock_server(0))
        });
        let storage_path = temp_dir.path().join("hpx-dl-state.json");

        Self {
            engine: None,
            mock_addr,
            mock_store,
            event_log: Arc::new(tokio::sync::Mutex::new(EventLog::default())),
            storage_path,
            last_download_id: None,
            downloads_by_name: HashMap::new(),
            last_error: None,
            originals: HashMap::new(),
            started_at: None,
            finished_at: None,
            proxy_url: None,
            default_proxy: None,
            temp_dir,
        }
    }
}

impl DownloadWorld {
    fn mock_url(&self, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("http://{}{}", self.mock_addr, path)
    }

    fn rewrite_url(&self, url: &str) -> String {
        self.mock_url(&request_path(url))
    }

    fn resolve_storage_path(&self, raw: &str) -> PathBuf {
        let path = Path::new(raw);
        if let Some(name) = path.file_name() {
            self.temp_dir.path().join(name)
        } else {
            self.storage_path.clone()
        }
    }

    fn destination_path(&self, dest: &str) -> PathBuf {
        let path = PathBuf::from(dest);
        if path.is_absolute() {
            path
        } else {
            self.temp_dir.path().join(path)
        }
    }

    async fn register_file_at(&mut self, path: &str, data: Vec<u8>, supports_range: bool) {
        let key = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let chunk_delay = if data.len() >= 512 * 1024 {
            Duration::from_millis(8)
        } else {
            Duration::ZERO
        };
        let chunk_size = 64 * 1024;

        self.mock_store.write().await.insert(
            key,
            MockFile {
                data,
                supports_range,
                etag: None,
                last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".to_string()),
                chunk_size,
                chunk_delay,
            },
        );
    }

    async fn register_file_for_url(
        &mut self,
        filename: &str,
        url: &str,
        data: Vec<u8>,
        supports_range: bool,
    ) {
        let path = request_path(url);
        self.originals.insert(filename.to_string(), data.clone());
        self.register_file_at(&path, data, supports_range).await;
    }

    fn set_engine(&mut self, engine: hpx_dl::DownloadEngine) {
        self.event_log = Arc::new(tokio::sync::Mutex::new(EventLog::default()));
        let mut rx = engine.subscribe();
        let log = Arc::clone(&self.event_log);
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                let mut log = log.lock().await;
                if let hpx_dl::DownloadEvent::Failed { id, error } = &event {
                    log.failures.insert(*id, error.clone());
                }
                if let hpx_dl::DownloadEvent::Completed { id } = event {
                    log.completed_order.push(id);
                    log.events.push(hpx_dl::DownloadEvent::Completed { id });
                    continue;
                }
                log.events.push(event);
            }
        });
        self.engine = Some(engine);
    }

    fn build_engine(
        &mut self,
        configure: impl FnOnce(hpx_dl::EngineBuilder) -> hpx_dl::EngineBuilder,
    ) {
        let builder = hpx_dl::DownloadEngine::builder().storage_path(&self.storage_path);
        let engine = configure(builder).build().expect("build download engine");
        self.set_engine(engine);
    }

    fn parse_priority(priority: &str) -> hpx_dl::DownloadPriority {
        match priority.to_ascii_lowercase().as_str() {
            "low" => hpx_dl::DownloadPriority::Low,
            "high" => hpx_dl::DownloadPriority::High,
            "critical" => hpx_dl::DownloadPriority::Critical,
            _ => hpx_dl::DownloadPriority::Normal,
        }
    }

    async fn wait_for_download_state(&mut self, id: hpx_dl::DownloadId) -> hpx_dl::DownloadState {
        let start = Instant::now();
        loop {
            let status = self
                .engine
                .as_ref()
                .expect("engine configured")
                .status(id)
                .expect("download status");
            if matches!(
                status.state,
                hpx_dl::DownloadState::Completed
                    | hpx_dl::DownloadState::Failed
                    | hpx_dl::DownloadState::Paused
            ) {
                self.finished_at = Some(Instant::now());
                if status.state == hpx_dl::DownloadState::Failed {
                    self.last_error = self.failure_for(id).await;
                }
                return status.state;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "download timed out"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn failure_for(&self, id: hpx_dl::DownloadId) -> Option<String> {
        self.event_log.lock().await.failures.get(&id).cloned()
    }
}

#[given(expr = "an hpx-dl engine is configured with max {int} connections per download")]
fn engine_with_max_connections(world: &mut DownloadWorld, max_conn: usize) {
    world.build_engine(|builder| builder.max_connections(max_conn));
}

#[given("an hpx-dl engine is configured")]
fn engine_default(world: &mut DownloadWorld) {
    world.build_engine(|builder| builder);
}

#[given(regex = r"^an hpx-dl engine is configured with global speed limit (\d+)KB/s$")]
fn engine_with_speed_limit(world: &mut DownloadWorld, limit_kb: u64) {
    world.build_engine(|builder| builder.speed_limit(Some(limit_kb * 1024)));
}

#[given(regex = r#"^an hpx-dl engine is configured with proxy "([^"]*)"$"#)]
fn engine_with_proxy(world: &mut DownloadWorld, _proxy_url: String) {
    if let Some(proxy_url) = &world.proxy_url {
        world.default_proxy = Some(hpx_dl::ProxyConfig {
            url: proxy_url.clone(),
            kind: hpx_dl::ProxyKind::Http,
        });
    }
    world.build_engine(|builder| builder);
}

#[given(regex = r#"^an hpx-dl engine is configured with SQLite storage at "([^"]*)"$"#)]
fn engine_with_sqlite(world: &mut DownloadWorld, db_path: String) {
    world.storage_path = world.resolve_storage_path(&db_path);
    let storage_path = world.storage_path.clone();
    world.build_engine(|builder| builder.storage_path(storage_path));
}

#[given(expr = "an hpx-dl engine is configured with max {int} concurrent download")]
fn engine_with_max_concurrent(world: &mut DownloadWorld, max_concurrent: usize) {
    world.build_engine(|builder| builder.max_concurrent(max_concurrent));
}

#[given(regex = r#"^a file "([^"]*)" of (\d+)MB is available at "([^"]*)"$"#)]
async fn file_of_size_mb(world: &mut DownloadWorld, filename: String, size_mb: u64, url: String) {
    let size = (size_mb * 1024 * 1024) as usize;
    let data = vec![0x42u8; size];
    world
        .register_file_for_url(&filename, &url, data, true)
        .await;
}

#[given(regex = r#"^a file "([^"]*)" of (\d+)KB is available at "([^"]*)"$"#)]
async fn file_of_size_kb(world: &mut DownloadWorld, filename: String, size_kb: u64, url: String) {
    let size = (size_kb * 1024) as usize;
    let data = vec![0x42u8; size];
    world
        .register_file_for_url(&filename, &url, data, true)
        .await;
}

#[given(regex = r#"^a file "([^"]*)" is available at "([^"]*)"$"#)]
async fn file_available(world: &mut DownloadWorld, filename: String, url: String) {
    let data = vec![0xABu8; 1024];
    world
        .register_file_for_url(&filename, &url, data, true)
        .await;
}

#[given(regex = r#"^a file "([^"]*)" with SHA-256 hash "([^"]*)" is available at "([^"]*)"$"#)]
async fn file_with_hash(world: &mut DownloadWorld, filename: String, hash: String, url: String) {
    let data = if hash
        .eq_ignore_ascii_case("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    {
        Vec::new()
    } else {
        vec![0u8; 1024]
    };
    world
        .register_file_for_url(&filename, &url, data, true)
        .await;
}

#[given("the server supports range requests")]
fn server_supports_range(_world: &mut DownloadWorld) {}

#[given("the server does not support range requests")]
async fn server_no_range_support(world: &mut DownloadWorld) {
    let mut files = world.mock_store.write().await;
    for file in files.values_mut() {
        file.supports_range = false;
    }
}

#[given(regex = r#"^the server returns ETag "([^"]*)" for "([^"]*)"$"#)]
async fn server_returns_etag(world: &mut DownloadWorld, etag: String, url: String) {
    let path = request_path(&url);
    if let Some(file) = world.mock_store.write().await.get_mut(&path) {
        file.etag = Some(etag);
    }
}

#[given(regex = r#"^a file "([^"]*)" of (\d+)MB is available at both mirrors$"#)]
async fn file_at_mirrors(world: &mut DownloadWorld, filename: String, size_mb: u64) {
    let size = (size_mb * 1024 * 1024) as usize;
    let data = vec![0x33u8; size];
    world.originals.insert(filename.clone(), data.clone());
    world
        .register_file_at(&format!("/mirror1/{filename}"), data.clone(), true)
        .await;
    world
        .register_file_at(&format!("/mirror2/{filename}"), data, true)
        .await;
}

#[given(regex = r#"^an HTTP proxy is running at "([^"]*)"$"#)]
async fn http_proxy_running(world: &mut DownloadWorld, _proxy_url: String) {
    let addr = start_proxy_server(0).await;
    world.proxy_url = Some(format!("http://{addr}"));
}

#[given(regex = r#"^a Metalink v4 file is available at "([^"]*)" containing:$"#)]
async fn metalink_available(world: &mut DownloadWorld, url: String) {
    let data = format!(
        "<?xml version=\"1.0\"?>\n\
         <metalink xmlns=\"urn:ietf:params:xml:ns:metalink\">\n\
           <file name=\"image.iso\">\n\
             <url priority=\"1\">{}</url>\n\
             <url priority=\"2\">{}</url>\n\
           </file>\n\
         </metalink>",
        world.mock_url("/mirror1/image.iso"),
        world.mock_url("/mirror2/image.iso"),
    );
    world
        .register_file_for_url("file.metalink", &url, data.into_bytes(), false)
        .await;
}

#[when(regex = r#"^I add a download for "([^"]*)" to "([^"]*)"$"#)]
async fn add_download(world: &mut DownloadWorld, url: String, dest: String) {
    if let Some(engine) = &world.engine {
        let rewritten = world.rewrite_url(&url);
        let destination = world.destination_path(&dest);
        let mut builder = hpx_dl::DownloadRequest::builder(&rewritten, &destination);
        if let Some(proxy) = world.default_proxy.clone() {
            builder = builder.proxy(proxy);
        }
        let request = builder.build();
        match engine.add(request) {
            Ok(id) => {
                world.last_download_id = Some(id);
                world.downloads_by_name.insert(dest.clone(), id);
                world.last_error = None;
                world.started_at.get_or_insert_with(Instant::now);
            }
            Err(error) => world.last_error = Some(error.to_string()),
        }
    }
}

#[when(regex = r#"^I add a download for "([^"]*)" to "([^"]*)" with priority "([^"]*)"$"#)]
async fn add_download_with_priority(
    world: &mut DownloadWorld,
    url: String,
    dest: String,
    priority: String,
) {
    if let Some(engine) = &world.engine {
        let rewritten = world.rewrite_url(&url);
        let destination = world.destination_path(&dest);
        let mut builder = hpx_dl::DownloadRequest::builder(&rewritten, &destination)
            .priority(DownloadWorld::parse_priority(&priority));
        if let Some(proxy) = world.default_proxy.clone() {
            builder = builder.proxy(proxy);
        }
        match engine.add(builder.build()) {
            Ok(id) => {
                world.last_download_id = Some(id);
                world.downloads_by_name.insert(dest.clone(), id);
                world.started_at.get_or_insert_with(Instant::now);
            }
            Err(error) => world.last_error = Some(error.to_string()),
        }
    }
}

#[when(regex = r#"^I add a download for "([^"]*)" to "([^"]*)" with SHA-256 checksum "([^"]*)"$"#)]
async fn add_download_with_checksum(
    world: &mut DownloadWorld,
    url: String,
    dest: String,
    expected_hash: String,
) {
    if let Some(engine) = &world.engine {
        let rewritten = world.rewrite_url(&url);
        let destination = world.destination_path(&dest);
        let mut builder = hpx_dl::DownloadRequest::builder(&rewritten, &destination).checksum(
            hpx_dl::ChecksumSpec {
                algorithm: hpx_dl::HashAlgorithm::Sha256,
                expected: expected_hash,
            },
        );
        if let Some(proxy) = world.default_proxy.clone() {
            builder = builder.proxy(proxy);
        }
        match engine.add(builder.build()) {
            Ok(id) => {
                world.last_download_id = Some(id);
                world.downloads_by_name.insert(dest.clone(), id);
                world.started_at.get_or_insert_with(Instant::now);
            }
            Err(error) => world.last_error = Some(error.to_string()),
        }
    }
}

#[when(expr = "I add a Metalink download from {string}")]
async fn add_metalink_download(world: &mut DownloadWorld, url: String) {
    if let Some(engine) = &world.engine {
        let path = request_path(&url);
        let manifest = world
            .mock_store
            .read()
            .await
            .get(&path)
            .map(|file| file.data.clone())
            .expect("metalink manifest registered");
        let files = hpx_dl::parse_metalink(&manifest).expect("parse metalink");
        let request = files
            .into_iter()
            .next()
            .expect("one metalink file")
            .into_download_requests(world.destination_path("image.iso"))
            .into_iter()
            .next()
            .expect("one request");

        match engine.add(request) {
            Ok(id) => {
                world.last_download_id = Some(id);
                world.downloads_by_name.insert("image.iso".to_string(), id);
                world.started_at.get_or_insert_with(Instant::now);
            }
            Err(error) => world.last_error = Some(error.to_string()),
        }
    }
}

#[when("I subscribe to download events")]
fn subscribe_events(_world: &mut DownloadWorld) {}

#[when("I wait for all downloads to complete")]
async fn wait_all_complete(world: &mut DownloadWorld) {
    let start = Instant::now();
    loop {
        let statuses = world
            .engine
            .as_ref()
            .expect("engine configured")
            .list()
            .expect("download list");
        if !statuses.is_empty()
            && statuses.iter().all(|status| {
                matches!(
                    status.state,
                    hpx_dl::DownloadState::Completed | hpx_dl::DownloadState::Failed
                )
            })
        {
            world.finished_at = Some(Instant::now());
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "downloads timed out"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[when("I wait for the download to complete")]
async fn wait_download_complete(world: &mut DownloadWorld) {
    let id = world.last_download_id.expect("download id");
    let _ = world.wait_for_download_state(id).await;
}

#[when(regex = r"^the download reaches (\d+)% progress$")]
async fn download_reaches_progress(world: &mut DownloadWorld, percent: u64) {
    let id = world.last_download_id.expect("download id");
    let start = Instant::now();
    loop {
        let status = world
            .engine
            .as_ref()
            .expect("engine configured")
            .status(id)
            .expect("download status");
        let reached = status
            .total_bytes
            .map(|total| {
                total > 0 && status.bytes_downloaded.saturating_mul(100) / total >= percent
            })
            .unwrap_or(false);
        if reached || status.state == hpx_dl::DownloadState::Completed {
            if reached && let Some(engine) = &world.engine {
                let _ = engine.pause(id);
            }
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "progress threshold timed out"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[when("I stop the engine")]
fn stop_engine(world: &mut DownloadWorld) {
    world.engine = None;
}

#[when(regex = r#"^I create a new engine with the same storage path "([^"]*)"$"#)]
fn create_engine_same_path(world: &mut DownloadWorld, path: String) {
    world.storage_path = world.resolve_storage_path(&path);
    let storage_path = world.storage_path.clone();
    world.build_engine(|builder| builder.storage_path(storage_path));
}

#[when("I create a new engine with the same storage path")]
fn create_engine_same_path_default(world: &mut DownloadWorld) {
    let storage_path = world.storage_path.clone();
    world.build_engine(|builder| builder.storage_path(storage_path));
}

#[when("I resume the download")]
fn resume_download(world: &mut DownloadWorld) {
    if let (Some(engine), Some(id)) = (&world.engine, world.last_download_id) {
        if let Err(error) = engine.resume(id) {
            world.last_error = Some(error.to_string());
        }
    }
}

#[when("I simulate an unclean shutdown by dropping the engine without cleanup")]
fn simulate_crash(world: &mut DownloadWorld) {
    world.engine = None;
}

#[then(expr = "the download state should be {string}")]
fn check_state(world: &mut DownloadWorld, expected_state: String) {
    let id = world.last_download_id.expect("download id");
    let status = world
        .engine
        .as_ref()
        .expect("engine configured")
        .status(id)
        .expect("download status");
    assert_eq!(
        status.state.to_string(),
        expected_state.to_ascii_lowercase(),
        "expected state {expected_state}, got {}",
        status.state
    );
}

#[then(regex = r#"^the "([^"]*)" download should have completed before the "([^"]*)" download$"#)]
async fn check_completion_order(world: &mut DownloadWorld, first: String, second: String) {
    let first_id = *world
        .downloads_by_name
        .get(&first)
        .expect("first download id present");
    let second_id = *world
        .downloads_by_name
        .get(&second)
        .expect("second download id present");
    let start = Instant::now();
    let (first_pos, second_pos) = loop {
        let completed_order = world.event_log.lock().await.completed_order.clone();
        if let (Some(first_pos), Some(second_pos)) = (
            completed_order.iter().position(|id| *id == first_id),
            completed_order.iter().position(|id| *id == second_id),
        ) {
            break (first_pos, second_pos);
        }

        assert!(
            start.elapsed() < Duration::from_secs(5),
            "expected both downloads to emit completion events"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert!(
        first_pos < second_pos,
        "expected {first} to finish before {second}"
    );
}

#[then(regex = r#"^the file "([^"]*)" should be exactly (\d+) bytes$"#)]
fn check_file_size(world: &mut DownloadWorld, filename: String, expected_size: u64) {
    let path = world.destination_path(&filename);
    let metadata = std::fs::metadata(&path).expect("file metadata");
    assert_eq!(
        metadata.len(),
        expected_size,
        "wrong file size for {filename}"
    );
}

#[then(regex = r#"^the file "([^"]*)" should match the original content$"#)]
fn check_file_content(world: &mut DownloadWorld, filename: String) {
    let expected = world.originals.get(&filename).expect("expected content");
    let actual = std::fs::read(world.destination_path(&filename)).expect("read downloaded file");
    assert_eq!(
        &actual, expected,
        "downloaded file content mismatch for {filename}"
    );
}

#[then(regex = r#"^the download error should contain "([^"]*)"$"#)]
async fn check_error_contains(world: &mut DownloadWorld, expected_msg: String) {
    let error = match (&world.last_error, world.last_download_id) {
        (Some(error), _) => Some(error.clone()),
        (None, Some(id)) => world.failure_for(id).await,
        (None, None) => None,
    }
    .expect("expected an error message");

    assert!(
        error.contains(&expected_msg),
        "expected error to contain '{expected_msg}', got '{error}'"
    );
}

#[then(regex = r#"^I should have received an? "([^"]*)" event$"#)]
async fn check_event_received(world: &mut DownloadWorld, event_type: String) {
    let events = &world.event_log.lock().await.events;
    let found = events
        .iter()
        .any(|event| matches_event_type(event, &event_type));
    assert!(found, "missing event type {event_type}");
}

#[then(regex = r#"^I should have received at least (\d+) "([^"]*)" event$"#)]
async fn check_min_events(world: &mut DownloadWorld, min_count: u64, event_type: String) {
    let count = world
        .event_log
        .lock()
        .await
        .events
        .iter()
        .filter(|event| matches_event_type(event, &event_type))
        .count() as u64;
    assert!(
        count >= min_count,
        "expected at least {min_count} {event_type} events, found {count}"
    );
}

#[then(regex = r"^the elapsed time should be at least (\d+) seconds$")]
fn check_elapsed_time(world: &mut DownloadWorld, min_seconds: u64) {
    let started = world.started_at.expect("download start time");
    let finished = world.finished_at.expect("download finish time");
    assert!(
        finished.duration_since(started) >= Duration::from_secs(min_seconds),
        "elapsed time {:?} was shorter than {}s",
        finished.duration_since(started),
        min_seconds
    );
}

#[then(regex = r"^the average speed should not exceed (\d+)KB/s$")]
fn check_average_speed(world: &mut DownloadWorld, max_speed_kb: u64) {
    let started = world.started_at.expect("download start time");
    let finished = world.finished_at.expect("download finish time");
    let elapsed = finished.duration_since(started).as_secs_f64();
    assert!(elapsed > 0.0, "elapsed time must be positive");

    let id = world.last_download_id.expect("download id");
    let status = world
        .engine
        .as_ref()
        .expect("engine configured")
        .status(id)
        .expect("download status");
    let total = status.total_bytes.unwrap_or_else(|| {
        std::fs::metadata(world.destination_path("throttled.bin"))
            .expect("download metadata")
            .len()
    });
    let average_kb = (total as f64 / 1024.0) / elapsed;
    assert!(
        average_kb <= max_speed_kb as f64,
        "average speed {average_kb:.2}KB/s exceeded {max_speed_kb}KB/s"
    );
}

#[then(expr = "the download should be in state {string}")]
fn check_download_state(world: &mut DownloadWorld, expected_state: String) {
    check_state(world, expected_state);
}

fn matches_event_type(event: &hpx_dl::DownloadEvent, event_type: &str) -> bool {
    let actual = match event {
        hpx_dl::DownloadEvent::Added { .. } => "Added",
        hpx_dl::DownloadEvent::Started { .. } => "Started",
        hpx_dl::DownloadEvent::Progress { .. } => "Progress",
        hpx_dl::DownloadEvent::StateChanged { .. } => "StateChanged",
        hpx_dl::DownloadEvent::Completed { .. } => "Completed",
        hpx_dl::DownloadEvent::Failed { .. } => "Failed",
        hpx_dl::DownloadEvent::Removed { .. } => "Removed",
    };
    actual == event_type
}

fn main() {
    if std::env::args().any(|arg| arg == "--list") {
        return;
    }

    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    runtime.block_on(async {
        DownloadWorld::run("features/").await;
    });
}
