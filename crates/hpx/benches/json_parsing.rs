//! Benchmarks for JSON parsing performance.
//!
//! Compares performance of `serde_json` vs `simd-json` for various JSON payloads.
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
use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

/// A simple JSON structure for benchmarking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SimpleData {
    id: u64,
    name: String,
    active: bool,
    score: f64,
}

/// A nested JSON structure for benchmarking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct NestedData {
    id: u64,
    name: String,
    metadata: Metadata,
    tags: Vec<String>,
    items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Metadata {
    created_at: String,
    updated_at: String,
    version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Item {
    id: u64,
    value: f64,
    description: String,
}

/// Generate a simple JSON payload
fn generate_simple_json() -> String {
    serde_json::to_string(&SimpleData {
        id: 12345,
        name: "benchmark_test".to_string(),
        active: true,
        score: 98.765,
    })
    .unwrap()
}

/// Generate a nested JSON payload with N items
fn generate_nested_json(item_count: usize) -> String {
    let items: Vec<Item> = (0..item_count)
        .map(|i| Item {
            id: i as u64,
            value: i as f64 * 1.5,
            description: format!("Item description number {}", i),
        })
        .collect();

    let tags: Vec<String> = (0..10).map(|i| format!("tag_{}", i)).collect();

    serde_json::to_string(&NestedData {
        id: 1,
        name: "nested_benchmark".to_string(),
        metadata: Metadata {
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            version: 1,
        },
        tags,
        items,
    })
    .unwrap()
}

/// Benchmark serde_json deserialization
fn bench_serde_json_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_deserialize");

    // Simple JSON
    let simple_json = generate_simple_json();
    let simple_bytes = Bytes::from(simple_json.clone());
    group.throughput(Throughput::Bytes(simple_bytes.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("serde_json", "simple"),
        &simple_bytes,
        |b, bytes| {
            b.iter(|| {
                let _: SimpleData = serde_json::from_slice(bytes).unwrap();
            });
        },
    );

    #[cfg(feature = "simd-json")]
    group.bench_with_input(
        BenchmarkId::new("simd_json", "simple"),
        &simple_bytes,
        |b, bytes| {
            b.iter(|| {
                let mut vec = bytes.to_vec();
                let _: SimpleData = simd_json::from_slice(&mut vec).unwrap();
            });
        },
    );

    // Nested JSON with varying sizes
    for item_count in [10, 100, 1000].iter() {
        let nested_json = generate_nested_json(*item_count);
        let nested_bytes = Bytes::from(nested_json.clone());

        group.throughput(Throughput::Bytes(nested_bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("serde_json", format!("nested_{}", item_count)),
            &nested_bytes,
            |b, bytes| {
                b.iter(|| {
                    let _: NestedData = serde_json::from_slice(bytes).unwrap();
                });
            },
        );

        #[cfg(feature = "simd-json")]
        group.bench_with_input(
            BenchmarkId::new("simd_json", format!("nested_{}", item_count)),
            &nested_bytes,
            |b, bytes| {
                b.iter(|| {
                    let mut vec = bytes.to_vec();
                    let _: NestedData = simd_json::from_slice(&mut vec).unwrap();
                });
            },
        );
    }

    group.finish();
}

/// Benchmark JSON serialization
fn bench_json_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_serialize");

    let simple_data = SimpleData {
        id: 12345,
        name: "benchmark_test".to_string(),
        active: true,
        score: 98.765,
    };

    group.bench_function("serde_json/simple", |b| {
        b.iter(|| {
            let _json = serde_json::to_string(&simple_data).unwrap();
        });
    });

    #[cfg(feature = "simd-json")]
    group.bench_function("simd_json/simple", |b| {
        b.iter(|| {
            // simd-json serialization delegates to serde_json
            let _json = simd_json::to_string(&simple_data).unwrap();
        });
    });

    // Nested JSON with varying sizes
    for item_count in [10, 100, 1000].iter() {
        let items: Vec<Item> = (0..*item_count)
            .map(|i| Item {
                id: i as u64,
                value: i as f64 * 1.5,
                description: format!("Item description number {}", i),
            })
            .collect();

        let tags: Vec<String> = (0..10).map(|i| format!("tag_{}", i)).collect();

        let nested_data = NestedData {
            id: 1,
            name: "nested_benchmark".to_string(),
            metadata: Metadata {
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
                version: 1,
            },
            tags,
            items,
        };

        group.bench_function(format!("serde_json/nested_{}", item_count), |b| {
            b.iter(|| {
                let _json = serde_json::to_string(&nested_data).unwrap();
            });
        });

        #[cfg(feature = "simd-json")]
        group.bench_function(format!("simd_json/nested_{}", item_count), |b| {
            b.iter(|| {
                let _json = simd_json::to_string(&nested_data).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_serde_json_deserialize, bench_json_serialize);
criterion_main!(benches);
