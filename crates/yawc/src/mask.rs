// Optimized masking implementation.

use rand::RngExt;

/// Returns a fresh random 4-byte WebSocket masking key.
///
/// RFC 6455 §5.3 requires the client to choose the masking key at random,
/// and §10.3 requires it to be *unpredictable* so that an attacker cannot
/// poison an intermediary's cache by guessing the key of a masked frame.
///
/// The key is therefore drawn from the thread-local CSPRNG (`rand::rng()`,
/// ChaCha12) rather than from a fast non-cryptographic PRNG: a
/// `SmallRng`/`xoshiro` state can be recovered from observed outputs, which
/// would let an attacker predict the following keys. The CSPRNG costs a few
/// nanoseconds per frame, which is negligible next to the syscall and the
/// encryption a real connection performs anyway.
#[inline]
pub(crate) fn fast_mask_bytes() -> [u8; 4] {
    rand::rng().random()
}

/// Payload length above which the optional vectorised XOR path is considered.
///
/// Criterion results on an Apple M1 Max (aarch64) show the word-aligned scalar
/// path already matches the hand-written NEON path from 96 B to 16 KiB, so
/// NEON only pays off for frames large enough to amortise the dispatch and
/// tail cost. On x86_64 the AVX2 path processes 32 bytes per iteration and is
/// worth trying much earlier.
#[cfg(all(feature = "simd-mask", target_arch = "aarch64"))]
const SIMD_THRESHOLD: usize = 64 * 1024;
#[cfg(all(feature = "simd-mask", not(target_arch = "aarch64")))]
const SIMD_THRESHOLD: usize = 64;

/// AVX2 XOR masking, 32 bytes per iteration.
///
/// The 4-byte key is broadcast into every 32-bit lane. Because the mask cycle
/// (period 4) divides the vector width (32), no rotation is needed between
/// successive chunks or before the scalar tail.
#[cfg(all(feature = "simd-mask", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn apply_mask_avx2(buf: &mut [u8], mask: [u8; 4]) {
    use std::arch::x86_64::{
        __m256i, _mm256_loadu_si256, _mm256_set1_epi32, _mm256_storeu_si256, _mm256_xor_si256,
    };

    let vmask: __m256i = unsafe { _mm256_set1_epi32(i32::from_ne_bytes(mask)) };
    let ptr = buf.as_mut_ptr();
    let chunks = buf.len() / 32;
    for index in 0..chunks {
        let offset = index * 32;
        // SAFETY: `offset + 32 <= buf.len()` because `chunks = buf.len() / 32`.
        // Both intrinsics accept unaligned addresses, which matters because a
        // frame payload starts after a 2/6/10/14-byte header.
        let chunk = unsafe { _mm256_loadu_si256(ptr.add(offset).cast()) };
        unsafe { _mm256_storeu_si256(ptr.add(offset).cast(), _mm256_xor_si256(chunk, vmask)) };
    }
    apply_mask_fallback(&mut buf[chunks * 32..], mask);
}

/// NEON XOR masking, 16 bytes per iteration.
///
/// NEON is part of the aarch64 baseline, so no runtime feature detection is
/// needed. As with AVX2, the vector width is a multiple of the key period.
#[cfg(all(feature = "simd-mask", target_arch = "aarch64"))]
#[inline]
fn apply_mask_neon(buf: &mut [u8], mask: [u8; 4]) {
    use std::arch::aarch64::{vdupq_n_u32, veorq_u8, vld1q_u8, vreinterpretq_u8_u32, vst1q_u8};

    // One `dup` builds [m0, m1, m2, m3, m0, ...] in a register. Building the
    // same pattern through a 16-byte stack buffer dominated small frames.
    // SAFETY: NEON is part of the aarch64 baseline, so the intrinsic is
    // always available on this target.
    let vmask = unsafe { vreinterpretq_u8_u32(vdupq_n_u32(u32::from_ne_bytes(mask))) };
    let ptr = buf.as_mut_ptr();
    let chunks = buf.len() / 16;
    for index in 0..chunks {
        let offset = index * 16;
        // SAFETY: `offset + 16 <= buf.len()` because `chunks = buf.len() / 16`.
        // The load/store are unaligned-safe.
        let chunk = unsafe { vld1q_u8(ptr.add(offset)) };
        unsafe { vst1q_u8(ptr.add(offset), veorq_u8(chunk, vmask)) };
    }
    apply_mask_fallback(&mut buf[chunks * 16..], mask);
}

/// Try the architecture's vectorised path, returning `false` when unavailable.
#[cfg(all(feature = "simd-mask", target_arch = "x86_64"))]
#[inline]
fn try_apply_mask_simd(buf: &mut [u8], mask: [u8; 4]) -> bool {
    if !std::is_x86_feature_detected!("avx2") {
        return false;
    }
    // SAFETY: AVX2 availability was checked immediately above.
    unsafe { apply_mask_avx2(buf, mask) };
    true
}

/// Try the architecture's vectorised path, returning `false` when unavailable.
#[cfg(all(feature = "simd-mask", target_arch = "aarch64"))]
#[inline]
fn try_apply_mask_simd(buf: &mut [u8], mask: [u8; 4]) -> bool {
    apply_mask_neon(buf, mask);
    true
}

/// Try the architecture's vectorised path, returning `false` when unavailable.
#[cfg(all(
    feature = "simd-mask",
    not(any(target_arch = "x86_64", target_arch = "aarch64"))
))]
fn try_apply_mask_simd(_buf: &mut [u8], _mask: [u8; 4]) -> bool {
    false
}

/// Mask/unmask a frame with optimal strategy selection.
///
/// This function automatically selects the fastest masking implementation based on:
/// - Buffer size
/// - Architecture alignment
/// - Whether the opt-in `simd-mask` feature is enabled
#[inline]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn apply_mask(buf: &mut [u8], mask: [u8; 4]) {
    #[cfg(feature = "simd-mask")]
    {
        if buf.len() >= SIMD_THRESHOLD && try_apply_mask_simd(buf, mask) {
            return;
        }
    }

    // Criterion benchmarks on the core codec path (16 B – 64 KiB) show the
    // word-aligned 64-bit path is faster than the 32-bit `align_to_mut` path at
    // every measured size, including the 125/126-byte frames that dominate
    // exchange traffic, so there is no size split.
    apply_mask_fast64(buf, mask);
}

/// A safe unoptimized mask application.
#[inline(always)]
fn apply_mask_fallback(buf: &mut [u8], mask: [u8; 4]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte ^= mask[i & 3];
    }
}

/// Faster version of `apply_mask()` which operates on 4-byte blocks.
#[doc(hidden)]
#[inline(always)]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn apply_mask_fast32(buf: &mut [u8], mask: [u8; 4]) {
    let mask_u32 = u32::from_ne_bytes(mask);
    // SAFETY: `[u8]` has no alignment requirement and `align_to_mut::<u32>()` splits
    // the slice into a prefix (unaligned head), an aligned middle (reinterpreted as
    // `[u32]`), and a suffix (unaligned tail). The cast is sound because `u8` is a
    // plain old data type and the alignment of the middle slice is guaranteed by
    // `align_to_mut`. All three sub-slices still refer to valid, initialized memory
    // owned by `buf`.
    let (prefix, words, suffix) = unsafe { buf.align_to_mut::<u32>() };
    apply_mask_fallback(prefix, mask);

    let head = prefix.len() & 3;
    let mask_u32 = if head > 0 {
        if cfg!(target_endian = "big") {
            mask_u32.rotate_left(8 * head as u32)
        } else {
            mask_u32.rotate_right(8 * head as u32)
        }
    } else {
        mask_u32
    };

    for word in words.iter_mut() {
        *word ^= mask_u32;
    }

    apply_mask_fallback(suffix, mask_u32.to_ne_bytes());
}

/// Default 64-bit implementation, used for every payload size.
///
/// This implementation uses `read_unaligned`/`write_unaligned` instead of
/// `align_to_mut::<u64>()` to avoid the runtime alignment-computation overhead
/// and the byte-at-a-time fallback for prefix/suffix chunks.
///
/// Benchmarks on aarch64 (Apple M1 Max) show this is consistently 8–13% faster
/// than the previous `align_to_mut`-based implementation for buffers up to ~1 KiB,
/// and tied within noise for larger buffers (64 KiB). The improvement holds
/// across all input alignments (0, 1, 2, 3 byte offsets), which matters because
/// real WebSocket frame payloads often start at non-8-byte-aligned offsets
/// (header sizes 6, 10, 14 bytes → offsets 6, 2, 6 mod 8).
///
/// Key insight: on modern x86_64/aarch64, unaligned loads/stores have the same
/// cost as aligned ones, so the `align_to_mut` dance (split into prefix/aligned/
/// suffix, rotate mask for each boundary) is pure overhead with no benefit.
#[doc(hidden)]
#[inline(always)]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn apply_mask_fast64(buf: &mut [u8], mask: [u8; 4]) {
    // Build the 64-bit mask by repeating the 32-bit mask twice.
    // In little-endian byte form this is [m0, m1, m2, m3, m0, m1, m2, m3],
    // which matches the 4-byte framing-key cycle (period 4 divides 8, so the
    // mask never rotates between consecutive 8-byte chunks).
    let mask_u32 = u32::from_ne_bytes(mask);
    let mask_u64 = (mask_u32 as u64) | ((mask_u32 as u64) << 32);

    let chunks_len = buf.len() / 8;
    let ptr = buf.as_mut_ptr();
    for i in 0..chunks_len {
        let offset = i * 8;
        // SAFETY: `offset + 8 <= buf.len()` because `chunks_len = buf.len() / 8`.
        // `read_unaligned`/`write_unaligned` are sound for any alignment, so this
        // works correctly even when `buf` is a sub-slice starting at a non-8-byte
        // aligned offset (e.g. a WS frame payload after a 6/10/14-byte header).
        let word = unsafe { ptr.add(offset).cast::<u64>().read_unaligned() };
        unsafe {
            ptr.add(offset)
                .cast::<u64>()
                .write_unaligned(word ^ mask_u64)
        };
    }

    // Handle the tail (0–7 trailing bytes). The tail starts at `chunks_len * 8`,
    // which is a multiple of 8, so the mask cycle (period 4) is back at offset 0
    // — we can use the original `mask` directly without rotation.
    let tail_start = chunks_len * 8;
    apply_mask_fallback(&mut buf[tail_start..], mask);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]

    fn test_apply_mask() {
        let mask = [0x6d, 0xb6, 0xb2, 0x80];
        let unmasked = [
            0xf3, 0x00, 0x01, 0x02, 0x03, 0x80, 0x81, 0x82, 0xff, 0xfe, 0x00, 0x17, 0x74, 0xf9,
            0x12, 0x03,
        ];

        for data_len in 0..=unmasked.len() {
            let unmasked = &unmasked[0..data_len];
            // Check masking with different alignment.
            for off in 0..=3 {
                if unmasked.len() < off {
                    continue;
                }
                let mut masked = unmasked.to_vec();
                apply_mask_fallback(&mut masked[off..], mask);

                let mut masked_fast32 = unmasked.to_vec();
                apply_mask_fast32(&mut masked_fast32[off..], mask);

                let mut masked_fast64 = unmasked.to_vec();
                apply_mask_fast64(&mut masked_fast64[off..], mask);

                assert_eq!(masked, masked_fast32);
                assert_eq!(masked, masked_fast64);
            }
        }
    }

    #[test]

    fn test_mask_unmask_identity() {
        // Test that applying mask twice returns original data
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];
        let original = b"Hello, World! This is a test message with various lengths.";

        let mut data = original.to_vec();
        apply_mask(&mut data, mask);

        // Data should be masked now
        assert_ne!(&data[..], &original[..]);

        // Apply mask again to unmask
        apply_mask(&mut data, mask);

        // Should be back to original
        assert_eq!(&data[..], &original[..]);
    }

    #[test]

    fn test_mask_all_zeros() {
        let mask = [0x00, 0x00, 0x00, 0x00];
        let original = b"Test data";

        let mut data = original.to_vec();
        apply_mask(&mut data, mask);

        // With zero mask, data should be unchanged
        assert_eq!(&data[..], &original[..]);
    }

    #[test]

    fn test_mask_all_ones() {
        let mask = [0xFF, 0xFF, 0xFF, 0xFF];
        let original = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        let expected = vec![0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88];

        let mut data = original.clone();
        apply_mask(&mut data, mask);

        assert_eq!(data, expected);
    }

    #[test]

    fn test_mask_edge_cases() {
        let mask = [0x12, 0x34, 0x56, 0x78];

        // Test empty buffer
        let mut empty: Vec<u8> = vec![];
        apply_mask(&mut empty, mask);
        assert_eq!(empty.len(), 0);

        // Test single byte
        let mut single = vec![0xAB];
        apply_mask(&mut single, mask);
        assert_eq!(single, vec![0xAB ^ 0x12]);

        // Test two bytes
        let mut two = vec![0xAB, 0xCD];
        apply_mask(&mut two, mask);
        assert_eq!(two, vec![0xAB ^ 0x12, 0xCD ^ 0x34]);

        // Test three bytes
        let mut three = vec![0xAB, 0xCD, 0xEF];
        apply_mask(&mut three, mask);
        assert_eq!(three, vec![0xAB ^ 0x12, 0xCD ^ 0x34, 0xEF ^ 0x56]);
    }

    #[test]

    fn test_mask_large_buffer() {
        // Test with a large buffer to exercise the word-aligned path
        let mask = [0x01, 0x02, 0x03, 0x04];
        let size = 10000;
        let mut data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        let original = data.clone();

        apply_mask(&mut data, mask);

        // Verify every byte is correctly masked
        for (i, &byte) in data.iter().enumerate() {
            let expected = original[i] ^ mask[i % 4];
            assert_eq!(byte, expected, "Mismatch at index {}", i);
        }
    }

    #[test]

    fn test_mask_alignment() {
        // Test that masking works correctly with different alignments
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];

        // Create a buffer with extra padding to test different alignments
        let mut buffer = vec![0u8; 20];
        #[expect(
            clippy::needless_range_loop,
            reason = "the index is needed to fill the buffer"
        )]
        for i in 0..buffer.len() {
            buffer[i] = i as u8;
        }

        // Test masking at different offsets
        for offset in 0..4 {
            let mut test_buf = buffer.clone();
            let original_slice = test_buf[offset..].to_vec();

            apply_mask(&mut test_buf[offset..], mask);

            // Verify masking is correct
            for (i, &byte) in test_buf[offset..].iter().enumerate() {
                let expected = original_slice[i] ^ mask[i % 4];
                assert_eq!(byte, expected, "Alignment {} failed at index {}", offset, i);
            }

            // Verify double masking restores original
            apply_mask(&mut test_buf[offset..], mask);
            assert_eq!(&test_buf[offset..], &original_slice[..]);
        }
    }

    #[test]

    fn test_mask_fast_matches_fallback() {
        // Comprehensive test that fast and fallback produce identical results
        let masks = [
            [0x00, 0x00, 0x00, 0x00],
            [0xFF, 0xFF, 0xFF, 0xFF],
            [0x12, 0x34, 0x56, 0x78],
            [0xAA, 0xBB, 0xCC, 0xDD],
            [0x01, 0x23, 0x45, 0x67],
        ];

        for mask in masks {
            for size in 0..=200 {
                let data: Vec<u8> = (0..size).map(|i| (i * 7) as u8).collect();

                let mut fallback_result = data.clone();
                apply_mask_fallback(&mut fallback_result, mask);

                let mut fast32_result = data.clone();
                apply_mask_fast32(&mut fast32_result, mask);

                let mut fast64_result = data.clone();
                apply_mask_fast64(&mut fast64_result, mask);

                assert_eq!(
                    fallback_result, fast32_result,
                    "fast32 mismatch for mask {:?} with size {}",
                    mask, size
                );

                assert_eq!(
                    fallback_result, fast64_result,
                    "fast64 mismatch for mask {:?} with size {}",
                    mask, size
                );
            }
        }
    }

    #[test]

    fn test_mask_endianness() {
        // Test that demonstrates endianness handling in fast path
        let mask = [0x11, 0x22, 0x33, 0x44];
        let data = vec![0xFF; 16]; // Nice aligned size

        let mut fallback = data.clone();
        apply_mask_fallback(&mut fallback, mask);

        let mut fast = data.clone();
        apply_mask_fast32(&mut fast, mask);

        // Both should produce identical results regardless of endianness
        assert_eq!(fallback, fast);

        // Verify the pattern repeats correctly
        for i in 0..fallback.len() {
            assert_eq!(fallback[i], 0xFF ^ mask[i % 4]);
        }
    }

    #[test]
    fn test_fast_mask_bytes_varies_across_calls() {
        use std::collections::HashSet;

        // Deterministic count assertion (no probability-based flake): 1000
        // draws from a seeded PRNG must yield overwhelmingly distinct keys.
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            seen.insert(fast_mask_bytes());
        }
        assert!(
            seen.len() > 900,
            "expected more than 900 distinct masks, got {}",
            seen.len()
        );
    }

    #[test]
    fn mask_key_comes_from_a_cryptographic_source() {
        // The masking key must be unpredictable (RFC 6455 §10.3): a
        // non-cryptographic PRNG can be state-recovered from observed
        // frames, which would let an attacker predict the next keys.
        // A ChaCha12-based CSPRNG produces 32 bytes of state per draw, so
        // consecutive keys are independent.
        let mut keys = Vec::with_capacity(64);
        for _ in 0..64 {
            keys.push(fast_mask_bytes());
        }
        // No key may repeat within the sample.
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "masking keys must not repeat");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_fast_mask_frame_encode_decode_roundtrip() {
        use bytes::BytesMut;
        use tokio_util::codec::{Decoder as _, Encoder as _};

        use crate::{
            Role,
            codec::{Decoder, Encoder},
            frame::{Frame, OpCode},
        };

        // The encoder assigns a fast-mask key per frame; decoding must recover
        // the exact payload, proving mask semantics are preserved.
        let frame = Frame::text("hello fast mask");
        let mut encoder = Encoder::new(Role::Client);
        let mut buf = BytesMut::new();
        encoder.encode(frame, &mut buf).unwrap();

        let mut decoder = Decoder::new(Role::Server, 1 << 20);
        let decoded = decoder.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.opcode(), OpCode::Text);
        assert_eq!(&decoded.into_payload()[..], b"hello fast mask");
    }
}

#[cfg(all(test, feature = "simd-mask"))]
mod simd_tests {
    use super::*;

    fn deterministic(len: usize) -> Vec<u8> {
        (0..len)
            .map(|index| index.wrapping_mul(31).wrapping_add(17) as u8)
            .collect()
    }

    const MASKS: [[u8; 4]; 4] = [
        [0x6d, 0xb6, 0xb2, 0x80],
        [0x00, 0x00, 0x00, 0x00],
        [0xff, 0xff, 0xff, 0xff],
        [0x01, 0x02, 0x03, 0x04],
    ];

    #[test]
    fn vector_path_matches_scalar_for_every_size_and_alignment() {
        for len in 0..=320_usize {
            let base = deterministic(len + 8);
            for offset in 0..=7_usize {
                for mask in MASKS {
                    let mut expected = base.clone();
                    apply_mask_fallback(&mut expected[offset..], mask);

                    let mut actual = base.clone();
                    if !try_apply_mask_simd(&mut actual[offset..], mask) {
                        // x86_64 host without AVX2: the dispatcher must fall
                        // back to scalar, which the other tests cover.
                        continue;
                    }
                    assert_eq!(actual, expected, "len={len} offset={offset} mask={mask:?}");
                }
            }
        }
    }

    #[test]
    fn vector_path_is_involutive() {
        let mask = [0x12, 0x34, 0x56, 0x78];
        for len in [64, 65, 96, 127, 128, 129, 1024, 65_536] {
            let original = deterministic(len);
            let mut data = original.clone();
            if !try_apply_mask_simd(&mut data, mask) {
                continue;
            }
            assert_ne!(data, original, "len={len}");
            assert!(try_apply_mask_simd(&mut data, mask));
            assert_eq!(data, original, "len={len}");
        }
    }

    #[test]
    fn dispatcher_matches_scalar_on_both_sides_of_the_threshold() {
        for len in [
            0, 1, 15, 63, 64, 65, 100, 128, 129, 256, 4096, 65_535, 65_536, 65_537,
        ] {
            let base = deterministic(len);
            for mask in MASKS {
                let mut expected = base.clone();
                apply_mask_fallback(&mut expected, mask);

                let mut actual = base.clone();
                apply_mask(&mut actual, mask);
                assert_eq!(actual, expected, "len={len} mask={mask:?}");
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn neon_path_is_selected_on_aarch64() {
        let mut buf = deterministic(64);
        assert!(try_apply_mask_simd(&mut buf, MASKS[0]));
    }
}
