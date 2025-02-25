# haproxy-otel

A Rust-based [OpenTelemetry] module for [HAProxy] Community Edition using the [Lua API].

[OpenTelemetry]: https://opentelemetry.io
[HAProxy]: https://www.haproxy.org
[Lua API]: https://www.arpalert.org/src/haproxy-lua-api/3.0/index.html

## Features

- Server-side and client-side span creation
- Custom attribute support
- OTLP exporter
- Support for Zipkin and Jaeger headers propagation formats

## Usage and Configuration

Please check the [module](module) and [tests](tests) directories for working examples.

### HAProxy Configuration

```haproxy
global
    master-worker
    # This is required to enable non-blocking background activity (exporting traces)
    insecure-fork-wanted
    lua-load-per-thread otel.lua

...

frontend http-in
    bind *:8080
    http-request lua.start_server_span
    http-request set-var-fmt(txn.custom_attr_value) "hello"
    http-request lua.set_span_attribute_var test_attribute txn.custom_attr_value
    default_backend default

backend default
    filter lua.opentelemetry-trace
    server srv1 127.0.0.1:8080
```

Create a file named `otel.lua` with the following content:

```lua
local opentelemetry = require("haproxy_otel_module")

opentelemetry.register({
    name = "loadbalancer",  -- Service name
    sampler = "AlwaysOn",  -- Sampling strategy
    propagator = "zipkin",  -- Propagation format
    otlp = {
        endpoint = "http://otel-collector:4317/v1/trace",  -- OTLP endpoint
        protocol = "json",  -- Protocol format (json or binary)
    }
})
```

### Configuration Options

| Option          | Description                      | Values                                       | Default                            |
| --------------- | -------------------------------- | -------------------------------------------- | ---------------------------------- |
| `name`          | Service name for the traces      | String                                       | `"haproxy"`                        |
| `sampler`       | Sampling strategy                | `"AlwaysOn"`, `"AlwaysOff"`, `"ParentBased"` | `"ParentBased"`                    |
| `propagator`    | Trace context propagation format | `"zipkin"`, `"jaeger"`                       | `"zipkin"`                         |
| `otlp.endpoint` | OTLP collector endpoint URL      | URL string                                   | `"http://localhost:4318/v1/trace"` |
| `otlp.protocol` | OTLP protocol format             | `"json"`, `"binary"`                         | `"binary"`                         |

### Tracing

To create a server-side and a client-side (upstream proxying) spans for incoming requests:

```
http-request lua.start_server_span
filter lua.opentelemetry-trace start_client_span=true
```

The `http-request` directive is required to start a new span and `filter` is responsible for (optional) client-side (upstream) span creation and finishing all the spans.


| Option              | Description                    | Values          | Default |
| ------------------- | ------------------------------ | --------------- | ------- |
| `start_client_span` | Whether to create client spans | `true`, `false` | `true`  |

### Adding Custom Attributes

You can add custom attributes to spans:

```
http-request set-var-fmt(txn.user_id) "%[req.hdr(User-ID)]"
http-request lua.set_span_attribute_var user.id txn.user_id
```

You need to assign a value to a variable first, and then use the `lua.set_span_attribute_var` function to add the attribute to the span.

## Integration with OpenTelemetry Collector

This module sends traces to an OpenTelemetry [collector]. Configure your collector to receive OTLP traces and export them to your preferred backend (Jaeger, Zipkin, etc.).

[collector]: https://opentelemetry.io/docs/collector/

## License

This project is licensed under the [MIT license](LICENSE).
