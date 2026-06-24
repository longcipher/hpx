use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hpx_browser::challenge::engine_classify;

fn bench_engine_classify(c: &mut Criterion) {
    let clean = "<html><head><title>OK</title></head><body><p>Normal page</p></body></html>";
    let cf_challenge =
        "<html><body><div id=\"cf-browser-verification\">just a moment</div></body></html>";
    let thin = "<html><body>x</body></html>";
    let large_clean = format!(
        "<html><body>{}</body></html>",
        "<p>content</p>".repeat(10_000)
    );
    let large_challenge = format!(
        "<html><body>{}<div>just a moment</div>{}</body></html>",
        "<!--padding-->".repeat(5000),
        "<!--more-->".repeat(5000),
    );

    let mut group = c.benchmark_group("engine_classify");
    for (name, body) in [
        ("clean", clean.to_string()),
        ("cf_challenge", cf_challenge.to_string()),
        ("thin_body", thin.to_string()),
        ("large_clean", large_clean),
        ("large_challenge", large_challenge),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), &body, |b, body| {
            b.iter(|| engine_classify(body));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_engine_classify);
criterion_main!(benches);
