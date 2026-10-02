//! CLI tests for `cast bal`.

use super::*;
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;
use serde_json::{Value, json};

/// A block access list covering every change kind, as `eth_getBlockAccessList` returns it.
fn block_access_list() -> Value {
    json!([
        {
            "address": "0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b",
            "storageChanges": [
                {
                    "key": "0x0",
                    "changes": [{ "index": "0x0", "value": "0x100" }],
                },
            ],
            "storageReads": ["0x1"],
            "balanceChanges": [{ "index": "0x0", "value": "0x56bc75e2d63100000" }],
            "nonceChanges": [{ "index": "0x0", "value": "0x1" }],
            "codeChanges": [{ "index": "0x0", "code": "0x6001" }],
        },
    ])
}

/// Serves `result` for both the officially specified `eth_getBlockAccessList` and the
/// `eth_getBlockAccessListByBlockNumber` extension a block tag currently resolves through.
async fn canned_endpoint(upstream: String, result: Value) -> String {
    let (endpoint, _) = spawn_rpc_proxy_canned_method(
        upstream,
        "eth_getBlockAccessListByBlockNumber",
        result.clone(),
    )
    .await;
    let (endpoint, _) =
        spawn_rpc_proxy_canned_method(endpoint, "eth_getBlockAccessList", result).await;
    endpoint
}

casttest!(bal, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = canned_endpoint(handle.http_endpoint(), block_access_list()).await;

    cmd.args(["bal", "latest", "--rpc-url", &endpoint]).assert_success().stdout_eq(str![[r#"
[
  {
    "address": "0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b",
    "storageChanges": [
      {
        "key": "0x0",
        "changes": [
          {
            "index": "0x0",
            "value": "0x100"
          }
        ]
      }
    ],
    "storageReads": [
      "0x1"
    ],
    "balanceChanges": [
      {
        "index": "0x0",
        "value": "0x56bc75e2d63100000"
      }
    ],
    "nonceChanges": [
      {
        "index": "0x0",
        "value": "0x1"
      }
    ],
    "codeChanges": [
      {
        "index": "0x0",
        "code": "0x6001"
      }
    ]
  }
]

"#]]);
});

casttest!(bal_raw, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = canned_endpoint(handle.http_endpoint(), block_access_list()).await;

    cmd.args(["bal", "latest", "--raw", "--rpc-url", &endpoint]).assert_success().stdout_eq(str![
        [r#"
0xf838f794a94f5374fce5edbc8e2a8697c15331677e6ebf0bc8c780c5c480820100c101cccb8089056bc75e2d63100000c3c28001c5c480826001

"#]
    ]);
});

casttest!(bal_not_found, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.args(["bal", "latest", "--rpc-url", &handle.http_endpoint()]).assert_failure().stderr_eq(
        str![[r#"
Error: block access list for latest not found

"#]],
    );
});
