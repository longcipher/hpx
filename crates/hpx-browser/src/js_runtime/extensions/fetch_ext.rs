use deno_core::op2;

#[op2]
#[string]
pub(crate) fn op_fetch(#[string] _url: String) -> String {
    // ponytail: stub — implement with hpx::Client when net module is ready
    r#"{"status":200,"url":"about:blank","headers":[],"body":"","error":"fetch not yet implemented"}"#.to_string()
}

#[op2]
#[string]
pub(crate) fn op_net_fetch_sync(#[string] _url: String, #[string] _method: String) -> String {
    // ponytail: stub
    r#"{"status":200,"url":"about:blank","headers":[],"body":"","error":"fetch not yet implemented"}"#.to_string()
}

deno_core::extension!(fetch_extension, ops = [op_fetch, op_net_fetch_sync],);
