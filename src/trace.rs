use anyhow::Result;
use opentelemetry::propagation::Injector;
use opentelemetry::trace::{SpanKind, SpanRef, TraceContextExt};
use opentelemetry::{Context, global};
use opentelemetry::{
    KeyValue,
    propagation::Extractor,
    trace::{Span, Tracer},
};
// use opentelemetry_otlp::tonic_types;
use std::collections::HashMap;
use std::fmt::{Debug, Write as _};
use std::sync::LazyLock;
use tonic::Request;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod attr;
pub mod impls;
pub mod otel_span;

/// Env var controlling how many bytes of a request/response payload are
/// rendered into a span attribute. Large payloads (image base64, bulk
/// imports) otherwise inflate exported spans to GB scale. `0` is a hard
/// kill switch that disables payload rendering entirely — it wins even over
/// the TRACE-level "show everything" path. Unset/invalid falls back to the
/// default. Read once (spans are hot-path); restart to change.
const TRACE_PAYLOAD_MAX_LEN_DEFAULT: usize = 2048;
static TRACE_PAYLOAD_MAX_LEN: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("TRACE_PAYLOAD_MAX_LEN")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(TRACE_PAYLOAD_MAX_LEN_DEFAULT)
});

/// A `fmt::Write` sink that keeps at most `max` bytes (snapped to a UTF-8
/// char boundary) while counting the full byte length it was fed. This lets
/// us bound a `Debug` rendering's allocation to `max` instead of
/// materializing the whole payload first — critical when the payload is a
/// multi-MB image base64 string that we only want 2 KB of.
struct BoundedWriter {
    buf: String,
    max: usize,
    total: usize,
    /// Set once a chunk can't be fully kept. After that `buf` is frozen so
    /// it stays a true prefix: otherwise a later chunk could append after a
    /// gap (e.g. a multi-byte char dropped for lack of room, then a short
    /// ASCII separator slipping into the leftover budget), yielding output
    /// that is NOT the actual leading bytes of the payload.
    truncated: bool,
}

impl std::fmt::Write for BoundedWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.total += s.len();
        if self.truncated {
            return Ok(());
        }
        let room = self.max - self.buf.len();
        let end = s.floor_char_boundary(room.min(s.len()));
        self.buf.push_str(&s[..end]);
        if end < s.len() {
            self.truncated = true;
        }
        Ok(())
    }
}

/// Render a Debug payload for a span attribute.
///
/// `TRACE_PAYLOAD_MAX_LEN` semantics, in precedence order:
/// - `0` is a hard kill switch — payloads are NEVER rendered, even at TRACE.
///   Operators set it precisely to keep huge payloads out of memory and
///   exported spans, so enabling TRACE must not silently reintroduce them.
/// - otherwise, when `TRACE` is enabled FOR THIS MODULE the full payload is
///   emitted untruncated (the operator has opted into seeing everything,
///   regardless of size — a TRACE on an unrelated target does not count);
/// - otherwise the rendering is bounded to that many kept bytes (default
///   2048) so large payloads don't inflate spans — or memory — to GB scale.
fn render_payload<T: Debug>(payload: &T) -> Option<String> {
    // `enabled!` (not `LevelFilter::current()`) so the check honors the
    // subscriber's per-target filter for THIS module. The global max-level
    // hint would report TRACE even when only an unrelated target is at TRACE
    // (e.g. `RUST_LOG=other_crate=trace,command_utils=info`), bypassing the
    // cap for payloads this module renders.
    let trace_enabled = tracing::enabled!(tracing::Level::TRACE);
    resolve_payload(payload, *TRACE_PAYLOAD_MAX_LEN, trace_enabled)
}

/// Apply the precedence rules of [`render_payload`], taking `max` and
/// `trace_enabled` as plain args so the kill-switch-beats-TRACE invariant is
/// testable without a `LazyLock` env read or a global tracing subscriber.
fn resolve_payload<T: Debug>(payload: &T, max: usize, trace_enabled: bool) -> Option<String> {
    if max == 0 {
        return None;
    }
    if trace_enabled {
        return Some(format!("{payload:?}"));
    }
    render_bounded(payload, max)
}

/// Bounded `Debug` rendering, split out so it is testable without a global
/// tracing subscriber or env state. Keeps up to `max` bytes of the prefix
/// (`0` suppresses entirely → `None`) and, when the payload was longer,
/// appends a `…(truncated N bytes)` marker so the omission is visible.
fn render_bounded<T: Debug>(payload: &T, max: usize) -> Option<String> {
    if max == 0 {
        return None;
    }
    let mut w = BoundedWriter {
        buf: String::new(),
        max,
        total: 0,
        truncated: false,
    };
    // BoundedWriter::write_str is infallible.
    let _ = write!(w, "{payload:?}");
    if w.truncated {
        let dropped = w.total - w.buf.len();
        let _ = write!(w.buf, "…(truncated {dropped} bytes)");
    }
    Some(w.buf)
}

// struct MetadataMap<'a>(&'a tonic_types::metadata::MetadataMap);
// struct MetadataMutMap<'a>(&'a mut tonic_types::metadata::MetadataMap);

struct MetadataMap<'a>(&'a tonic::metadata::MetadataMap);
struct MetadataMutMap<'a>(&'a mut tonic::metadata::MetadataMap);

// for server-side metadata extraction
impl Extractor for MetadataMap<'_> {
    /// Get a value for a key from the MetadataMap.  If the value can't be converted to &str, returns None
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|metadata| metadata.to_str().ok())
    }

    /// Collect all the keys from the MetadataMap.
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|key| match key {
                tonic::metadata::KeyRef::Ascii(v) => v.as_str(),
                tonic::metadata::KeyRef::Binary(v) => v.as_str(),
            })
            .collect::<Vec<_>>()
    }
}

// Trait for tracing requests in OpenTelemetry
impl Injector for MetadataMutMap<'_> {
    /// Set a key and value in the MetadataMap.  Does nothing if the key or value are not valid inputs
    fn set(&mut self, key: &str, value: String) {
        if let Ok(key) = tonic::metadata::MetadataKey::from_bytes(key.as_bytes())
            && let Ok(val) = tonic::metadata::MetadataValue::try_from(&value)
        {
            self.0.insert(key, val);
        }
    }
}

//https://opentelemetry.io/docs/specs/semconv/general/trace/
pub trait Tracing {
    fn create_context(metadata: &HashMap<String, String>) -> Context {
        global::get_text_map_propagator(|propagator| propagator.extract(metadata))
    }
    fn metadata_from_context(cx: &Context) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        global::get_text_map_propagator(|propagator| propagator.inject_context(cx, &mut metadata));
        metadata
    }
    fn inject_metadata_from_context(metadata: &mut HashMap<String, String>, cx: &Context) {
        global::get_text_map_propagator(|propagator| propagator.inject_context(cx, metadata));
    }
    // XXX not working...
    fn tracing_span_from_metadata(
        metadata: &HashMap<String, String>,
        app_name: &'static str,
        span_name: &'static str,
    ) -> tracing::Span {
        let parent_cx = global::get_text_map_propagator(|prop| prop.extract(metadata));
        let child_tracing_span = tracing::span!(
            tracing::Level::INFO,
            "_",
            "app.name" = app_name,
            "span.name" = span_name
        );
        let _ = child_tracing_span.set_parent(parent_cx.clone());
        child_tracing_span
    }
    // XXX not working...
    fn child_tracing_span(
        parent_cx: &opentelemetry::Context,
        app_name: &'static str,
        span_name: String,
    ) -> tracing::Span {
        let span = tracing::info_span!("_", "app.name" = app_name, "span.name" = span_name);
        let _ = span.set_parent(parent_cx.clone());
        span
    }
    fn start_child_otel_span(
        parent_cx: &opentelemetry::Context,
        app_name: &'static str,
        span_name: String,
    ) -> global::BoxedSpan {
        global::tracer(app_name).start_with_context(span_name, parent_cx)
    }
    fn otel_span_from_metadata(
        metadata: &HashMap<String, String>,
        app_name: &'static str,
        span_name: &'static str,
    ) -> global::BoxedSpan {
        let parent_cx = global::get_text_map_propagator(|prop| prop.extract(metadata));
        global::tracer(app_name).start_with_context(span_name, &parent_cx)
    }

    fn trace_request<T: Debug>(
        name: &'static str,
        span_name: &'static str,
        request: &Request<T>,
    ) -> global::BoxedSpan {
        let parent_cx =
            global::get_text_map_propagator(|prop| prop.extract(&MetadataMap(request.metadata())));
        let mut span = global::tracer(name).start_with_context(span_name, &parent_cx);

        span.set_attribute(KeyValue::new("service.name", name));
        span.set_attribute(KeyValue::new("service.method", span_name));
        if let Some(payload) = render_payload(request) {
            span.set_attribute(KeyValue::new("request", payload));
        }

        if let Some(req_path) = request.metadata().get("path")
            && let Ok(path_str) = req_path.to_str()
        {
            // Clone the string to own it, avoiding reference lifetime issues
            span.set_attribute(KeyValue::new("request.path", path_str.to_string()));
        }

        span
    }
    fn trace_response<T: Debug>(span: &mut global::BoxedSpan, response: &T) {
        if let Some(payload) = render_payload(response) {
            span.set_attribute(KeyValue::new("response", payload));
        }
        span.end();
    }
    fn trace_error(span: &mut global::BoxedSpan, error: &dyn std::error::Error) {
        span.record_error(error);
        span.set_status(opentelemetry::trace::Status::error(error.to_string()));
        span.end();
    }

    // Helper function for tracing gRPC client operations with custom request and context injection
    fn trace_grpc_client_with_request<D, F, Fut, T>(
        context: Option<Context>,
        name: &'static str,
        span_name: &'static str,
        method_name: &'static str,
        mut request_data: tonic::Request<D>,
        operation: F,
    ) -> impl std::future::Future<Output = Result<T>> + Send
    where
        D: Debug + Send + 'static,
        F: FnOnce(tonic::Request<D>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T>> + Send + 'static,
        T: Debug + Send + 'static,
    {
        async move {
            let mut attributes = vec![
                KeyValue::new("rpc.system", "grpc"),
                KeyValue::new("service.name", name),
                KeyValue::new("rpc.method", method_name),
            ];
            if let Some(payload) = render_payload(&request_data) {
                attributes.push(KeyValue::new("input.value", payload));
            }
            let span =
                Self::start_client_span_with_context(name, span_name, attributes, context.as_ref());
            let cx = context.unwrap_or_else(|| Context::current_with_span(span));

            // Create request and inject trace context
            global::get_text_map_propagator(|propagator| {
                propagator.inject_context(&cx, &mut MetadataMutMap(request_data.metadata_mut()))
            });

            let response = operation(request_data).await;

            match response {
                Ok(res) => {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new("rpc.result", "success"));
                    span.end();
                    Ok(res)
                }
                Err(e) => {
                    let span = cx.span();
                    span.record_error(e.as_ref());
                    span.set_status(opentelemetry::trace::Status::error(e.to_string()));
                    span.end();
                    Err(e)
                }
            }
        }
    }
    fn start_client_span_with_context(
        name: &'static str,
        span_name: &'static str,
        attributes: Vec<KeyValue>,
        context: Option<&Context>,
    ) -> opentelemetry::global::BoxedSpan {
        let tracer = global::tracer(name);
        if let Some(ctx) = context {
            tracer
                .span_builder(span_name)
                .with_kind(SpanKind::Client)
                .with_attributes(attributes)
                .start_with_context(&tracer, ctx)
        } else {
            tracer
                .span_builder(span_name)
                .with_kind(SpanKind::Client)
                .with_attributes(attributes)
                .start(&tracer)
        }
    }
    fn record_error(span: &SpanRef<'_>, error: &str) {
        span.set_status(opentelemetry::trace::Status::error(error.to_string()));
        span.set_attribute(KeyValue::new("error", error.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::{render_bounded, resolve_payload};

    /// Debug-renders to exactly its inner string (no quoting), so tests can
    /// assert on byte budgets without reasoning about `Debug` escaping.
    struct Raw<'a>(&'a str);
    impl std::fmt::Debug for Raw<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    #[test]
    fn resolve_payload_kill_switch_beats_trace() {
        let big = "x".repeat(10_000);
        // max == 0 must suppress regardless of the TRACE level: it is the
        // operator's hard "never render payloads" switch.
        assert_eq!(resolve_payload(&Raw(&big), 0, true), None);
        assert_eq!(resolve_payload(&Raw(&big), 0, false), None);
    }

    #[test]
    fn resolve_payload_trace_emits_full_when_enabled() {
        let big = "x".repeat(10_000);
        // TRACE + non-zero cap: full payload, untruncated.
        assert_eq!(resolve_payload(&Raw(&big), 16, true), Some(big.clone()));
        // non-TRACE: bounded to the cap.
        let bounded = resolve_payload(&Raw(&big), 16, false).unwrap();
        assert!(bounded.len() < big.len());
        assert!(bounded.starts_with(&"x".repeat(16)));
    }

    #[test]
    fn render_bounded_disabled_returns_none() {
        assert_eq!(render_bounded(&Raw("anything"), 0), None);
    }

    #[test]
    fn render_bounded_under_limit_is_unchanged() {
        assert_eq!(render_bounded(&Raw("hello"), 16), Some("hello".to_string()));
        // exactly at the limit must not be truncated
        assert_eq!(render_bounded(&Raw("hello"), 5), Some("hello".to_string()));
    }

    #[test]
    fn render_bounded_over_limit_truncates_with_marker() {
        // 10 bytes total, kept 4, dropped 6
        let out = render_bounded(&Raw("abcdefghij"), 4).unwrap();
        assert_eq!(out, "abcd…(truncated 6 bytes)");
    }

    #[test]
    fn render_bounded_respects_utf8_char_boundary() {
        // each "あ" is 3 bytes; cap at 4 must back off to 3 (one full char)
        let out = render_bounded(&Raw("ああ"), 4).unwrap();
        assert_eq!(out, "あ…(truncated 3 bytes)");
        // cap landing mid-first-char (1 or 2) backs off to 0 kept bytes
        let out = render_bounded(&Raw("ああ"), 2).unwrap();
        assert_eq!(out, "…(truncated 6 bytes)");
    }

    #[test]
    fn render_bounded_caps_allocation_not_just_output() {
        // The kept buffer must stay within `max` even for a huge payload —
        // this is the whole point: bound memory, not just the exported span.
        let huge = "x".repeat(10_000_000);
        let out = render_bounded(&Raw(&huge), 16).unwrap();
        assert!(out.starts_with(&"x".repeat(16)));
        assert!(out.ends_with("…(truncated 9999984 bytes)"));
    }

    /// Emits each `&str` as a separate `write_str` call, mimicking how
    /// `Debug` for a struct interleaves field values with separators.
    struct Chunks<'a>(&'a [&'a str]);
    impl std::fmt::Debug for Chunks<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            for s in self.0 {
                f.write_str(s)?;
            }
            Ok(())
        }
    }

    #[test]
    fn render_bounded_output_is_a_true_prefix_across_chunks() {
        // Regression: once a chunk can't be fully kept, no LATER chunk may
        // sneak into the leftover budget. With max=4: "あ" (3B) fits, then
        // "い" (3B) has only 1B room → dropped (truncated). The trailing
        // ", x" must NOT slip into that remaining 1B — output stays a real
        // prefix of the rendered payload.
        let out = render_bounded(&Chunks(&["あ", "い", ", x"]), 4).unwrap();
        // total = 3 + 3 + 3 = 9 bytes; kept = "あ" (3) → dropped 6
        assert_eq!(out, "あ…(truncated 6 bytes)");
    }
}
