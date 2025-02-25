#![cfg(test)]

use std::net::TcpListener;

use serde_json::{json, Value as JsonValue};
use std::time::Duration;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockGuard, MockServer, ResponseTemplate};

#[tokio::test]
async fn integration_tests() {
    // Compile haproxy-otel-module
    tokio::process::Command::new("cargo")
        .args(&["build", "--release", "-p", "haproxy-otel-module"])
        .current_dir("..")
        .status()
        .await
        .expect("Failed to compile haproxy-otel-module");

    // Start the mock server on port 4317
    let listener = TcpListener::bind("127.0.0.1:4317").unwrap();
    let mock_server = MockServer::builder().listener(listener).start().await;

    // Set up the mock for OTLP traces endpoint
    let otlp_mock = Mock::given(method("POST"))
        .and(path("/v1/trace"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accepted": true})))
        .expect(1)
        .mount_as_scoped(&mock_server)
        .await;

    // Set up the mock for regular HTTP requests (for testing propagation)
    let http_mock = Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Hello from test server"))
        .expect(1)
        .mount_as_scoped(&mock_server)
        .await;

    // Spawn haproxy and wait
    let mut haproxy = tokio::process::Command::new("haproxy")
        .args(&["-f", "haproxy.cfg"])
        .spawn()
        .expect("Failed to start haproxy");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Run the tests
    run_tests(&otlp_mock, &http_mock)
        .await
        .expect("Tests failed");

    haproxy.kill().await.expect("Failed to stop haproxy");
}

async fn run_tests(
    otlp_mock: &MockGuard,
    http_mock: &MockGuard,
) -> Result<(), Box<dyn std::error::Error>> {
    // Make a request to HAProxy
    let client = reqwest::Client::new();
    let response = client
        .get("http://localhost:8080/test")
        .header("X-Test-Header", "test-value")
        .send()
        .await?;
    assert_eq!(response.status(), 200);

    // Verify b3 headers propagation
    let http_req = (http_mock.received_requests().await)
        .pop()
        .expect("No HTTP test requests were received");
    let trace_headers = http_req
        .headers
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("x-b3"))
        .count();
    assert_eq!(trace_headers, 3, "Expected 3 tracing headers");

    // Verify the received OTLP spans
    timeout(Duration::from_secs(6), otlp_mock.wait_until_satisfied())
        .await
        .unwrap();
    let otlp_request = otlp_mock.received_requests().await.pop().unwrap();
    let spans = otlp_request.body_json::<JsonValue>().unwrap();

    // Get the spans array
    let spans_array = spans
        .pointer("/resourceSpans/0/scopeSpans/0/spans")
        .and_then(|s| s.as_array())
        .expect("Could not find spans array");

    // Verify we have exactly 2 spans
    assert_eq!(spans_array.len(), 2);

    // Find client and server spans
    let client_span = spans_array
        .iter()
        .find(|span| span["kind"].as_i64() == Some(3))
        .expect("Client span (kind=3) not found");
    let server_span = spans_array
        .iter()
        .find(|span| span["kind"].as_i64() == Some(2))
        .expect("Server span (kind=2) not found");

    // Verify service name
    let service_name = spans
        .pointer("/resourceSpans/0/resource/attributes/0/value/stringValue")
        .and_then(|v| v.as_str())
        .expect("Could not find service name");
    assert_eq!(service_name, "haproxy", "Service name should be 'haproxy'");

    // Verify span relationship (parent-child)
    assert_eq!(
        client_span["parentSpanId"], server_span["spanId"],
        "Client span's parent ID should match server span's ID"
    );

    // Verify both spans have the same trace ID
    assert_eq!(
        client_span["traceId"], server_span["traceId"],
        "Both spans should have the same trace ID"
    );

    // Verify custom attribute on server span
    let has_custom_attr = (server_span["attributes"].as_array().unwrap())
        .iter()
        .any(|attr| {
            attr["key"].as_str() == Some("test_attribute")
                && attr["value"]["stringValue"].as_str() == Some("hello")
        });
    assert!(
        has_custom_attr,
        "Server span missing custom attribute 'test_attribute' with value 'hello'"
    );

    Ok(())
}
