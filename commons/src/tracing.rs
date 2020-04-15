//! Tracing macros

#[macro_export]
/// Log message in the span.
macro_rules! trace_log {
    ( $span:expr, $msg:expr ) => {{
        $span.log(|log| {
            log.std().message($msg);
        });
    }};
}
