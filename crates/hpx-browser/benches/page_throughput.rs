use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hpx_browser::html_parser::parse_html;

fn bench_parse_html(c: &mut Criterion) {
    let small = "<html><head><title>t</title></head><body><p>hello</p></body></html>";
    let medium = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<p>line</p>".repeat(500)
    );
    let large = format!(
        "<html><head><title>test</title></head><body>{}</body></html>",
        "<div class=\"item\"><span>content</span><a href=\"#\">link</a></div>".repeat(2000)
    );

    let mut group = c.benchmark_group("parse_html");
    for (name, html) in [
        ("small", small),
        ("medium", medium.as_str()),
        ("large", large.as_str()),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), html, |b, html| {
            b.iter(|| parse_html(html));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse_html);
criterion_main!(benches);
