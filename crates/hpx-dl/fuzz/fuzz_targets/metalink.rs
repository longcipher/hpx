//! Fuzz the Metalink v4 parser (RFC 5854).
//!
//! Must never panic on arbitrary input.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hpx_dl::metalink::parse_metalink(data);
});
