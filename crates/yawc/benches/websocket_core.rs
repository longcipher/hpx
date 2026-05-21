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
    #[cfg(feature = "hotpath")]
    let _hotpath = hotpath::HotpathGuardBuilder::new("yawc_websocket_core_bench")
        .sections(vec![hotpath::Section::FunctionsTiming])
        .percentiles(&[50.0, 95.0, 99.0])
        .functions_limit(32)
        .build();

    bench_mask(c);
    bench_encode(c);
    bench_decode(c);
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
