//! Fuzz the three WebSocket masking implementations against each other.
//!
//! `apply_mask` dispatches between a 64-bit-word path and a 32-bit-word path
//! based on buffer length, and both use `unsafe` pointer arithmetic to process
//! whole words. The failure modes are subtle: a wrong rotation at a
//! non-word-aligned head, a wrong mask phase for the trailing bytes, or an
//! off-by-one at the buffer end. None of these panic — they silently corrupt
//! the payload, which then fails much later as an application-level decode
//! error.
//!
//! This target treats the byte-at-a-time reference (`mask[i % 4]`) as the
//! specification and drives the two optimized paths with:
//!
//! * every buffer start alignment mod 4 (and mod 8), so prefix/word/suffix
//!   splitting is fully exercised;
//! * lengths spanning both dispatch branches and every tail remainder;
//! * masks chosen to include all-zero and all-ones edge cases.
//!
//! It also asserts the involution property, since RFC 6455 masking is its own
//! inverse and every real use of these functions depends on that.

#![no_main]

use hpx_yawc::mask::{apply_mask, apply_mask_fast32, apply_mask_fast64};
use libfuzzer_sys::fuzz_target;

/// The reference implementation straight from RFC 6455 §5.3.
fn reference_mask(buf: &mut [u8], mask: [u8; 4]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
}

fuzz_target!(|data: &[u8]| {
    // The first four bytes select the mask, the remainder is the payload.
    // Inputs shorter than four bytes are zero-padded rather than sliced, so an
    // empty or tiny input exercises the zero-mask path instead of panicking in
    // the harness.
    let split = data.len().min(4);
    let (mask_bytes, payload) = data.split_at(split);
    let mut mask = [0u8; 4];
    mask[..split].copy_from_slice(mask_bytes);
    if payload.is_empty() {
        return;
    }

    // Pad so the payload can be placed at every alignment inside one buffer.
    // 8 bytes of padding covers both the u32 and u64 word alignments.
    let mut buffer = vec![0u8; payload.len() + 8];
    for offset in 0..8usize {
        buffer[offset..offset + payload.len()].copy_from_slice(payload);
        let slice = &mut buffer[offset..offset + payload.len()];

        let mut want = slice.to_vec();
        reference_mask(&mut want, mask);

        let mut via_fast32 = slice.to_vec();
        apply_mask_fast32(&mut via_fast32, mask);
        assert_eq!(
            via_fast32, want,
            "apply_mask_fast32 diverged at alignment {offset}"
        );

        let mut via_fast64 = slice.to_vec();
        apply_mask_fast64(&mut via_fast64, mask);
        assert_eq!(
            via_fast64, want,
            "apply_mask_fast64 diverged at alignment {offset}"
        );

        // The dispatcher must agree with both of its branches.
        let mut via_dispatch = slice.to_vec();
        apply_mask(&mut via_dispatch, mask);
        assert_eq!(
            via_dispatch, want,
            "apply_mask diverged from the reference at alignment {offset}"
        );

        // RFC 6455 masking is an involution; every caller relies on this to
        // unmask without keeping the original payload around.
        apply_mask(&mut via_dispatch, mask);
        assert_eq!(
            via_dispatch, slice,
            "masking is not an involution at alignment {offset}"
        );
    }
});
