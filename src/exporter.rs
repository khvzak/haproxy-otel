use std::error::Error as StdError;

use opentelemetry_jaeger_propagator as opentelemetry_jaeger;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};
use opentelemetry_sdk::Resource;

#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) service_name: String,
    // Can be: "AlwaysOn", "SilentOn", "AlwaysOff", "ParentBased"
    pub(crate) sampler: Option<String>,
    // Can be: "jaeger", "zipkin"
    pub(crate) propagator: Option<String>,
    pub(crate) endpoint: Option<String>,
    // Can be: "binary" or "json"
    pub(crate) protocol: Option<String>,
}

pub fn init(options: Options) -> Result<(), Box<dyn StdError + Send + Sync + 'static>> {
    match options.propagator.as_deref() {
        None | Some("zipkin") => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_zipkin::Propagator::new());
        }
        Some("jaeger") => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
        }
        _ => {}
    }

    let mut exporter_builder = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint((options.endpoint.as_deref()).unwrap_or("http://localhost:4318/v1/trace"));
    match options.protocol.as_deref() {
        None | Some("binary") => {
            exporter_builder =
                exporter_builder.with_protocol(opentelemetry_otlp::Protocol::HttpBinary);
        }
        Some("json") => {
            exporter_builder =
                exporter_builder.with_protocol(opentelemetry_otlp::Protocol::HttpJson);
        }
        _ => {}
    }
    let exporter = exporter_builder.build()?;

    let processor =
        BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build();

    let mut tracer_provider_builder = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(
            Resource::builder_empty()
                .with_service_name(options.service_name)
                .build(),
        );

    match options.sampler.as_deref() {
        Some("AlwaysOn" | "SilentOn") => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOn);
        }
        Some("AlwaysOff") => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOff);
        }
        None | Some("ParentBased") => {
            // This is the default sampler
            tracer_provider_builder = tracer_provider_builder
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)));
        }
        _ => {}
    }

    opentelemetry::global::set_tracer_provider(tracer_provider_builder.build());

    Ok(())
}
