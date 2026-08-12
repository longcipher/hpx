use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("hpx=debug").try_init().ok();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let server = tokio::spawn(async move {
        for (path, ct, body) in [("/a.css", "text/css", "body{}"), ("/b.js", "application/javascript", "alert(1)")] {
            let (mut stream, _) = listener.accept().await.unwrap();
            eprintln!("[server] got connection for {path}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let client = hpx::Client::new();
    let r = client.get(&format!("{base}/a.css")).send().await;
    eprintln!("resp1: {r:?}");
    if let Ok(resp) = r {
        eprintln!("status: {}, headers: {:?}", resp.status(), resp.headers());
        eprintln!("text: {:?}", resp.text().await);
    }
    let r2 = client.get(&format!("{base}/b.js")).send().await;
    eprintln!("resp2: {r2:?}");
    server.await.unwrap();
}
