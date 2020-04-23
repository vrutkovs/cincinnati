//! Tracing service.

use opentelemetry::api::Key;
use opentelemetry::{global, sdk};

/// Header name used by jaeger to set trace context
pub static TRACE_HEADER_NAME: &str = "uber-trace-id";
// TODO: find a way to import rustracing_jaeger::constants::TRACER_CONTEXT_HEADER_NAME?

fn init_tracer() -> thrift::Result<()> {
    let exporter = opentelemetry_jaeger::Exporter::builder()
        .with_agent_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(opentelemetry_jaeger::Process {
            service_name: "policy-engine".to_string(),
            tags: vec![Key::new("exporter").string("jaeger")],
        })
        .init()?;

    let provider = sdk::Provider::builder()
        .with_simple_exporter(exporter)
        .with_config(sdk::Config {
            default_sampler: Box::new(sdk::Sampler::Always),
            ..Default::default()
        })
        .build();
    global::set_provider(provider);

    Ok(())
}

// /// Build HeaderName and HeaderValue for current span
// pub fn build_header_for_span(span: &Span) -> (HeaderName, HeaderValue) {
//     let trace_id = match span.context() {
//         Some(context) => context.state().to_string(),
//         None => String::new(),
//     };
//     (
//         HeaderName::from_lowercase(TRACE_HEADER_NAME.as_bytes()).unwrap(),
//         HeaderValue::from_str(&trace_id).unwrap(),
//     )
// }

// /// Extract headers from request and prepare a new span
// pub fn create_span_from_headers(
//     tracer: &Tracer,
//     span_name: &'static str,
//     headers: &HeaderMap,
// ) -> Span {
//     let mut carrier: HashMap<String, String> = HashMap::new();
//     for (k, v) in headers {
//         carrier.insert(k.to_string(), v.to_str().unwrap().to_string());
//     }

//     let context = track_try_unwrap!(SpanContext::extract_from_http_header(&carrier));
//     let mut _span_builder = tracer.span(span_name).child_of(&context);
//     for (k, v) in carrier {
//         _span_builder = _span_builder.tag(Tag::new(k, v))
//     }
//     _span_builder.start()
// }
