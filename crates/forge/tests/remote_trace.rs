use alloy_primitives::{Address, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_rpc_types::trace::geth::{
    CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
    GethDebugTracingOptions, GethTrace,
};
use alloy_serde::WithOtherFields;
use foundry_cli::utils;
use foundry_config::Config;

#[tokio::test(flavor = "multi_thread")]
async fn debug_trace_call_parses_calltracer() {
    let server = httpmock::MockServer::start();

    let _m = server.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/")
            .json_body_includes(serde_json::json!({"method":"debug_traceCall"}));

        then.status(200).json_body(serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":{
                "type":"CALL",
                "from":"0x0000000000000000000000000000000000000001",
                "to":"0x0000000000000000000000000000000000000002",
                "gas":"0x1",
                "gasUsed":"0x0",
                "input":"0x",
                "output":"0x",
                "value":"0x0",
                "calls":[]
            }
        }));
    });

    let mut config = Config::default();
    config.eth_rpc_url = Some(server.base_url());
    let provider = utils::get_provider(&config).unwrap();

    let tx = TransactionRequest::default()
        .with_from(Address::with_last_byte(1))
        .with_to(Address::with_last_byte(2))
        .with_input(Bytes::new());

    let opts = GethDebugTracingCallOptions::default().with_tracing_options(
        GethDebugTracingOptions::default()
            .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
            .with_call_config(CallConfig::default().with_log()),
    );

    let trace = provider
        .debug_trace_call(WithOtherFields::new(tx), BlockId::latest(), opts)
        .await
        .unwrap();

    match trace {
        GethTrace::CallTracer(frame) => {
            assert!(frame.calls.is_empty());
        }
        _ => panic!("expected CallTracer"),
    }
}

