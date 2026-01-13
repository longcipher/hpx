use std::time::Duration;

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

pub fn init_metrics(endpoint: &str) -> SdkMeterProvider {
    // Create the OTLP metrics exporter
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .expect("Failed to create OTLP metrics exporter");

    // Create a periodic reader with 15-second interval
    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(15))
        .build();

    // Build the meter provider
    let provider = SdkMeterProvider::builder().with_reader(reader).build();

    // Set as global meter provider
    global::set_meter_provider(provider.clone());

    provider
}
