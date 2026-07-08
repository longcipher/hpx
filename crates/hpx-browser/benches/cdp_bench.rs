#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use hpx_browser::{
    cdp_client::cdp::{CdpClient, CdpMessage},
    har::HarCapture,
};
use tokio::sync::broadcast;

fn bench_cdp_message_routing(c: &mut Criterion) {
    let mut group = c.benchmark_group("cdp_message_routing");

    group.bench_function("dispatch_event_no_pending", |b| {
        let client = CdpClient::new();
        let mut _rx = client.subscribe_events();
        b.iter(|| {
            client.dispatch_message(CdpMessage {
                id: None,
                method: Some("Page.loadEventFired".to_string()),
                params: Some(serde_json::json!({})),
                result: None,
                error: None,
                session_id: None,
            });
        });
    });

    group.bench_function("subscribe_and_dispatch_event", |b| {
        let client = CdpClient::new();
        let mut rx = client.subscribe_events();
        b.iter(|| {
            client.dispatch_message(CdpMessage {
                id: None,
                method: Some("Network.requestWillBeSent".to_string()),
                params: Some(serde_json::json!({"requestId": "r1"})),
                result: None,
                error: None,
                session_id: None,
            });
            let _ = rx.try_recv();
        });
    });

    group.finish();
}

fn bench_har_capture_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("har_capture");

    group.bench_function("capture_via_broadcast_10_requests", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (tx, rx) = broadcast::channel::<CdpMessage>(64);
                let mut capture = HarCapture::new(rx);
                for i in 0..10 {
                    let _ = tx.send(CdpMessage {
                        id: None,
                        method: Some("Network.requestWillBeSent".to_string()),
                        params: Some(serde_json::json!({
                            "requestId": format!("req-{i}"),
                            "timestamp": 1705321845.0,
                            "request": {
                                "method": "GET",
                                "url": format!("https://example.com/page/{i}"),
                                "httpVersion": "HTTP/1.1",
                                "headers": {}
                            }
                        })),
                        result: None,
                        error: None,
                        session_id: None,
                    });
                    let _ = tx.send(CdpMessage {
                        id: None,
                        method: Some("Network.responseReceived".to_string()),
                        params: Some(serde_json::json!({
                            "requestId": format!("req-{i}"),
                            "response": {
                                "status": 200,
                                "statusText": "OK",
                                "headers": { "content-type": "text/html" },
                                "bodySize": 1024
                            }
                        })),
                        result: None,
                        error: None,
                        session_id: None,
                    });
                    let _ = tx.send(CdpMessage {
                        id: None,
                        method: Some("Network.loadingFinished".to_string()),
                        params: Some(serde_json::json!({
                            "requestId": format!("req-{i}"),
                            "encodedDataLength": 1024
                        })),
                        result: None,
                        error: None,
                        session_id: None,
                    });
                }
                drop(tx);
                capture.stop().await.expect("stop");
            });
        });
    });

    group.bench_function("har_archive_serialization", |b| {
        let archive = rt.block_on(async {
            let (tx, rx) = broadcast::channel::<CdpMessage>(64);
            let mut capture = HarCapture::new(rx);
            for i in 0..100 {
                let _ = tx.send(CdpMessage {
                    id: None,
                    method: Some("Network.requestWillBeSent".to_string()),
                    params: Some(serde_json::json!({
                        "requestId": format!("req-{i}"),
                        "timestamp": 1705321845.0,
                        "request": {
                            "method": "GET",
                            "url": format!("https://example.com/page/{i}"),
                            "httpVersion": "HTTP/1.1",
                            "headers": {}
                        }
                    })),
                    result: None,
                    error: None,
                    session_id: None,
                });
                let _ = tx.send(CdpMessage {
                    id: None,
                    method: Some("Network.responseReceived".to_string()),
                    params: Some(serde_json::json!({
                        "requestId": format!("req-{i}"),
                        "response": {
                            "status": 200,
                            "statusText": "OK",
                            "headers": { "content-type": "text/html" },
                            "bodySize": 1024
                        }
                    })),
                    result: None,
                    error: None,
                    session_id: None,
                });
                let _ = tx.send(CdpMessage {
                    id: None,
                    method: Some("Network.loadingFinished".to_string()),
                    params: Some(serde_json::json!({
                        "requestId": format!("req-{i}"),
                        "encodedDataLength": 1024
                    })),
                    result: None,
                    error: None,
                    session_id: None,
                });
            }
            drop(tx);
            capture.stop().await.expect("stop")
        });

        b.iter(|| {
            serde_json::to_string(&archive).expect("serialize");
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cdp_message_routing,
    bench_har_capture_overhead
);
criterion_main!(benches);
