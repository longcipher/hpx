use std::net::IpAddr;

use hpx::redirect::Policy;

#[tokio::main]
async fn main() -> hpx::Result<()> {
    // Use the API you're already familiar with
    let resp = hpx::get("http://www.baidu.com")
        .redirect(Policy::default())
        .local_address(IpAddr::from([192, 168, 1, 226]))
        .send()
        .await?;
    println!("{}", resp.text().await?);

    Ok(())
}
