use std::sync::OnceLock;

use haproxy_api::Txn;
use mlua::prelude::LuaString;
use opentelemetry::{Context, TraceId};

// This is a global cache to store the context of the spans
// It can be reused independently of http session in many listeners
static TRACE_CACHE: OnceLock<quick_cache::sync::Cache<String, Context>> = OnceLock::new();

fn init_cache() -> quick_cache::sync::Cache<String, Context> {
    quick_cache::sync::Cache::new(1_000_000)
}

// Get the context from the global cache
pub(crate) fn get_context(txn: &Txn) -> Option<Context> {
    let trace_id = txn.get_var::<LuaString>("txn.otel_trace_id").ok()?;
    TRACE_CACHE
        .get_or_init(init_cache)
        .get(trace_id.to_str().ok()?)
}

// Store the context in the globally cache to share it between listeners/frontends
pub(crate) fn store_context(txn: &Txn, trace_id: TraceId, context: Context) {
    let trace_id = const_hex::encode(trace_id.to_bytes());
    let _ = txn.set_var("txn.otel_trace_id", &*trace_id);
    TRACE_CACHE
        .get_or_init(init_cache)
        .insert(trace_id, context);
}

pub(crate) fn remove_context(trace_id: TraceId) {
    let trace_id = const_hex::encode(trace_id.to_bytes());
    TRACE_CACHE.get_or_init(init_cache).remove(&trace_id);
}
