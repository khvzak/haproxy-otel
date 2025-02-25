use std::collections::HashMap;
use std::time::SystemTime;

use haproxy_api::Txn;
use mlua::prelude::{Lua, LuaResult, LuaString, LuaTable};
use opentelemetry::trace::{self, Span, TraceContextExt, Tracer};
use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions::trace::{
    HTTP_REQUEST_METHOD, NETWORK_PEER_ADDRESS, URL_PATH, URL_QUERY,
};

use crate::{get_context, store_context};

pub(crate) fn start_server_span(_lua: &Lua, txn: Txn) -> LuaResult<()> {
    let tracer = opentelemetry::global::tracer("haproxy-otel");
    let http = txn.http()?;

    // Extract parent context from the request headers
    let headers = http.req_get_headers().and_then(tracing_headers2map)?;
    let remote_context = opentelemetry::global::get_text_map_propagator(|p| p.extract(&headers));

    let method = txn.f.get_str("method", ())?;
    let uri = txn.f.get_str("pathq", ())?;
    let host = headers.get("host").cloned().unwrap_or_default();
    let peer_addr = txn.f.get_str("src", ())?;

    let mut uri_parts = uri.splitn(2, '?').map(|s| s.to_string());
    let span_builder = tracer
        .span_builder(format!("{method} {host}"))
        .with_kind(trace::SpanKind::Server)
        .with_start_time(SystemTime::now())
        .with_attributes([
            KeyValue::new(HTTP_REQUEST_METHOD, method),
            KeyValue::new(URL_PATH, uri_parts.next().unwrap_or_default()),
            KeyValue::new(URL_QUERY, uri_parts.next().unwrap_or_default()),
            KeyValue::new("http.request.header.host", host),
            KeyValue::new(NETWORK_PEER_ADDRESS, peer_addr),
        ]);
    let span = tracer.build_with_context(span_builder, &remote_context);
    let trace_id = span.span_context().trace_id();
    let context = remote_context.with_span(span);

    // Mark this session as "main" for finishing the server span
    // This is a private variable to share data with filter
    txn.set_var("txn.__otel_server_span", true)?;

    // Save the context independently of the session
    store_context(&txn, trace_id, context);

    Ok(())
}

pub(crate) fn set_span_attribute(
    _lua: &Lua,
    (txn, name, var_name): (Txn, String, String),
) -> LuaResult<()> {
    if let Ok(value) = txn.get_var::<String>(&var_name) {
        if let Some(context) = get_context(&txn) {
            context.span().set_attribute(KeyValue::new(name, value));
        }
    }
    Ok(())
}

/// Convert only specific tracing headers to a map for context extraction
fn tracing_headers2map(headers: haproxy_api::Headers) -> LuaResult<HashMap<String, String>> {
    let mut map = HashMap::new();
    headers.for_each::<LuaString, LuaTable>(|name, value| {
        let nameb = name.as_bytes();
        if nameb == b"host"
            || nameb == b"b3"
            || nameb.starts_with(b"x-b3")
            || nameb.starts_with(b"uber")
        {
            let name = name.to_string_lossy();
            let value = value.get::<_, LuaString>(0);
            if let Ok(value) = value.as_ref().map(|v| v.to_string_lossy()) {
                map.insert(name.into_owned(), value.into_owned());
            }
        }
        Ok(())
    })?;
    Ok(map)
}
