//! Fuzz cookie-jar insertion with hostile `Set-Cookie` values.
//!
//! A jar is fed strings that came straight off the wire, then asked to match
//! them against request URIs for domain/path/secure scoping. Both halves are
//! attacker-influenced: the `Set-Cookie` line is chosen by the origin, and the
//! request URI is chosen by the page being loaded. Bugs here are classic
//! cookie-tossing / cookie-shadowing primitives, so the scoping invariants are
//! worth pinning even though the immediate goal is "no panic".
//!
//! Assertions:
//!
//! 1. **No panic.** Malformed cookie syntax must be tolerated.
//! 2. **Bounded growth.** One `Set-Cookie` line must not expand into an
//!    unbounded number of stored entries — that would be a memory-exhaustion
//!    vector reachable from any origin.
//! 3. **Read-back consistency.** Whatever the jar returns for a name must equal
//!    what it stored, so a lookup cannot silently substitute another origin's
//!    value.
//! 4. **Secure scoping** and **path scoping** and **host scoping** are pinned by
//!    the deterministic companion suite in `crates/hpx/tests/cookie_scoping.rs`,
//!    where the correct answer is known exactly and a fuzzed input cannot mask a
//!    regression.

#![no_main]

use hpx::cookie::Jar;
use libfuzzer_sys::fuzz_target;

/// Upper bound on the entries a single `Set-Cookie` line may produce.
const MAX_ENTRIES_PER_LINE: usize = 8;

/// Hosts the jar is queried against, chosen so each one stresses a different
/// scoping rule: an exact match, a parent domain, a bare public suffix, an
/// unrelated origin, and a deeper path.
const PROBE_URIS: &[&str] = &[
    "http://fuzz.example/",
    "https://fuzz.example/",
    "http://www.fuzz.example/",
    "http://notfuzz.example/",
    "http://fuzz.example/admin/settings",
];

fuzz_target!(|data: &[u8]| {
    // Cookie syntax is ASCII; fold everything else to a space so the fuzzer
    // spends its budget on structural variation rather than byte classes.
    let text: String = data
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' || b == b'\t' {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    if text.is_empty() {
        return;
    }

    let jar = Jar::new(false);

    // 1. No panic on hostile syntax.
    jar.add_cookie_str(&text, "http://fuzz.example/");

    // 2. Bounded growth.
    let stored = jar.get_all();
    assert!(
        stored.len() <= MAX_ENTRIES_PER_LINE,
        "one Set-Cookie line produced {} entries",
        stored.len()
    );

    // 3. Read-back consistency across a spread of probe URIs.
    for probe in PROBE_URIS {
        for cookie in &stored {
            let Some(found) = jar.get(cookie.name(), *probe) else {
                continue;
            };
            assert_eq!(
                found.value(),
                cookie.value(),
                "jar returned a different value for {:?} at {probe} than it stored",
                cookie.name()
            );
        }
    }
});
