use std::hint::black_box;

use bytes::{Bytes, BytesMut};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hpx_yawc::{
    Role,
    codec::{Decoder as WsDecoder, Encoder as WsEncoder},
    frame::Frame,
};
use tokio_util::codec::{Decoder as _, Encoder as _};

const MASK: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
const MAX_PAYLOAD_SIZE: usize = 32 * 1024 * 1024;
const SIZES: [usize; 5] = [16, 125, 126, 1024, 64 * 1024];

// =============================================================================
// Experimental SIMD masking implementation (ported from tokio-websockets).
//
// This module is intentionally inlined in the benchmark file so we can measure
// the speedup of AVX2/SSE2/NEON paths against yawc's current scalar paths
// BEFORE committing to a production change in `src/mask.rs`.
//
// Key correctness invariant (copied from tokio-websockets): the framing key
// cycles every 4 bytes. When `align_to_mut` splits the input into
// (prefix, aligned, suffix), the prefix/suffix may not be 4-byte aligned, so
// the key MUST be rotated in-place after processing each non-empty prefix/
// suffix chunk. SIMD `aligned` chunks are always a multiple of 16/32 bytes
// (divisible by 4), so no rotation is needed after the aligned loop.
// =============================================================================
mod simd_experimental {
    /// Rotates the mask in-place by `offset` bytes (mirrors the 4-byte cycle).
    fn rotate_mask(key: &mut [u8; 4], offset: usize) {
        *key = u32::from_be_bytes(*key)
            .rotate_left((offset % key.len()) as u32 * u8::BITS)
            .to_be_bytes();
    }

    /// Byte-at-a-time mask, rotates `key` after processing so the next chunk
    /// (aligned or suffix) starts at the correct mask offset.
    fn one_byte_at_once(key: &mut [u8; 4], input: &mut [u8]) {
        for (index, byte) in input.iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        rotate_mask(key, input.len());
    }

    /// AVX2 path: 32 bytes per cycle.
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn frame_avx2(key: &mut [u8; 4], input: &mut [u8]) {
        use std::arch::x86_64::{__m256i, _mm256_set1_epi32, _mm256_xor_si256};
        // SAFETY: `align_to_mut` produces three disjoint sub-slices covering
        // the whole input; the aligned middle is properly aligned for `__m256i`.
        // `one_byte_at_once` rotates `key` so SIMD `aligned` starts at offset 0.
        unsafe {
            let (prefix, aligned, suffix) = input.align_to_mut::<__m256i>();
            if !prefix.is_empty() {
                one_byte_at_once(key, prefix);
            }
            if !aligned.is_empty() {
                let mask = _mm256_set1_epi32(i32::from_ne_bytes(*key));
                for block in aligned {
                    *block = _mm256_xor_si256(*block, mask);
                }
                // 32 % 4 == 0, no rotate needed.
            }
            if !suffix.is_empty() {
                one_byte_at_once(key, suffix);
            }
        }
    }

    /// SSE2 path: 16 bytes per cycle.
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse2")]
    unsafe fn frame_sse2(key: &mut [u8; 4], input: &mut [u8]) {
        use std::arch::x86_64::{__m128i, _mm_set1_epi32, _mm_xor_si128};
        // SAFETY: see `frame_avx2`.
        unsafe {
            let (prefix, aligned, suffix) = input.align_to_mut::<__m128i>();
            if !prefix.is_empty() {
                one_byte_at_once(key, prefix);
            }
            if !aligned.is_empty() {
                let mask = _mm_set1_epi32(i32::from_ne_bytes(*key));
                for block in aligned {
                    *block = _mm_xor_si128(*block, mask);
                }
                // 16 % 4 == 0, no rotate needed.
            }
            if !suffix.is_empty() {
                one_byte_at_once(key, suffix);
            }
        }
    }

    /// NEON path: 16 bytes per cycle.
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn frame_neon(key: &mut [u8; 4], input: &mut [u8]) {
        use std::arch::aarch64::{uint8x16_t, veorq_u8, vld1q_dup_s32, vreinterpretq_u8_s32};
        // SAFETY: see `frame_avx2`. All unsafe calls below are wrapped in an
        // explicit unsafe block per Rust 2024 `unsafe_op_in_unsafe_fn`.
        unsafe {
            let (prefix, aligned, suffix) = input.align_to_mut::<uint8x16_t>();
            if !prefix.is_empty() {
                one_byte_at_once(key, prefix);
            }
            if !aligned.is_empty() {
                // SAFETY: `key` is a 4-byte array, reading as i32 is sound.
                let k = key.as_ptr().cast::<i32>().read_unaligned();
                let mask = vreinterpretq_u8_s32(vld1q_dup_s32(&raw const k));
                for block in aligned {
                    *block = veorq_u8(*block, mask);
                }
                // 16 % 4 == 0, no rotate needed.
            }
            if !suffix.is_empty() {
                one_byte_at_once(key, suffix);
            }
        }
    }

    /// Scalar fallback: 8 bytes per cycle using u64.
    fn fallback_frame(key: &mut [u8; 4], input: &mut [u8]) {
        // SAFETY: `align_to_mut::<u64>` produces three disjoint sub-slices
        // covering the whole input; the middle is properly aligned.
        let (prefix, aligned, suffix) = unsafe { input.align_to_mut::<u64>() };
        if !prefix.is_empty() {
            one_byte_at_once(key, prefix);
        }
        if !aligned.is_empty() {
            let masking_key = u64::from(u32::from_ne_bytes(*key));
            let mask = (masking_key << u32::BITS) | masking_key;
            for block in aligned {
                *block ^= mask;
            }
            // 8 % 4 == 0, no rotate needed.
        }
        if !suffix.is_empty() {
            one_byte_at_once(key, suffix);
        }
    }

    /// Runtime-dispatched SIMD mask, mirrors tokio-websockets' public API.
    ///
    /// NOTE: takes `mask: [u8; 4]` by value (yawc API style) but internally
    /// mutates a local copy to track rotation across alignment boundaries.
    #[inline]
    pub fn apply_mask_simd(buf: &mut [u8], mask: [u8; 4]) {
        let mut key = mask;
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                return unsafe { frame_avx2(&mut key, buf) };
            }
            if std::arch::is_x86_feature_detected!("sse2") {
                return unsafe { frame_sse2(&mut key, buf) };
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") {
                return unsafe { frame_neon(&mut key, buf) };
            }
        }
        fallback_frame(&mut key, buf);
    }

    /// Naive u64 loop WITHOUT `align_to_mut`, to test whether LLVM
    /// auto-vectorizes the simple `*word ^= mask` loop into NEON instructions.
    ///
    /// Assumes `buf` is 8-byte aligned (true for `Vec<u8>` from heap alloc on
    /// most platforms). Does NOT handle misaligned inputs — for benchmark
    /// comparison only, NOT for production.
    pub fn naive_u64_loop(buf: &mut [u8], mask: [u8; 4]) {
        let mask_u32 = u32::from_ne_bytes(mask);
        let mask_u64 = (mask_u32 as u64) | ((mask_u32 as u64) << 32);

        // SAFETY: For benchmark only — `buf` from `payload()` is heap-allocated
        // and 16-byte aligned on aarch64/macOS. We reinterpret the leading
        // `buf.len() / 8 * 8` bytes as `&mut [u64]`. This is sound ONLY when
        // `buf.as_ptr()` is 8-byte aligned, which holds for our bench inputs.
        let words_len = buf.len() / 8;
        let words: &mut [u64] =
            unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u64>(), words_len) };
        for word in words.iter_mut() {
            *word ^= mask_u64;
        }
        // Handle the (rare) tail bytes with byte-at-a-time.
        // tail_start = words_len * 8 is always a multiple of 8, so (tail_start % 4) == 0,
        // meaning the mask cycle is at offset 0 — we can use the original mask directly.
        let tail_start = words_len * 8;
        for (i, b) in buf[tail_start..].iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }

    /// Safe variant of `naive_u64_loop` that uses `read_unaligned`/`write_unaligned`
    /// so it is sound for ANY input alignment, not just 8-byte-aligned buffers.
    ///
    /// Hypothesis: on x86_64 / aarch64, unaligned loads/stores are essentially the
    /// same cost as aligned ones, so this should match `naive_u64_loop` while being
    /// safe for production use (e.g. when `buf` is a sub-slice of a larger buffer
    /// starting at an offset that isn't 8-byte aligned).
    pub fn safe_unaligned_u64(buf: &mut [u8], mask: [u8; 4]) {
        let mask_u32 = u32::from_ne_bytes(mask);
        let mask_u64 = (mask_u32 as u64) | ((mask_u32 as u64) << 32);

        let chunks_len = buf.len() / 8;
        let ptr = buf.as_mut_ptr();
        for i in 0..chunks_len {
            let offset = i * 8;
            // SAFETY: `offset + 8 <= buf.len()` because `chunks_len = buf.len() / 8`.
            // `read_unaligned`/`write_unaligned` are sound for any alignment.
            let word = unsafe { ptr.add(offset).cast::<u64>().read_unaligned() };
            unsafe {
                ptr.add(offset)
                    .cast::<u64>()
                    .write_unaligned(word ^ mask_u64)
            };
        }

        // Tail: starts at `chunks_len * 8`, which is a multiple of 8, so the mask
        // cycle (period 4) is back at offset 0 — use the original mask directly.
        let tail_start = chunks_len * 8;
        for (i, b) in buf[tail_start..].iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }

    /// Hybrid variant: branches once on `is_aligned_to(8)` and uses the aligned
    /// `&mut [u64]` slice path when possible (matching `naive_u64_loop`), and
    /// falls back to `read_unaligned`/`write_unaligned` otherwise.
    ///
    /// Hypothesis: most production callers pass 8/16-byte-aligned buffers (Vec<u8>,
    /// BytesMut), so the aligned path will dominate. The single branch is
    /// predictable and cheaper than the per-call `align_to_mut` overhead.
    pub fn hybrid_aligned_check(buf: &mut [u8], mask: [u8; 4]) {
        let mask_u32 = u32::from_ne_bytes(mask);
        let mask_u64 = (mask_u32 as u64) | ((mask_u32 as u64) << 32);

        let chunks_len = buf.len() / 8;
        let (head, tail) = buf.split_at_mut(chunks_len * 8);

        if (head.as_ptr() as usize).is_multiple_of(8) {
            // SAFETY: we just verified `head.as_ptr()` is 8-byte aligned, and
            // `head.len() == chunks_len * 8`, so the slice has room for `chunks_len`
            // `u64` values. Aliasing is fine because `head` is borrowed exclusively.
            let words: &mut [u64] = unsafe {
                std::slice::from_raw_parts_mut(head.as_mut_ptr().cast::<u64>(), chunks_len)
            };
            for word in words.iter_mut() {
                *word ^= mask_u64;
            }
        } else {
            let ptr = head.as_mut_ptr();
            for i in 0..chunks_len {
                let offset = i * 8;
                // SAFETY: see `safe_unaligned_u64`.
                let word = unsafe { ptr.add(offset).cast::<u64>().read_unaligned() };
                unsafe {
                    ptr.add(offset)
                        .cast::<u64>()
                        .write_unaligned(word ^ mask_u64)
                };
            }
        }

        // Tail: starts at `chunks_len * 8`, mask cycle is at offset 0.
        for (i, b) in tail.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }

    /// Sanity check that SIMD output matches a byte-at-a-time reference.
    /// Called at the start of `benchmark()` so `cargo bench` fails fast if
    /// the experimental SIMD code is incorrect.
    pub fn correctness_check() {
        fn reference_mask(buf: &mut [u8], mask: [u8; 4]) {
            for (i, b) in buf.iter_mut().enumerate() {
                *b ^= mask[i & 3];
            }
        }

        let mask = [0x6d, 0xb6, 0xb2, 0x80];
        for size in [
            0usize, 1, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 1024, 4096, 16384,
        ] {
            let original: Vec<u8> = (0..size)
                .map(|i| (i.wrapping_mul(31) & 0xff) as u8)
                .collect();

            for alignment_offset in 0..=3 {
                if size < alignment_offset {
                    continue;
                }

                // Test all candidate implementations against the reference.
                // The `safe_unaligned_u64` and `hybrid_aligned_check` variants
                // must work for ANY alignment (including non-8-byte-aligned
                // sub-slices), so we exercise the alignment_offset cases
                // (1, 2, 3) which produce unaligned pointers.
                #[allow(clippy::type_complexity)]
                let candidates: &[(&str, fn(&mut [u8], [u8; 4]))] = &[
                    ("apply_mask_simd", apply_mask_simd),
                    ("safe_unaligned_u64", safe_unaligned_u64),
                    ("hybrid_aligned_check", hybrid_aligned_check),
                ];

                for &(name, f) in candidates {
                    let mut buf = original.clone();
                    let mut ref_buf = original.clone();

                    f(&mut buf[alignment_offset..], mask);
                    reference_mask(&mut ref_buf[alignment_offset..], mask);

                    assert_eq!(
                        buf, ref_buf,
                        "{name}: size={size} alignment_offset={alignment_offset}"
                    );

                    // Apply twice to verify idempotent unmask
                    f(&mut buf[alignment_offset..], mask);
                    assert_eq!(
                        buf, original,
                        "{name}: double-mask size={size} offset={alignment_offset}"
                    );
                }
            }
        }
    }
}

fn payload(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| (index.wrapping_mul(31) & 0xff) as u8)
        .collect()
}

fn assume_ok<T, E>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(_) => std::process::abort(),
    }
}

fn assume_some<T>(value: Option<T>) -> T {
    match value {
        Some(value) => value,
        None => std::process::abort(),
    }
}

fn encode_frame(role: Role, frame: Frame) -> Vec<u8> {
    let mut encoder = WsEncoder::new(role);
    let mut dst = BytesMut::with_capacity(frame.payload().len() + 16);
    assume_ok(encoder.encode(frame, &mut dst));
    dst.freeze().to_vec()
}

fn bench_mask(c: &mut Criterion) {
    let mut group = c.benchmark_group("yawc/mask");

    for size in SIZES {
        group.throughput(Throughput::Bytes(size as u64));

        let mut apply_data = payload(size);
        group.bench_with_input(BenchmarkId::new("apply_mask", size), &size, |bench, _| {
            bench.iter(|| {
                hpx_yawc::mask::apply_mask(black_box(&mut apply_data), black_box(MASK));
            });
        });

        let mut fast32_data = payload(size);
        group.bench_with_input(BenchmarkId::new("fast32", size), &size, |bench, _| {
            bench.iter(|| {
                hpx_yawc::mask::apply_mask_fast32(black_box(&mut fast32_data), black_box(MASK));
            });
        });

        let mut fast64_data = payload(size);
        group.bench_with_input(BenchmarkId::new("fast64", size), &size, |bench, _| {
            bench.iter(|| {
                hpx_yawc::mask::apply_mask_fast64(black_box(&mut fast64_data), black_box(MASK));
            });
        });
    }

    group.finish();
}

/// Head-to-head comparison of yawc's scalar mask paths vs an experimental
/// runtime-dispatched SIMD implementation (AVX2 / SSE2 / NEON).
///
/// Hypothesis under test: for payloads >= ~1 KiB, SIMD paths should be at
/// least 2x faster than the current scalar `apply_mask_fast64`.
fn bench_mask_simd_vs_scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("yawc/mask-simd-vs-scalar");
    // Mask is a micro-operation; 2s × 50 samples gives stable numbers without
    // spending 5s × 100 samples per (size, path) combination.
    group.sample_size(50);
    group.measurement_time(std::time::Duration::from_secs(2));

    for size in SIZES {
        group.throughput(Throughput::Bytes(size as u64));

        let mut scalar_fast64 = payload(size);
        group.bench_with_input(
            BenchmarkId::new("scalar_fast64", size),
            &size,
            |bench, _| {
                bench.iter(|| {
                    hpx_yawc::mask::apply_mask_fast64(
                        black_box(&mut scalar_fast64),
                        black_box(MASK),
                    );
                });
            },
        );

        let mut scalar_fast32 = payload(size);
        group.bench_with_input(
            BenchmarkId::new("scalar_fast32", size),
            &size,
            |bench, _| {
                bench.iter(|| {
                    hpx_yawc::mask::apply_mask_fast32(
                        black_box(&mut scalar_fast32),
                        black_box(MASK),
                    );
                });
            },
        );

        let mut simd_data = payload(size);
        group.bench_with_input(BenchmarkId::new("simd_runtime", size), &size, |bench, _| {
            bench.iter(|| {
                simd_experimental::apply_mask_simd(black_box(&mut simd_data), black_box(MASK));
            });
        });

        // Naive u64 loop WITHOUT align_to_mut, to test whether LLVM
        // auto-vectorizes the simple loop into NEON. If this matches or beats
        // `scalar_fast64`, auto-vectorization is confirmed as the reason SIMD
        // doesn't help.
        let mut naive_u64_data = payload(size);
        group.bench_with_input(
            BenchmarkId::new("naive_u64_loop", size),
            &size,
            |bench, _| {
                bench.iter(|| {
                    simd_experimental::naive_u64_loop(
                        black_box(&mut naive_u64_data),
                        black_box(MASK),
                    );
                });
            },
        );

        // Safe variant: uses `read_unaligned`/`write_unaligned` so it works
        // for any alignment. Should match `naive_u64_loop` on aligned inputs.
        let mut safe_unaligned_data = payload(size);
        group.bench_with_input(
            BenchmarkId::new("safe_unaligned_u64", size),
            &size,
            |bench, _| {
                bench.iter(|| {
                    simd_experimental::safe_unaligned_u64(
                        black_box(&mut safe_unaligned_data),
                        black_box(MASK),
                    );
                });
            },
        );

        // Hybrid: one branch on `is_aligned_to(8)`, then aligned slice path
        // or unaligned read/write path. Should match `naive_u64_loop` on
        // aligned inputs and `safe_unaligned_u64` on misaligned inputs.
        let mut hybrid_data = payload(size);
        group.bench_with_input(
            BenchmarkId::new("hybrid_aligned_check", size),
            &size,
            |bench, _| {
                bench.iter(|| {
                    simd_experimental::hybrid_aligned_check(
                        black_box(&mut hybrid_data),
                        black_box(MASK),
                    );
                });
            },
        );
    }

    group.finish();
}

/// Misaligned-input benchmark: simulates real-world WebSocket frame decoding
/// where the payload starts at a non-8-byte-aligned offset (e.g. a 6-byte
/// header with no mask, or a 10-byte header with mask).
///
/// WS frame header sizes:
/// - 6 bytes: no mask, payload len < 126  → mask offset 6, 6 % 8 = 6 (unaligned)
/// - 8 bytes: no mask, payload len >= 126 → mask offset 8, 8 % 8 = 0 (aligned)
/// - 10 bytes: with mask, payload len < 126 → mask offset 10, 10 % 8 = 2 (unaligned)
/// - 14 bytes: with mask, payload len >= 126 → mask offset 14, 14 % 8 = 6 (unaligned)
///
/// So in 3 of 4 cases the payload buffer passed to `apply_mask_*` is NOT
/// 8-byte aligned. This benchmark measures how each variant behaves when
/// the input is intentionally misaligned by 1, 2, or 3 bytes.
fn bench_mask_misaligned(c: &mut Criterion) {
    let mut group = c.benchmark_group("yawc/mask-misaligned");
    group.sample_size(50);
    group.measurement_time(std::time::Duration::from_secs(2));

    // Use a single representative payload size: large enough to exercise the
    // u64 loop meaningfully, small enough to keep the benchmark fast.
    const SIZE: usize = 1024;
    group.throughput(Throughput::Bytes(SIZE as u64));

    // We allocate `SIZE + 3` bytes and slice `[offset..offset+SIZE]` to get a
    // SIZE-byte buffer starting at `offset` bytes into the allocation. This
    // produces pointers with alignment offset 0, 1, 2, 3 (mod 4) — and on a
    // 16-byte-aligned allocation, also mod 8 = 0, 1, 2, 3.
    for offset in 0..=3u8 {
        for (variant_name, variant_fn) in [
            (
                "scalar_fast64",
                hpx_yawc::mask::apply_mask_fast64 as fn(&mut [u8], [u8; 4]),
            ),
            (
                "scalar_fast32",
                hpx_yawc::mask::apply_mask_fast32 as fn(&mut [u8], [u8; 4]),
            ),
            (
                "safe_unaligned_u64",
                simd_experimental::safe_unaligned_u64 as fn(&mut [u8], [u8; 4]),
            ),
            (
                "hybrid_aligned_check",
                simd_experimental::hybrid_aligned_check as fn(&mut [u8], [u8; 4]),
            ),
            // NOTE: `naive_u64_loop` is intentionally OMITTED from the misaligned
            // benchmark — calling it on a non-8-byte-aligned buffer is UB.
        ] {
            // Allocate fresh per-bench so each measurement starts from a known state.
            let mut buf: Vec<u8> = (0..(SIZE + offset as usize))
                .map(|i| (i.wrapping_mul(31) & 0xff) as u8)
                .collect();

            group.bench_with_input(
                BenchmarkId::new(format!("{variant_name}/off{offset}"), SIZE),
                &offset,
                |bench, _| {
                    bench.iter(|| {
                        let slice: &mut [u8] = &mut buf[offset as usize..offset as usize + SIZE];
                        variant_fn(black_box(slice), black_box(MASK));
                    });
                },
            );
        }
    }

    group.finish();
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("yawc/encode");

    for size in SIZES {
        let bytes = Bytes::from(payload(size));
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("client_pre_masked", size),
            &size,
            |bench, _| {
                let mut encoder = WsEncoder::new(Role::Client);
                let mut dst = BytesMut::with_capacity(size + 16);
                bench.iter(|| {
                    dst.clear();
                    let frame = Frame::binary(bytes.clone()).with_mask(MASK);
                    assume_ok(encoder.encode(black_box(frame), black_box(&mut dst)));
                    black_box(dst.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("client_random_mask", size),
            &size,
            |bench, _| {
                let mut encoder = WsEncoder::new(Role::Client);
                let mut dst = BytesMut::with_capacity(size + 16);
                bench.iter(|| {
                    dst.clear();
                    let frame = Frame::binary(bytes.clone());
                    assume_ok(encoder.encode(black_box(frame), black_box(&mut dst)));
                    black_box(dst.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("server_unmasked", size),
            &size,
            |bench, _| {
                let mut encoder = WsEncoder::new(Role::Server);
                let mut dst = BytesMut::with_capacity(size + 16);
                bench.iter(|| {
                    dst.clear();
                    let frame = Frame::binary(bytes.clone());
                    assume_ok(encoder.encode(black_box(frame), black_box(&mut dst)));
                    black_box(dst.len());
                });
            },
        );
    }

    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("yawc/decode");

    for size in SIZES {
        let bytes = Bytes::from(payload(size));
        let masked = encode_frame(Role::Client, Frame::binary(bytes.clone()).with_mask(MASK));
        let unmasked = encode_frame(Role::Server, Frame::binary(bytes));
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("server_masked", size),
            &size,
            |bench, _| {
                bench.iter_batched(
                    || BytesMut::from(masked.as_slice()),
                    |mut src| {
                        let mut decoder = WsDecoder::new(Role::Server, MAX_PAYLOAD_SIZE);
                        let frame = assume_some(assume_ok(decoder.decode(black_box(&mut src))));
                        black_box(frame.payload().len());
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("client_unmasked", size),
            &size,
            |bench, _| {
                bench.iter_batched(
                    || BytesMut::from(unmasked.as_slice()),
                    |mut src| {
                        let mut decoder = WsDecoder::new(Role::Client, MAX_PAYLOAD_SIZE);
                        let frame = assume_some(assume_ok(decoder.decode(black_box(&mut src))));
                        black_box(frame.payload().len());
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn benchmark(c: &mut Criterion) {
    // Fail fast if the experimental SIMD mask produces wrong output.
    simd_experimental::correctness_check();

    #[cfg(feature = "hotpath")]
    let _hotpath = hotpath::HotpathGuardBuilder::new("yawc_websocket_core_bench")
        .sections(vec![hotpath::Section::FunctionsTiming])
        .percentiles(&[50.0, 95.0, 99.0])
        .functions_limit(32)
        .build();

    bench_mask(c);
    bench_mask_simd_vs_scalar(c);
    bench_mask_misaligned(c);
    bench_encode(c);
    bench_decode(c);
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
