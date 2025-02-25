local opentelemetry = require("haproxy_otel_module")

opentelemetry.register({
	name = "haproxy",
	otlp = {
		endpoint = "http://localhost:4317/v1/trace",
		protocol = "json",
	},
	sampler = "AlwaysOn",
	propagator = "zipkin",
})
