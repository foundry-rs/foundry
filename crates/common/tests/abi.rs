//! Explorer source lookup tests.

use alloy_primitives::{Address, map::HashMap};
use axum::{Json, Router, extract::Query, routing::get};
use foundry_block_explorers::Client;
use foundry_common::abi::find_source;
use serde_json::json;

#[tokio::test]
async fn falls_back_to_proxy_source_for_unverified_implementation() {
    let proxy = Address::repeat_byte(0x11);
    let implementation = Address::repeat_byte(0x22);
    let app = Router::new().route(
        "/api",
        get(move |Query(query): Query<HashMap<String, String>>| async move {
            Json(if query["address"] == proxy.to_string() {
                json!({
                    "status": "1",
                    "message": "OK",
                    "result": [{
                        "SourceCode": "contract Proxy {}",
                        "ABI": "[]",
                        "ContractName": "Proxy",
                        "CompilerVersion": "v0.8.30+commit.73712a01",
                        "OptimizationUsed": "0",
                        "EVMVersion": "Default",
                        "Proxy": "1",
                        "Implementation": implementation
                    }]
                })
            } else {
                json!({
                    "status": "0",
                    "message": "NOTOK",
                    "result": "Contract source code not verified"
                })
            })
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
    let result = find_source(client, proxy).await;
    server.abort();
    assert_eq!(result.unwrap().items[0].contract_name, "Proxy");
}
