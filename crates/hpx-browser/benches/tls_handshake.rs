use criterion::{Criterion, criterion_group, criterion_main};
use hpx_browser::tls::DeviceFingerprint;

fn bench_fingerprint_build(c: &mut Criterion) {
    // ponytail: actual TLS handshake requires a live server; benchmarking
    // fingerprint construction as a proxy for per-connection overhead.
    let mut group = c.benchmark_group("tls");
    group.bench_function("fingerprint_chrome", |b| {
        b.iter(DeviceFingerprint::chrome_147);
    });
    group.bench_function("fingerprint_safari", |b| {
        b.iter(DeviceFingerprint::safari_ios_18);
    });
    group.bench_function("fingerprint_firefox", |b| {
        b.iter(DeviceFingerprint::firefox_135);
    });
    group.bench_function("to_tls_options", |b| {
        let fp = DeviceFingerprint::chrome_147();
        b.iter(|| fp.to_tls_options());
    });
    group.finish();
}

criterion_group!(benches, bench_fingerprint_build);
criterion_main!(benches);
