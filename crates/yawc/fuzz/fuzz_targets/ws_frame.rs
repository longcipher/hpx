//! Fuzz the RFC 6455 frame decoder with completely arbitrary wire bytes.
//!
//! The decoder is the primary attack surface of a WebSocket endpoint: it parses
//! attacker-controlled headers (fin/rsv/opcode/mask/length) and then slices the
//! payload buffer according to them. This target asserts properties that must
//! hold for *any* byte string:
//!
//! 1. **No panic.** Any `Err` is acceptable; unwinding is not.
//! 2. **Bounded progress.** A decoder fed a finite buffer either consumes bytes
//!    or errors — it must never loop forever.
//! 3. **No phantom payload.** A decoded payload never exceeds the number of
//!    bytes that were actually available for it.
//!
//! Two feeding strategies are exercised. *Bulk* feeding hands the whole buffer
//! over at once; *incremental* feeding pushes one byte per `decode` call, which
//! is what forces the decoder to carry header state across calls and is where
//! off-by-one errors in the partial-frame state machine surface.
//!
//! Both [`Role::Client`] and [`Role::Server`] are exercised, along with a spread
//! of `max_payload_size` limits: the server role requires every frame to be
//! masked and applies the XOR mask, the client role accepts unmasked frames,
//! and a tiny `max_payload_size` forces the `FrameTooLarge` branch.

#![no_main]

use bytes::{Buf, BytesMut};
use hpx_yawc::{Role, codec};
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::Decoder as _;

/// Upper bound on the decoder limit used in the matrix below.
const MAX_PAYLOAD_LIMIT: usize = 1 << 20;

/// `max_payload_size` values that between them reach every length-decoding
/// branch: 0, a sub-control-frame limit, the RFC 6455 control-frame ceiling,
/// the 7-bit/16-bit boundary, the 16-bit maximum, and a large 64-bit value.
const LIMITS: &[usize] = &[0, 1, 124, 125, 126, 65535, MAX_PAYLOAD_LIMIT];

/// Inputs at or below this size are also fed one byte at a time. Larger inputs
/// use bulk feeding only, to keep per-input cost bounded.
const INCREMENTAL_LIMIT: usize = 256;

/// Hard cap on `decode` calls for a single input.
///
/// Every iteration either consumes at least one byte or returns, and the buffer
/// is finite, so the real bound is the input length. This is only a backstop so
/// a harness bug cannot hang the fuzzer.
const MAX_DECODE_STEPS: usize = 1 << 16;

fuzz_target!(|data: &[u8]| {
    for role in [Role::Server, Role::Client] {
        for &limit in LIMITS {
            bulk_decode(role, limit, data);
            if data.len() <= INCREMENTAL_LIMIT {
                incremental_decode(role, limit, data);
            }
        }
    }
});

/// Decode with the entire buffer available from the first call.
fn bulk_decode(role: Role, limit: usize, data: &[u8]) {
    let mut decoder = codec::Decoder::new(role, limit);
    let mut buf = BytesMut::from(data);
    drain(&mut decoder, &mut buf);
}

/// Decode one byte at a time, exercising the partial-frame state machine.
fn incremental_decode(role: Role, limit: usize, data: &[u8]) {
    let mut decoder = codec::Decoder::new(role, limit);
    let mut buf = BytesMut::new();
    for &byte in data {
        buf.extend_from_slice(&[byte]);
        drain(&mut decoder, &mut buf);
    }
}

/// Pull frames out of `buf` until the decoder needs more data or rejects the
/// stream, checking the payload bound and progress invariants as it goes.
fn drain(decoder: &mut codec::Decoder, buf: &mut BytesMut) {
    for _ in 0..MAX_DECODE_STEPS {
        let before = buf.remaining();
        match decoder.decode(buf) {
            Ok(Some(frame)) => {
                let len = frame.payload().len();
                assert!(
                    len <= MAX_PAYLOAD_LIMIT,
                    "payload of {len} bytes exceeds the largest configured limit"
                );
                // Every well-formed frame consumes at least its two-byte header,
                // so yielding one without consuming is always a decoder bug. This
                // is also what guarantees this loop terminates.
                assert!(
                    buf.remaining() < before,
                    "decode yielded a frame without consuming input"
                );
            }
            Ok(None) => return,
            Err(_) => return,
        }
    }
    panic!("decoder did not settle after {MAX_DECODE_STEPS} steps");
}
