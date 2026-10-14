//! hpx-browser test/example crate.

#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    missing_docs,
    unfulfilled_lint_expectations
)]
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("hpx=debug")
        .try_init()
        .ok();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let server = tokio::spawn(async move {
        for (path, ct, body) in [
            ("/a.css", "text/css", "body{}"),
            ("/b.js", "application/javascript", "alert(1)"),
        ] {
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
