#![allow(clippy::too_many_arguments)]

use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use eyre::Context;
use hpx_browser::{dom::NodeId, host::EngineHandle, page::Page};
use tokio::sync::Semaphore;

use crate::cli::DumpFormat;

pub(crate) async fn handle_fetch(
    url: String,
    dump: DumpFormat,
    selector: Option<String>,
    wait: u64,
    timeout: u64,
    wait_until: String,
    eval: Option<String>,
    output: Option<PathBuf>,
    quiet: bool,
    _obey_robots: bool,
    _allow_private_network: bool,
    _v8_flags: Option<String>,
    _storage_dir: Option<PathBuf>,
) -> eyre::Result<()> {
    if !quiet {
        eprintln!("fetch: {url} (dump={dump:?}, wait={wait}s, timeout={timeout}s)");
    }

    // Process-level hard deadline: exit(124) after timeout + wait + 10s.
    // ponytail: HPX_CDP_COMMAND_TIMEOUT_MS and HPX_SCRIPT_DEADLINE_MS
    // not yet wired to browser config. Add when CDP command timeout is implemented.
    let deadline_secs = timeout + wait + 10;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(deadline_secs));
        std::process::exit(124);
    });

    // Original format: short-circuit browser, use raw HTTP client.
    if dump == DumpFormat::Original {
        let client = hpx::Client::new();
        let resp = client.get(&url).send().await.wrap_err("fetch failed")?;
        let bytes = resp.bytes().await.wrap_err("reading response body")?;
        write_output(&output, &bytes)?;
        return Ok(());
    }

    // ponytail: selector wait requires JS engine (v8). Ignored for now.
    if selector.is_some() && !quiet {
        eprintln!("warning: --selector requires v8, ignoring");
    }
    // ponytail: eval requires JS engine (v8). Ignored for now.
    if eval.is_some() && !quiet {
        eprintln!("warning: --eval requires v8, ignoring");
    }
    // ponytail: wait_until is not yet wired to Page::navigate. Ignored.
    let _ = wait_until;

    // Page is !Send, so spawn a dedicated std::thread with its own runtime.
    let url_clone = url.clone();
    let (tx, rx) = std::sync::mpsc::channel::<eyre::Result<String>>();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Err(eyre::Report::from(e)));
                return;
            }
        };

        let result = rt.block_on(async move {
            let mut page = Page::new(EngineHandle::new());
            page.navigate(&url_clone)
                .await
                .wrap_err("navigation failed")?;

            if wait > 0 {
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }

            let content = match dump {
                DumpFormat::Html => page.content(),
                DumpFormat::Text => page
                    .text_content()
                    .await
                    .wrap_err("text extraction failed")?,
                DumpFormat::Links => {
                    let html = page.content();
                    let links = extract_links(&html);
                    format_links_tsv(&links)
                }
                DumpFormat::Assets => {
                    let html = page.content();
                    let assets = extract_assets(&html);
                    format_assets_ndjson(&assets)
                }
                DumpFormat::Markdown => {
                    // ponytail: markdown conversion pending Task 4.4
                    page.content()
                }
                DumpFormat::Cookies => {
                    // ponytail: cookie extraction pending storage integration
                    "[]".to_string()
                }
                DumpFormat::Original => unreachable!("handled above"),
            };

            Ok::<_, eyre::Report>(content)
        });

        let _ = tx.send(result);
    });

    let content = rx.recv().wrap_err("page worker thread panicked")??;
    write_output(&output, content.as_bytes())?;

    Ok(())
}

fn write_output(path: &Option<PathBuf>, data: &[u8]) -> eyre::Result<()> {
    if let Some(p) = path {
        std::fs::write(p, data).wrap_err_with(|| format!("writing {}", p.display()))
    } else {
        std::io::stdout()
            .write_all(data)
            .wrap_err("writing to stdout")?;
        Ok(())
    }
}

fn extract_links(html: &str) -> Vec<(String, String)> {
    let dom = hpx_browser::html_parser::parse_html(html);
    let anchors = dom.get_elements_by_tag_name(NodeId::DOCUMENT, "a");
    let mut results = Vec::with_capacity(anchors.len());
    for &id in &anchors {
        let Some(node) = dom.get(id) else {
            continue;
        };
        let Some(elem) = node.as_element() else {
            continue;
        };
        let href = elem
            .attrs
            .iter()
            .find(|a| a.name.local == "href")
            .map(|a| a.value.as_str());
        let Some(href) = href else { continue };
        let text = dom.text_content(id);
        results.push((href.to_string(), text));
    }
    results
}

fn extract_assets(html: &str) -> Vec<(String, String)> {
    let dom = hpx_browser::html_parser::parse_html(html);
    let mut results = Vec::new();

    for &id in &dom.get_elements_by_tag_name(NodeId::DOCUMENT, "script") {
        let Some(node) = dom.get(id) else { continue };
        let Some(elem) = node.as_element() else {
            continue;
        };
        if let Some(src) = elem.attrs.iter().find(|a| a.name.local == "src") {
            results.push((src.value.clone(), "script".to_string()));
        }
    }

    for &id in &dom.get_elements_by_tag_name(NodeId::DOCUMENT, "link") {
        let Some(node) = dom.get(id) else { continue };
        let Some(elem) = node.as_element() else {
            continue;
        };
        if let Some(href) = elem.attrs.iter().find(|a| a.name.local == "href") {
            results.push((href.value.clone(), "link".to_string()));
        }
    }

    for &id in &dom.get_elements_by_tag_name(NodeId::DOCUMENT, "img") {
        let Some(node) = dom.get(id) else { continue };
        let Some(elem) = node.as_element() else {
            continue;
        };
        if let Some(src) = elem.attrs.iter().find(|a| a.name.local == "src") {
            results.push((src.value.clone(), "img".to_string()));
        }
    }

    results
}

fn format_links_tsv(links: &[(String, String)]) -> String {
    let mut out = String::new();
    for (url, text) in links {
        // Strip newlines from text to keep each entry on one line.
        let clean = text.replace(['\n', '\r'], " ");
        out.push_str(url);
        out.push('\t');
        out.push_str(&clean);
        out.push('\n');
    }
    out
}

fn format_assets_ndjson(assets: &[(String, String)]) -> String {
    let mut out = String::new();
    for (url, asset_type) in assets {
        let line = serde_json::json!({"url": url, "type": asset_type});
        out.push_str(&line.to_string());
        out.push('\n');
    }
    out
}

#[derive(serde::Serialize)]
struct ScrapeResult {
    url: String,
    title: Option<String>,
    html: Option<String>,
    text: Option<String>,
    eval: Option<String>,
    time_ms: u64,
    error: Option<String>,
}

pub(crate) async fn handle_scrape(
    urls: Vec<String>,
    eval: Option<String>,
    concurrency: usize,
    format: String,
    timeout: u64,
    quiet: bool,
    obey_robots: bool,
    allow_private_network: bool,
    v8_flags: Option<String>,
    storage_dir: Option<PathBuf>,
) -> eyre::Result<()> {
    if !quiet {
        eprintln!(
            "scrape: {} urls (concurrency={concurrency}, format={format}, timeout={timeout}s)",
            urls.len()
        );
        if let Some(expr) = &eval {
            eprintln!("  eval: {expr}");
        }
        eprintln!("  obey_robots: {obey_robots}");
        eprintln!("  allow_private_network: {allow_private_network}");
        if let Some(flags) = &v8_flags {
            eprintln!("  v8_flags: {flags}");
        }
        if let Some(dir) = &storage_dir {
            eprintln!("  storage_dir: {}", dir.display());
        }
    }

    let _ = (obey_robots, allow_private_network, v8_flags, storage_dir);
    // ponytail: obey_robots / allow_private_network / v8_flags / storage_dir
    // not yet wired to in-process worker. Ignored for now.

    let sem = std::sync::Arc::new(Semaphore::new(concurrency));
    let eval = std::sync::Arc::new(eval);

    let mut handles = Vec::with_capacity(urls.len());
    for url in urls {
        let permit = sem.clone().acquire_owned().await?;
        let eval = eval.clone();

        handles.push(tokio::spawn(async move {
            let result = scrape_single_url(&url, eval.as_deref(), timeout).await;
            drop(permit);
            result
        }));
    }

    let mut results: Vec<ScrapeResult> = Vec::with_capacity(handles.len());
    for h in handles {
        results.push(h.await.wrap_err("task join")?);
    }

    if format.as_str() == "text" {
        for r in &results {
            if let Some(err) = &r.error {
                eprintln!("error: {} — {err}", r.url);
            } else {
                let snippet = r
                    .text
                    .as_deref()
                    .or(r.html.as_deref())
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>();
                println!("{}: {}", r.url, snippet);
            }
        }
    } else {
        let json = serde_json::to_string_pretty(&results)?;
        println!("{json}");
    }

    Ok(())
}

/// Scrape a single URL in-process. Page is !Send, so it runs on a dedicated
/// std::thread with its own single-threaded tokio runtime.
async fn scrape_single_url(url: &str, eval: Option<&str>, _timeout: u64) -> ScrapeResult {
    let t0 = Instant::now();
    let url_owned = url.to_string();
    let eval_owned = eval.map(|e| e.to_string());
    let url_for_err = url_owned.clone();

    let (tx, rx) = std::sync::mpsc::channel::<eyre::Result<ScrapeResult>>();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Err(eyre::Report::from(e)));
                return;
            }
        };

        let result = rt.block_on(async move {
            let mut page = Page::new(EngineHandle::new());
            page.navigate(&url_owned)
                .await
                .wrap_err("navigation failed")?;

            let title = page.title();
            let html = page.content();
            let text = page
                .text_content()
                .await
                .wrap_err("text extraction failed")
                .ok();

            let eval_result = if let Some(ref expr) = eval_owned {
                page.evaluate(expr).ok()
            } else {
                None
            };

            Ok::<_, eyre::Report>(ScrapeResult {
                url: url_owned,
                title: Some(title).filter(|s| !s.is_empty()),
                html: Some(html),
                text,
                eval: eval_result,
                time_ms: t0.elapsed().as_millis() as u64,
                error: None,
            })
        });

        let _ = tx.send(result);
    });

    match rx.recv() {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ScrapeResult {
            url: url_for_err,
            title: None,
            html: None,
            text: None,
            eval: None,
            time_ms: t0.elapsed().as_millis() as u64,
            error: Some(e.to_string()),
        },
        Err(_) => ScrapeResult {
            url: url_for_err,
            title: None,
            html: None,
            text: None,
            eval: None,
            time_ms: t0.elapsed().as_millis() as u64,
            error: Some("worker thread panicked".to_string()),
        },
    }
}

pub(crate) async fn handle_serve(
    port: u16,
    host: String,
    _stealth: bool,
    workers: u16,
    _allow_file_access: bool,
    _storage_dir: Option<PathBuf>,
    quiet: bool,
    _obey_robots: bool,
    _allow_private_network: bool,
    _v8_flags: Option<String>,
) -> eyre::Result<()> {
    // ponytail: stealth not wired to CDP server yet.
    // ponytail: allow_file_access, storage_dir, obey_robots, allow_private_network, v8_flags not wired yet.

    let html = "<html><body></body></html>";

    if !host.parse::<std::net::IpAddr>()?.is_loopback() {
        eprintln!(
            "WARNING: CDP WebSocket bound to {host}:{port} — accessible to external networks. Full browser control is exposed."
        );
    }

    if workers <= 1 {
        let server = hpx_browser::protocol::server::CdpServer::start(html, port)
            .map_err(|e| eyre::eyre!("failed to start CDP server: {}", e))?;

        if !quiet {
            println!("hpx serve v{}", env!("CARGO_PKG_VERSION"));
            println!("CDP WebSocket URL: {}", server.ws_url());
        }

        tokio::signal::ctrl_c().await?;
        drop(server);
    } else {
        // ponytail: TCP load balancer not yet implemented; each worker has its own port. Add when round-robin routing is needed.
        let mut servers = Vec::with_capacity(workers as usize);
        for i in 0..workers {
            let p = port + i;
            let server = hpx_browser::protocol::server::CdpServer::start(html, p)
                .map_err(|e| eyre::eyre!("failed to start CDP server on port {}: {}", p, e))?;
            if !quiet {
                println!(
                    "hpx serve v{} — worker {} CDP WebSocket URL: {}",
                    env!("CARGO_PKG_VERSION"),
                    i,
                    server.ws_url()
                );
            }
            servers.push(server);
        }

        tokio::signal::ctrl_c().await?;
        drop(servers);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_links_basic() {
        let html = r#"<html><body>
<a href="https://example.com">Example</a>
<a href="/relative">Relative</a>
<a>No Href</a>
</body></html>"#;
        let links = extract_links(html);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "https://example.com");
        assert_eq!(links[0].1, "Example");
        assert_eq!(links[1].0, "/relative");
        assert_eq!(links[1].1, "Relative");
    }

    #[test]
    fn extract_links_empty() {
        let html = "<html><body><p>No links</p></body></html>";
        let links = extract_links(html);
        assert!(links.is_empty());
    }

    #[test]
    fn extract_assets_basic() {
        let html = r#"<html><head>
<script src="/app.js"></script>
<link href="/style.css" rel="stylesheet">
</head><body>
<img src="/logo.png">
</body></html>"#;
        let assets = extract_assets(html);
        assert_eq!(assets.len(), 3);
        assert_eq!(assets[0], ("/app.js".to_string(), "script".to_string()));
        assert_eq!(assets[1], ("/style.css".to_string(), "link".to_string()));
        assert_eq!(assets[2], ("/logo.png".to_string(), "img".to_string()));
    }

    #[test]
    fn extract_assets_empty() {
        let html = "<html><body><p>No assets</p></body></html>";
        let assets = extract_assets(html);
        assert!(assets.is_empty());
    }

    #[test]
    fn format_links_tsv_basic() {
        let links = vec![
            ("https://a.com".to_string(), "A".to_string()),
            ("https://b.com".to_string(), "B\nNewline".to_string()),
        ];
        let tsv = format_links_tsv(&links);
        assert_eq!(tsv, "https://a.com\tA\nhttps://b.com\tB Newline\n");
    }

    #[test]
    fn format_assets_ndjson_basic() {
        let assets = vec![
            ("/a.js".to_string(), "script".to_string()),
            ("/b.css".to_string(), "link".to_string()),
        ];
        let ndjson = format_assets_ndjson(&assets);
        assert_eq!(
            ndjson,
            "{\"url\":\"/a.js\",\"type\":\"script\"}\n{\"url\":\"/b.css\",\"type\":\"link\"}\n"
        );
    }

    #[test]
    fn scrape_result_serializes() {
        let r = ScrapeResult {
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            html: Some("<html></html>".to_string()),
            text: None,
            eval: None,
            time_ms: 42,
            error: None,
        };
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(json.contains("https://example.com"));
        assert!(json.contains("Example"));
    }

    #[test]
    fn scrape_result_error_serializes() {
        let r = ScrapeResult {
            url: "https://bad.example".to_string(),
            title: None,
            html: None,
            text: None,
            eval: None,
            time_ms: 100,
            error: Some("timeout".to_string()),
        };
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(json.contains("timeout"));
    }

    #[test]
    fn extract_links_nested_elements() {
        let html = r#"<html><body>
<a href="https://example.com"><span>Nested</span> Text</a>
</body></html>"#;
        let links = extract_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "https://example.com");
        assert!(links[0].1.contains("Nested"));
        assert!(links[0].1.contains("Text"));
    }

    #[test]
    fn cdp_server_starts_and_stops() {
        let server =
            hpx_browser::protocol::server::CdpServer::start_ephemeral("<html><body></body></html>")
                .expect("CdpServer should start");
        assert!(server.port() > 0);
        assert!(server.ws_url().contains("127.0.0.1"));
        drop(server);
    }

    #[test]
    fn extract_assets_multiple_scripts() {
        let html = r#"<html><head>
<script src="/a.js"></script>
<script src="/b.js"></script>
<script>inline</script>
</head></html>"#;
        let assets = extract_assets(html);
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].0, "/a.js");
        assert_eq!(assets[1].0, "/b.js");
    }

    #[test]
    fn ndjson_escapes_special_characters() {
        let assets = vec![
            (
                "https://example.com/path?q=\"hello\"".to_string(),
                "script".to_string(),
            ),
            (
                "https://example.com/normal".to_string(),
                "style".to_string(),
            ),
        ];
        let output = super::format_assets_ndjson(&assets);
        for line in output.lines() {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each NDJSON line must be valid JSON");
            assert!(parsed["url"].is_string());
            assert!(parsed["type"].is_string());
        }
        // Verify the URL with quotes is properly escaped
        let first_line = output.lines().next().unwrap();
        assert!(
            first_line.contains("\\\"hello\\\""),
            "quotes should be escaped"
        );
    }
}
