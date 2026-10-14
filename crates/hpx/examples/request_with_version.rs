#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]
use http::Version;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    // Use the API you're already familiar with
    let resp = hpx::get("https://www.google.com")
        .version(Version::HTTP_11)
        .send()
        .await?;

    assert_eq!(resp.version(), Version::HTTP_11);
    println!("{}", resp.text().await?);

    Ok(())
}
