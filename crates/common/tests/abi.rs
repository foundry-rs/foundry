//! Explorer source lookup tests.

use alloy_primitives::{Address, map::HashMap};
use axum::{Json, Router, extract::Query, routing::get};
use foundry_block_explorers::{Client, contract::ContractMetadata, errors::EtherscanError};
use foundry_common::abi::find_source;
use serde_json::{Value, json};

const PROXY: Address = Address::repeat_byte(0x11);
const IMPLEMENTATION: Address = Address::repeat_byte(0x22);

fn source_response(name: &str, implementation: Option<Address>) -> Value {
    json!({
        "status": "1",
        "message": "OK",
        "result": [{
            "SourceCode": format!("contract {name} {{}}"),
            "ABI": "[]",
            "ContractName": name,
            "CompilerVersion": "v0.8.30+commit.73712a01",
            "OptimizationUsed": "0",
            "EVMVersion": "Default",
            "Proxy": if implementation.is_some() { "1" } else { "0" },
            "Implementation": implementation
        }]
    })
}

async fn find_proxy_source(implementation_response: Value) -> eyre::Result<ContractMetadata> {
    let app = Router::new().route(
        "/api",
        get(move |Query(query): Query<HashMap<String, String>>| {
            let response = if query["address"] == PROXY.to_string() {
                source_response("Proxy", Some(IMPLEMENTATION))
            } else {
                implementation_response.clone()
            };
            async move { Json(response) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = Client::builder()
        .with_api_url(&url)
        .unwrap()
        .with_url(&url)
        .unwrap()
        .no_proxy()
        .build()
        .unwrap();
    let result = find_source(client, PROXY).await;
    server.abort();
    result
}

#[tokio::test]
async fn falls_back_to_proxy_source_for_unverified_implementation() {
    let source = find_proxy_source(json!({
        "status": "0",
        "message": "NOTOK",
        "result": "Contract source code not verified"
    }))
    .await
    .unwrap();
    assert_eq!(source.items[0].contract_name, "Proxy");
}

#[tokio::test]
async fn follows_verified_proxy_implementation() {
    let source = find_proxy_source(source_response("Implementation", None)).await.unwrap();
    assert_eq!(source.items[0].contract_name, "Implementation");
}

#[tokio::test]
async fn propagates_proxy_implementation_errors() {
    let error = find_proxy_source(json!({
        "status": "0",
        "message": "NOTOK",
        "result": "Invalid API Key"
    }))
    .await
    .unwrap_err();
    assert!(matches!(error.downcast_ref::<EtherscanError>(), Some(EtherscanError::InvalidApiKey)));
}
