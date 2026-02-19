use std::time::Duration;

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

use crate::error::{TransportError, TransportResult};

pub fn init_metrics(endpoint: &str) -> TransportResult<SdkMeterProvider> {
    // Create the OTLP metrics exporter
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| {
            TransportError::config(format!("Failed to create OTLP metrics exporter: {e}"))
        })?;

    // Create a periodic reader with 15-second interval
    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(15))
        .build();

    // Build the meter provider
    let provider = SdkMeterProvider::builder().with_reader(reader).build();

    // Set as global meter provider
    global::set_meter_provider(provider.clone());

    Ok(provider)
}
