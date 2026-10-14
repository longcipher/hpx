#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
use hpx::Proxy;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    // Use the API you're already familiar with
    let resp = hpx::get("https://api.ip.sb/ip")
        .proxy(Proxy::all("socks5h://localhost:6153")?)
        .send()
        .await?;
    println!("{}", resp.text().await?);

    Ok(())
}
