use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

fn bench_reload_html(c: &mut Criterion) {
    let small = "<html><head><title>t</title></head><body><p>hello</p></body></html>";
    let medium = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<p>line</p>".repeat(500)
    );
    let large = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<div class=\"item\"><span>content</span><a href=\"#\">link</a></div>".repeat(2000)
    );

    let mut group = c.benchmark_group("navigation_reload_html");
    for (name, html) in [
        ("small", small),
        ("medium", medium.as_str()),
        ("large", large.as_str()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), html, |b, html| {
            b.iter(|| {
                let engine = hpx_browser::host::EngineHandle::new();
                let mut page = hpx_browser::page::Page::new(engine);
                page.reload_html(html, "http://example.com");
                page
            });
        });
    }
    group.finish();
}

fn bench_navigate_to_local(c: &mut Criterion) {
    use std::io::Read;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // Start a local HTTP server serving a test page
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let html = include_str!("test_page.html").to_string();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                html.len(),
                html
            );
            use std::io::Write;
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let url = format!("http://127.0.0.1:{port}");

    c.bench_function("navigate_to_local", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let engine = hpx_browser::host::EngineHandle::new();
                let mut page = hpx_browser::page::Page::new(engine);
                let _ = page.navigate(&url).await;
                page
            })
        });
    });
}

criterion_group!(benches, bench_reload_html, bench_navigate_to_local);
criterion_main!(benches);
