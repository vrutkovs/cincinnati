//! Tracing service.

use opentelemetry::api::{
    Carrier, HttpTextFormat, Key, Provider, Span, SpanContext, TraceContextPropagator,
};
use opentelemetry::{global, sdk};
use opentelemetry_jaeger::{Exporter, Process};

use actix_web::dev::ServiceRequest;
use actix_web::http;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

/// init_tracer sets up Jaeger tracer
pub fn init_tracer(name: &'static str) -> thrift::Result<()> {
    let exporter = Exporter::builder()
        .with_agent_endpoint("127.0.0.1:6831".parse().unwrap())
        .with_process(Process {
            service_name: name.to_string(),
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

/// get_tracer returns an instance of global tracer
pub fn get_tracer() -> global::BoxedTracer {
    global::trace_provider().get_tracer("")
}

struct HttpHeaderMapCarrier<'a>(&'a http::HeaderMap);
impl<'a> Carrier for HttpHeaderMapCarrier<'a> {
    fn get(&self, key: &'static str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn set(&mut self, _key: &'static str, _value: String) {
        unimplemented!()
    }
}

struct ClientHeaderMapCarrier<'a>(&'a mut HeaderMap);
impl<'a> Carrier for ClientHeaderMapCarrier<'a> {
    fn get(&self, key: &'static str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn set(&mut self, key: &'static str, value: String) {
        self.0.insert(
            HeaderName::from_bytes(key.as_bytes()).unwrap(),
            HeaderValue::from_str(&value).unwrap(),
        );
    }
}

/// Return the parent context for the request if specific headers found.
pub fn get_context(req: &ServiceRequest) -> SpanContext {
    let propagator = TraceContextPropagator::new();
    propagator.extract(&HttpHeaderMapCarrier(&req.headers()))
}

/// Inject context data into headers
pub fn set_context(context: SpanContext, headers: &mut HeaderMap) {
    let propagator = TraceContextPropagator::new();
    propagator.inject(context, &mut ClientHeaderMapCarrier(headers));
}

/// Add span attributes from servicerequest
pub fn set_span_tags(req: &ServiceRequest, span: &dyn Span) {
    span.set_attribute(Key::new("path").string(req.path()));
    req.headers().iter().for_each(|(k, v)| {
        span.set_attribute(
            Key::new(format!("header.{}", k.to_string())).string(v.to_str().unwrap()),
        )
    });
}
