use haproxy_api::{Channel, FilterMethod, FilterResult, HttpMessage, Txn, UserFilter};
use mlua::prelude::{Lua, LuaResult, LuaTable};
use opentelemetry::propagation::Injector;
use opentelemetry::trace::{self, TraceContextExt, Tracer};
use opentelemetry::{Context, KeyValue, TraceId};
use opentelemetry_semantic_conventions::trace::{
    HTTP_REQUEST_METHOD, HTTP_RESPONSE_STATUS_CODE, URL_PATH, URL_QUERY,
};

use crate::{get_context, remove_context};

#[derive(Default)]
pub(crate) struct TraceFilter {
    start_client_span: Option<bool>,
    trace_id: Option<TraceId>,
    parent_context: Context,
    context: Context,
}

impl TraceFilter {
    // This method is called before proxying the request to the server (upstream)
    fn on_request_headers(
        &mut self,
        lua: &Lua,
        txn: Txn,
        msg: HttpMessage,
    ) -> LuaResult<FilterResult> {
        let tracer = opentelemetry::global::tracer("haproxy-otel");

        // Find parent context (if any)
        self.parent_context = match get_context(&txn) {
            Some(cx) => cx,
            None => return Ok(FilterResult::Continue),
        };
        self.trace_id = Some(self.parent_context.span().span_context().trace_id());

        // Skip client span creation if this option is disabled
        if self.start_client_span == Some(false) {
            return Ok(FilterResult::Continue);
        }

        let method = txn.f.get_str("method", ())?;
        let uri = txn.f.get_str("pathq", ())?;
        let mut uri_parts = uri.splitn(2, '?').map(|s| s.to_string());

        let span_builder = tracer
            .span_builder("upstream")
            .with_kind(trace::SpanKind::Client)
            .with_attributes([
                KeyValue::new(HTTP_REQUEST_METHOD, method),
                KeyValue::new(URL_PATH, uri_parts.next().unwrap_or_default()),
                KeyValue::new(URL_QUERY, uri_parts.next().unwrap_or_default()),
            ]);
        let span = tracer.build_with_context(span_builder, &self.parent_context);
        self.context = self.parent_context.with_span(span);

        // Inject tracing headers
        let silent_on = lua
            .app_data_ref::<crate::exporter::Options>()
            .map(|c| c.sampler.as_deref() == Some("SilentOn"))
            .unwrap_or_default();
        opentelemetry::global::get_text_map_propagator(|injector| {
            injector.inject_context(&self.context, &mut HeaderInjector::new(&msg, silent_on));
        });

        Ok(FilterResult::Continue)
    }

    // This method is called after receiving the response from the server (upstream)
    fn on_response_headers(
        &mut self,
        _lua: &Lua,
        txn: Txn,
        msg: HttpMessage,
    ) -> LuaResult<FilterResult> {
        // Skip this logic is client span creation is disabled
        if self.start_client_span == Some(false) {
            return Ok(FilterResult::Continue);
        }

        let span = self.context.span();
        span.add_event("received response headers", vec![]);

        let stline = msg.get_stline()?;
        let status = stline.raw_get::<_, i64>("code").unwrap_or_default();
        span.set_attribute(KeyValue::new(HTTP_RESPONSE_STATUS_CODE, status));
        if status < 500 {
            span.set_status(trace::Status::Ok);
        } else {
            span.set_status(trace::Status::error(stline.raw_get::<_, String>("reason")?));
        }

        let srv_name = txn.f.get_str("srv_name", ())?;
        span.set_attribute(KeyValue::new("haproxy.server.name", srv_name));

        Ok(FilterResult::Continue)
    }
}

impl UserFilter for TraceFilter {
    const METHODS: u8 = FilterMethod::END_ANALYZE | FilterMethod::HTTP_HEADERS;

    fn new(_lua: &Lua, args: LuaTable) -> LuaResult<Self> {
        let mut this = Self::default();
        if let Ok(args) = args.get::<_, String>(1) {
            for arg in args.split(';') {
                let (name, value) = arg.split_once('=').unwrap_or_default();
                match name {
                    "start_client_span" => {
                        this.start_client_span = Some(value.parse().unwrap_or(true));
                    }
                    _ => {}
                }
            }
        }
        Ok(this)
    }

    fn http_headers(&mut self, lua: &Lua, txn: Txn, msg: HttpMessage) -> LuaResult<FilterResult> {
        if !msg.is_resp()? {
            self.on_request_headers(lua, txn, msg)
        } else {
            self.on_response_headers(lua, txn, msg)
        }
    }

    fn end_analyze(&mut self, _lua: &Lua, txn: Txn, chn: Channel) -> LuaResult<FilterResult> {
        if chn.is_resp()? {
            // Finish client span
            if self.start_client_span.unwrap_or(true) {
                self.context.span().end();
            }

            // Finish server span
            if !txn
                .get_var::<bool>("txn.__otel_server_span")
                .unwrap_or_default()
                || self.trace_id.is_none()
            {
                return Ok(FilterResult::Continue);
            }

            remove_context(self.trace_id.unwrap());

            let span = self.parent_context.span();
            let status = (txn.f.get::<_, Option<i64>>("txn_status", ())?).unwrap_or_default();
            span.set_attribute(KeyValue::new(HTTP_RESPONSE_STATUS_CODE, status));
            if status < 500 {
                span.set_status(trace::Status::Ok);
            } else {
                span.set_status(trace::Status::error("5xx status code"));
            }

            let termination_state =
                (txn.f.get::<_, Option<String>>("txn_sess_term_state", ()))?.unwrap_or_default();
            span.set_attribute(KeyValue::new(
                "haproxy.termination_state",
                termination_state,
            ));

            span.end();
        }

        Ok(FilterResult::Continue)
    }
}

struct HeaderInjector<'a, 'b> {
    msg: &'a HttpMessage<'b>,
    silent_on: bool,
}

impl<'a, 'b> HeaderInjector<'a, 'b> {
    fn new(msg: &'a HttpMessage<'b>, silent_on: bool) -> Self {
        Self { msg, silent_on }
    }
}

impl Injector for HeaderInjector<'_, '_> {
    fn set(&mut self, key: &str, value: String) {
        if self.silent_on && key.eq_ignore_ascii_case("x-b3-sampled") {
            return;
        }
        let _ = self.msg.set_header(key, value);
    }
}
