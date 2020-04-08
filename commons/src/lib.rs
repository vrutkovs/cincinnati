//! Common utilities for Cincinnati backend.

#![deny(missing_docs)]

extern crate actix_web;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate trackable;

mod config;
pub use crate::config::MergeOptions;

pub mod de;
pub mod metrics;
pub mod testing;

mod errors;
pub use errors::{register_metrics, GraphError, MISSING_APPSTATE_PANIC_MSG};

use actix_web::http::{header, HeaderMap};
use std::collections::{HashMap, HashSet};
use url::form_urlencoded;

use reqwest::header::{HeaderName, HeaderValue};
use rustracing::tag::Tag;
use rustracing_jaeger::span::Span;
use rustracing_jaeger::span::SpanContext;
use rustracing_jaeger::Tracer;

/// Header name used by jaeger to set trace context
pub static TRACE_HEADER_NAME: &str = "uber-trace-id";
// TODO: find a way to import rustracing_jaeger::constants::TRACER_CONTEXT_HEADER_NAME?

/// Strip all but one leading slash and all trailing slashes
pub fn parse_path_prefix<S>(path_prefix: S) -> String
where
    S: AsRef<str>,
{
    format!("/{}", path_prefix.as_ref().to_string().trim_matches('/'))
}

/// Deserialize path_prefix
pub fn de_path_prefix<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let path_prefix = String::deserialize(deserializer)?;
    Ok(Some(parse_path_prefix(path_prefix)))
}

/// Parse a comma-separated set of client parameters keys.
pub fn parse_params_set<S>(params: S) -> HashSet<String>
where
    S: AsRef<str>,
{
    params
        .as_ref()
        .split(',')
        .filter_map(|key| {
            let trimmed = key.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect()
}

/// Make sure `query` string contains all `params` keys.
pub fn ensure_query_params(
    required_params: &HashSet<String>,
    query: &str,
) -> Result<(), GraphError> {
    // No mandatory parameters, always fine.
    if required_params.is_empty() {
        return Ok(());
    }

    // Extract and de-duplicate keys from input query.
    let query_keys: HashSet<String> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .map(|(k, _)| k)
        .collect();

    // Make sure no mandatory parameters are missing.
    let mut missing: Vec<String> = required_params.difference(&query_keys).cloned().collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(GraphError::MissingParams(missing));
    }

    Ok(())
}

/// Make sure client requested a valid content type.
pub fn ensure_content_type(
    headers: &HeaderMap,
    content_type: &'static str,
) -> Result<(), GraphError> {
    let content_json = header::HeaderValue::from_static(content_type);

    if !headers
        .get(header::ACCEPT)
        .map(|accept| accept == content_json)
        .unwrap_or(false)
    {
        Err(GraphError::InvalidContentType)
    } else {
        Ok(())
    }
}

/// Build HeaderName and HeaderValue for current span
pub fn build_header_for_span(span: &Span) -> (HeaderName, HeaderValue) {
    let trace_id = match span.context() {
        Some(context) => context.state().to_string(),
        None => String::new(),
    };
    (
        HeaderName::from_lowercase(TRACE_HEADER_NAME.as_bytes()).unwrap(),
        HeaderValue::from_str(&trace_id).unwrap(),
    )
}

/// Extract headers from request and prepare a new span
pub fn create_span_from_headers(
    tracer: &Tracer,
    span_name: &'static str,
    headers: &HeaderMap,
) -> Span {
    let mut carrier: HashMap<String, String> = HashMap::new();
    for (k, v) in headers {
        carrier.insert(k.to_string(), v.to_str().unwrap().to_string());
    }

    let context = track_try_unwrap!(SpanContext::extract_from_http_header(&carrier));
    let mut _span_builder = tracer.span(span_name).child_of(&context);
    for (k, v) in carrier {
        _span_builder = _span_builder.tag(Tag::new(k, v))
    }
    _span_builder.start()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_path_prefix() {
        assert_eq!(parse_path_prefix("//a/b/c//"), "/a/b/c");
        assert_eq!(parse_path_prefix("/a/b/c/"), "/a/b/c");
        assert_eq!(parse_path_prefix("/a/b/c"), "/a/b/c");
        assert_eq!(parse_path_prefix("a/b/c"), "/a/b/c");
    }

    #[test]
    fn test_parse_params_set() {
        assert_eq!(parse_params_set(""), HashSet::new());

        let basic = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(parse_params_set("a,b,c"), basic.into_iter().collect());

        let dedup = vec!["a".to_string(), "b".to_string()];
        assert_eq!(parse_params_set("a,b,a"), dedup.into_iter().collect());

        let trimmed = vec!["foo".to_string(), "bar".to_string()];
        assert_eq!(
            parse_params_set("foo , , bar"),
            trimmed.into_iter().collect()
        );
    }

    #[test]
    fn test_ensure_query_params() {
        let empty = HashSet::new();
        ensure_query_params(&empty, "").unwrap();
        ensure_query_params(&empty, "a=b").unwrap();

        let simple = vec!["a".to_string()].into_iter().collect();
        ensure_query_params(&simple, "a=b").unwrap();
        ensure_query_params(&simple, "a=b&a=c").unwrap();
        ensure_query_params(&simple, "").unwrap_err();
        ensure_query_params(&simple, "c=d").unwrap_err();
    }

    #[test]
    fn test_ensure_content_type() {
        let mut headers = actix_web::http::HeaderMap::new();
        headers.insert(header::ACCEPT, "application/json".parse().unwrap());
        ensure_content_type(&headers, "application/json").unwrap();
        ensure_content_type(&headers, "text/html").unwrap_err();
    }
}
