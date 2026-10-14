#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
use hpx::tls::KeyLog;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    // Build a client
    let client = hpx::Client::builder()
        .keylog(KeyLog::from_file("keylog.txt"))
        .cert_verification(false)
        .build()?;

    // Use the API you're already familiar with
    let resp = client.get("https://yande.re/post.json").send().await?;
    println!("{}", resp.text().await?);

    Ok(())
}
