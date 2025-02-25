use haproxy_api::{Action, Core};
use mlua::prelude::{Lua, LuaExternalResult as _, LuaResult, LuaTable};

pub(crate) use cache::{get_context, remove_context, store_context};

pub fn register(lua: &Lua, options: LuaTable) -> LuaResult<()> {
    let core = Core::new(lua)?;

    let service_name = (options.get::<_, String>("name")).unwrap_or_else(|_| "haproxy".to_string());
    let sampler = (options.get::<_, Option<String>>("sampler")).unwrap_or_default();
    let propagator = (options.get::<_, Option<String>>("propagator")).unwrap_or_default();
    let otlp = (options.get::<_, LuaTable>("otlp")).unwrap_or_else(|_| lua.create_table().unwrap());
    let endpoint = (otlp.get::<_, Option<String>>("endpoint")).unwrap_or_default();
    let protocol = (otlp.get::<_, Option<String>>("protocol")).unwrap_or_default();

    let options = exporter::Options {
        service_name: service_name.clone(),
        sampler: sampler.clone(),
        propagator: propagator.clone(),
        endpoint: endpoint.clone(),
        protocol: protocol.clone(),
    };
    lua.set_app_data(options.clone());

    if core.thread()? <= 1 {
        core.register_task(move |_lua| exporter::init(options.clone()).into_lua_err())?;
    }

    #[rustfmt::skip]
    core.register_action("start_server_span", &[Action::HttpReq], 0, span::start_server_span)?;
    core.register_action(
        "set_span_attribute_var",
        &[Action::HttpReq, Action::HttpRes, Action::HttpAfterRes],
        2,
        span::set_span_attribute,
    )?;
    core.register_filter::<filter::TraceFilter>("opentelemetry-trace")?;

    Ok(())
}

mod cache;
mod exporter;
mod filter;
mod runtime;
mod span;
