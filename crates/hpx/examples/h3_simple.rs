//! Simple HTTP/3 example.
//!
//! This example demonstrates making a GET request to a public HTTP/3 server
//! using `http3_only()`.

#[tokio::main]
async fn main() -> hpx::Result<()> {
    let client = hpx::Client::builder().http3_only().build()?;

    let resp = client.get("https://cloudflare-quic.com/").send().await?;

    println!("Status: {}", resp.status());
    println!("Body: {}", resp.text().await?);
    Ok(())
}
