//! This example illustrates the way to send and receive arbitrary JSON.
//!
//! This is useful for some ad-hoc experiments and situations when you don't
//! really care about the structure of the JSON and just need to display it or
//! process it at runtime.

// This is using the `tokio` runtime. You'll need the following dependency:
//
// `tokio = { version = "1", features = ["full"] }`
#[tokio::main]
async fn main() -> hpx::Result<()> {
    let echo_json: serde_json::Value = hpx::post("https://jsonplaceholder.typicode.com/posts")
        .json(&serde_json::json!({
            "title": "hpx.rs",
            "body": "https://docs.rs/hpx",
            "userId": 1
        }))
        .send()
        .await?
        .json()
        .await?;

    println!("{echo_json:#?}");
    // Object(
    //     {
    //         "body": String(
    //             "https://docs.rs/hpx"
    //         ),
    //         "id": Number(
    //             101
    //         ),
    //         "title": String(
    //             "hpx.rs"
    //         ),
    //         "userId": Number(
    //             1
    //         )
    //     }
    // )
    Ok(())
}
