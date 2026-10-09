//! CLI tests for run commands.

use super::*;
use alloy_primitives::bytes;
use alloy_signer::SignerSync;
use foundry_test_utils::rpc::spawn_rpc_proxy_canned_method;

// <https://github.com/foundry-rs/foundry/issues/2705>
#[casttest]
fn run_succeeds(cmd: _) {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "run",
        "-v",
        "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
        "--quick",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);
}

// Regression test: pre-Berlin tx (block 12243999) must use Istanbul gas costs.
// Without correct hardfork resolution, this tx OOGs under Prague/Berlin cold SLOAD costs.
#[casttest]
fn run_pre_berlin_tx_uses_correct_spec(cmd: _) {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "run",
        "-v",
        "0xbb4dece05b8d41a2f79475f76daccf7abdd816f6813897cd02ef8509205ebecb",
        "--quick",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);
}

// Tests that `cast --disable-block-gas-limit` commands are working correctly for BSC
// <https://github.com/foundry-rs/foundry/pull/9996>
// Equivalent transaction on Binance Smart Chain Testnet:
// <https://testnet.bscscan.com/tx/0x0db4f279fc4d47dca1e6ace180f45f50c5bf12e2b968f210c217f57031e02744>
#[casttest]
#[expect(clippy::disallowed_macros, reason = "skips have to be visible in the test log")]
fn run_replays_transaction_over_block_gas_limit(cmd: _) {
    let bsc_testnet_rpc_url = next_rpc_endpoint(NamedChain::BinanceSmartChainTestnet);

    let latest_block_json: serde_json::Value = serde_json::from_str(
        &cmd.args(["block", "--rpc-url", bsc_testnet_rpc_url.as_str(), "--json"])
            .assert_success()
            .get_output()
            .stdout_lossy(),
    )
    .expect("Failed to parse latest block");

    let latest_excessive_gas_limit_tx =
        latest_block_json["transactions"].as_array().and_then(|txs| {
            txs.iter()
                .find(|tx| tx.get("gas").and_then(|gas| gas.as_str()) == Some("0x7fffffffffffffff"))
        });

    match latest_excessive_gas_limit_tx {
        Some(tx) => {
            let tx_hash =
                tx.get("hash").and_then(|h| h.as_str()).expect("Transaction missing hash");

            // The chain accepted this transaction even though its gas limit exceeds the block
            // gas limit, so replay must not re-apply the check.
            cmd.cast_fuse()
                .args(["run", "-v", tx_hash, "--quick", "--rpc-url", bsc_testnet_rpc_url.as_str()])
                .assert_success()
                .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);

            // `--disable-block-gas-limit` is now implied and must not change the outcome.
            cmd.cast_fuse()
                .args([
                    "run",
                    "-v",
                    tx_hash,
                    "--quick",
                    "--rpc-url",
                    bsc_testnet_rpc_url.as_str(),
                    "--disable-block-gas-limit",
                ])
                .assert_success()
                .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);
        }
        None => {
            eprintln!(
                "Skipping test: No transaction with gas = 0x7fffffffffffffff found in the latest block."
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/10699>
#[forgetest]
async fn cast_run_uses_chain_rpc_endpoint(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    let endpoint = handle.http_endpoint();
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .from(sender)
                .to(address!("000000000000000000000000000000000000dEaD"))
                .value(U256::ONE)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    fs::write(
        prj.root().join("foundry.toml"),
        r#"[rpc_endpoints]
mainnet = "${CAST_RUN_MAINNET_RPC_URL}"
"#,
    )
    .unwrap();

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    cmd.env("CAST_RUN_MAINNET_RPC_URL", endpoint);
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["run", &receipt.transaction_hash.to_string(), "--chain", "mainnet", "--quick"])
        .assert_success();
}

// Without BAL support, `cast run` replays the block even on a node that supports the debug API.
// The prestate tracer must be explicitly opted into via `--prestate-tracer`.
#[forgetest]
async fn cast_run_default_uses_block_replay(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let tx_hash = deploy_counter_and_set_number(&prj, &mut cmd, &api, &endpoint).await;

    cmd.cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);
}

// tests cast can decode external libraries traces with project cached selectors
#[forgetest_init]
async fn flaky_decode_external_libraries_with_cached_selectors(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    prj.add_source(
        "ExternalLib",
        r#"
import "./CounterInExternalLib.sol";
library ExternalLib {
    function updateCounterInExternalLib(CounterInExternalLib.Info storage counterInfo, uint256 counter) public {
        counterInfo.counter = counter + 1;
    }
}
   "#,
    );
    prj.add_source(
        "CounterInExternalLib",
        r#"
import "./ExternalLib.sol";
contract CounterInExternalLib {
    struct Info {
        uint256 counter;
    }
    Info info;
    constructor() {
        ExternalLib.updateCounterInExternalLib(info, 100);
    }
}
   "#,
    );
    prj.add_script(
        "CounterInExternalLibScript",
        r#"
import "forge-std/Script.sol";
import {CounterInExternalLib} from "../src/CounterInExternalLib.sol";
contract CounterInExternalLibScript is Script {
    function run() public {
        vm.startBroadcast();
        new CounterInExternalLib();
        vm.stopBroadcast();
    }
}
   "#,
    );

    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterInExternalLibScript",
    ])
    .assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(1))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Build and cache project selectors.
    cmd.forge_fuse().args(["build"]).assert_success();
    cmd.forge_fuse().args(["selectors", "cache"]).assert_success();
    // Assert cast with local artifacts can decode external lib signature.
    cmd.cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
...
Traces:
  [..] → new <unknown>@0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
    ├─ [..] [..]::updateCounterInExternalLib(0, 100) [delegatecall]
    │   └─ ← [Stop]
    └─ ← [Return] [..] bytes of code


Transaction successfully executed.
[GAS]

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/9541
#[forgetest]
async fn flaky_cast_run_impersonated_tx(cmd: _) {
    let (_api, handle) = anvil::spawn(
        NodeConfig::test()
            .with_auto_impersonate(true)
            .with_eth_rpc_url(Some("https://sepolia.base.org")),
    )
    .await;

    let http_endpoint = handle.http_endpoint();

    let provider = ProviderBuilder::new().connect_http(http_endpoint.parse().unwrap());

    // send impersonated tx
    let tx = TransactionRequest::default()
        .with_from(address!("0x041563c07028Fc89106788185763Fc73028e8511"))
        .with_to(address!("0xF38aA5909D89F5d98fCeA857e708F6a6033f6CF8"))
        .with_input(bytes!(
            "0x60fe47b1000000000000000000000000000000000000000000000000000000000000000c"
        ));

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    // run impersonated tx
    cmd.cast_fuse()
        .args(["run", &receipt.transaction_hash.to_string(), "--rpc-url", &http_endpoint])
        .assert_success();
}

// <https://github.com/foundry-rs/foundry/issues/10553>
// <https://basescan.org/tx/0x17b2de59ebd7dfd2452a3638a16737b6b65ae816c1c5571631dc0d80b63c41de>
#[casttest]
fn flaky_osaka_can_run_p256_precompile(cmd: _) {
    cmd.args([
    "run",
    "0x17b2de59ebd7dfd2452a3638a16737b6b65ae816c1c5571631dc0d80b63c41de",
    "--rpc-url",
    next_rpc_endpoint(NamedChain::Base).as_str(),
    "--quick",
    "--evm-version",
    "osaka",
])
.assert_success()
.stdout_eq(str![[r#"
Traces:
  [..] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::execute(0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    ├─ [2241] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::fallback(00) [staticcall]
    │   └─ ← [Return] 0x0000000000000000000000000b55b053230e4effb6609de652fca73fd1c29804
    ├─ [9750] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0xA12384c5E52fD646E7BC7F6B3b33A605651F566E) [staticcall]
    │   ├─ [2553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0xA12384c5E52fD646E7BC7F6B3b33A605651F566E) [delegatecall]
    │   │   └─ ← [Return] 62393 [6.239e4]
    │   └─ ← [Return] 62393 [6.239e4]
    ├─ [..] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::[..]()
    │   ├─ [..] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::unwrapAndValidateSignature(0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00) [staticcall]
    │   │   ├─ [..] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::unwrapAndValidateSignature(0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00) [delegatecall]
    │   │   │   ├─ [2369] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::pauseFlag() [staticcall]
    │   │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000
    │   │   │   ├─ [120] PRECOMPILES::sha256(0x7b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d) [staticcall]
    │   │   │   │   └─ ← [Return] 0xc13089327d3c20c0ce35f2f058c423de29977e6950e406c095e366a8fabd463f
    │   │   │   ├─ [96] PRECOMPILES::sha256(0x424242424242424242424242424242424242424242424242424242424242424201000000c13089327d3c20c0ce35f2f058c423de29977e6950e406c095e366a8fabd463f) [staticcall]
    │   │   │   │   └─ ← [Return] 0xc544bd9a4ea526dda3a008f43c21b6f0be3031b1ff71832b9876915dc91deea0
    │   │   │   ├─ [..] PRECOMPILES::p256Verify(0xc544bd9a4ea526dda3a008f43c21b6f0be3031b1ff71832b9876915dc91deea0, 100105265279889746367868033207795503835004638867404470555471132548343465058056, 7072134396011491412722047857354424388694374548663828422239323654972225288485, 86541895207843662984465061939919839471417016180185784541173973900783506472007, 70542876722349398371292179791476581686494250953390952520841573290937254949513) [staticcall]
    │   │   │   │   └─ ← [Return] true
    │   │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000011bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b
    │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000011bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b
    │   ├─ [..] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::checkAndIncrementNonce(23)
    │   │   ├─ [..] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::checkAndIncrementNonce(23) [delegatecall]
    │   │   │   └─ ← [Stop]
    │   │   └─ ← [Return]
    │   ├─ [3250] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [staticcall]
    │   │   ├─ [2553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [delegatecall]
    │   │   │   └─ ← [Return] 38539 [3.853e4]
    │   │   └─ ← [Return] 38539 [3.853e4]
    │   ├─ [16411] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::pay(1551, 0x1bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b, 0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    │   │   ├─ [15711] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::pay(1551, 0x1bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b, 0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [delegatecall]
    │   │   │   ├─ [12963] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::transfer(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, 1551)
    │   │   │   │   ├─ [12263] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::transfer(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, 1551) [delegatecall]
    │   │   │   │   │   ├─ emit Transfer(from: 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E, to: 0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, amount: 1551)
    │   │   │   │   │   └─ ← [Return] true
    │   │   │   │   └─ ← [Return] true
    │   │   │   └─ ← [Stop]
    │   │   └─ ← [Return]
    │   ├─ [1250] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [staticcall]
    │   │   ├─ [553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [delegatecall]
    │   │   │   └─ ← [Return] 40090 [4.009e4]
    │   │   └─ ← [Return] 40090 [4.009e4]
    │   ├─ [..] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::[..]()
    │   │   ├─ [..] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::execute(0x0100000000007821000100000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000201bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b)
    │   │   │   ├─ [..] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::execute(0x0100000000007821000100000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000201bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b) [delegatecall]
    │   │   │   │   ├─ [435] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::fallback()
    │   │   │   │   │   ├─ [55] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::fallback() [delegatecall]
    │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   └─ ← [Return]
    │   │   │   │   └─ ← [Stop]
    │   │   │   └─ ← [Return]
    │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000
    │   └─ ← [Stop]
    ├─  emit topic 0: 0x31e2fdd22f7eeca688d70008a7bee8e41aa5640885c2bc592419ae8d09d889f1
    │        topic 1: 0x000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e
    │        topic 2: 0x0000000000000000000000000000000000000000000000000000000000000017
    │           data: 0x00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000


Transaction successfully executed.
[GAS]

"#]]);
}

// Test cast run Celo transfer with precompiles.
#[casttest]
fn flaky_run_celo_with_precompiles(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Celo);
    cmd.args([
        "run",
        "0xa652b9f41bb1a617ea6b2835b3316e79f0f21b8264e7bcd20e57c4092a70a0f6",
        "--quick",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [17776] 0x471EcE3750Da237f93B8E339c536989b8978a438::transfer(0xD2eB2d37d238Caeff39CFA36A013299C6DbAC56A, 138000000000000000 [1.38e17])
    ├─ [12370] 0xFeA1B35f1D5f2A58532a70e7A32e6F2D3Bc4F7B1::transfer(0xD2eB2d37d238Caeff39CFA36A013299C6DbAC56A, 138000000000000000 [1.38e17]) [delegatecall]
    │   ├─ [9000] CELO_TRANSFER_PRECOMPILE::00000000(00000000000000008106680ba7095cfd8f4351a8b7041da3060afb83000000000000000000000000d2eb2d37d238caeff39cfa36a013299c6dbac56a00000000000000000000000000000000000000000000000001ea4644d3010000)
    │   │   └─ ← [Return]
    │   ├─ emit Transfer(from: 0x8106680Ba7095CfD8F4351a8B7041da3060Afb83, to: 0xD2eB2d37d238Caeff39CFA36A013299C6DbAC56A, amount: 138000000000000000 [1.38e17])
    │   └─ ← [Return] true
    └─ ← [Return] true


Transaction successfully executed.
[GAS]

"#]]);
}

// Test that `cast run --evm-version` correctly updates gas parameters for historical blocks.
// Mainnet tx 0xb856d9...d05d9647 is a Homestead-era tx (block 1,625,693).
// EXP gas pricing differs between Homestead (10 gas/byte) and Spurious Dragon+ (50 gas/byte).
// Without the fix, `set_spec()` only updated the spec discriminant but not the gas_params table,
// so the executor would use stale (latest) gas pricing even when `--evm-version homestead` is set.
#[casttest]
fn run_evm_version_updates_gas_params(cmd: _) {
    let rpc = next_http_archive_rpc_url();
    let tx = "0xb856d9c8dffeaa317d89ed6abba861d007a708c54971da91233abcd2d05d9647";

    // Run with --evm-version homestead: gas must match on-chain gasUsed (166651).
    let homestead_output = cmd
        .args(["run", tx, "--quick", "--rpc-url", rpc.as_str(), "--evm-version", "homestead"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        homestead_output.contains("Gas used: 166651"),
        "expected Homestead gas (166651), got: {homestead_output}"
    );

    // Run with --evm-version spuriousDragon: higher gas due to EXP repricing (50 vs 10 gas/byte).
    let sd_output = cmd
        .cast_fuse()
        .args(["run", tx, "--quick", "--rpc-url", rpc.as_str(), "--evm-version", "spuriousDragon"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        sd_output.contains("Gas used: 177241"),
        "expected Spurious Dragon gas (177241), got: {sd_output}"
    );
}

// Anvil can use an Elastic chain ID while still executing EVM bytecode.
#[casttest]
async fn cast_run_replays_elastic_chain_id_on_anvil(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(324u64))).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let tx_hash = provider
        .send_transaction(TransactionRequest::default().with_from(from).with_to(from).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .tx_hash()
        .to_string();

    cmd.args(["run", &tx_hash, "--rpc-url", &handle.http_endpoint()]).assert_success().stdout_eq(
        str![[r#"
...
Transaction successfully executed.
[GAS]

"#]],
    );
}

// Without Anvil metadata, retain the chain-ID-based rejection for Elastic chains.
#[casttest]
async fn cast_run_rejects_elastic_chains(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(324u64))).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let tx_hash = provider
        .send_transaction(TransactionRequest::default().with_from(from).with_to(from).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .tx_hash()
        .to_string();
    let endpoint = spawn_rpc_proxy_method_not_found_before(
        handle.http_endpoint(),
        "anvil_nodeInfo",
        usize::MAX,
    )
    .await;

    cmd.args(["run", &tx_hash, "--rpc-url", &endpoint])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: zksync executes EraVM bytecode, which cannot be replayed locally; `--debug-trace-transaction` renders the node's own trace instead

"#]]);
}

// Without Anvil metadata the endpoint identity is discovered once and reused for the environment,
// the fork, and the executor.
#[casttest]
async fn cast_run_discovers_fork_endpoint_once(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let tx_hash = provider
        .send_transaction(TransactionRequest::default().with_from(from).with_to(from).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .tx_hash()
        .to_string();
    let endpoint = spawn_rpc_proxy_method_not_found_before(
        handle.http_endpoint(),
        "anvil_nodeInfo",
        usize::MAX,
    )
    .await;
    let (endpoint, chain_ids) = spawn_rpc_proxy_recording_method(endpoint, "eth_chainId").await;
    let (endpoint, node_infos) = spawn_rpc_proxy_recording_method(endpoint, "anvil_nodeInfo").await;

    for args in [&[][..], &["--debug-trace-transaction"]] {
        chain_ids.lock().unwrap().clear();
        node_infos.lock().unwrap().clear();

        cmd.cast_fuse().args(["run", &tx_hash, "--rpc-url", &endpoint]).args(args).assert_success();

        assert_eq!(chain_ids.lock().unwrap().len(), 1, "{args:?}");
        assert_eq!(node_infos.lock().unwrap().len(), 1, "{args:?}");
    }
}

// Tracing replays exact chain history, so it ignores the number-based state opt-in.
#[casttest]
async fn cast_run_keeps_hash_addressed_state(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let tx_hash = provider
        .send_transaction(TransactionRequest::default().with_from(from).with_to(from).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .transaction_hash()
        .to_string();
    let mut endpoint = handle.http_endpoint();
    for method in ["anvil_nodeInfo", "anvil_metadata"] {
        endpoint = spawn_rpc_proxy_method_not_found_before(endpoint, method, usize::MAX).await;
    }
    let mut recorded = Vec::new();
    for method in ["eth_getBalance", "eth_getTransactionCount", "eth_getCode", "eth_getStorageAt"] {
        let (next_endpoint, requests) = spawn_rpc_proxy_recording_method(endpoint, method).await;
        endpoint = next_endpoint;
        recorded.push(requests);
    }

    cmd.env("FOUNDRY_FORK_STATE_BY_NUMBER", "true");
    cmd.args(["run", &tx_hash, "--rpc-url", &endpoint]).assert_success();

    let mut reads = 0;
    for requests in recorded {
        let requests = requests.lock().unwrap();
        reads += requests.len();
        assert!(
            requests.iter().all(|params| params.as_array().unwrap().last().unwrap().is_object()),
            "{requests:?}"
        );
    }
    assert!(reads > 0);
}

// A replay that does not reproduce the transaction's receipt must say so. The `--evm-version`
// overrides stand in for rules the replay does not model: Shanghai predates the `MCOPY` the first
// transaction executes, and Cancun predates the EIP-7623 calldata floor that prices the second.
#[casttest]
async fn cast_run_warns_on_receipt_mismatch(cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    // MCOPY(0, 0, 0) STOP
    api.anvil_set_code(Address::with_last_byte(0xaa), bytes!("0x6000600060005e00")).await.unwrap();
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let mut tx_hashes = Vec::new();
    for (to, input) in [
        (Address::with_last_byte(0xaa), Bytes::new()),
        (Address::with_last_byte(0xcc), vec![1u8; 1000].into()),
    ] {
        let receipt = provider
            .send_transaction(
                TransactionRequest::default().with_from(from).with_to(to).with_input(input).into(),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        tx_hashes.push(receipt.transaction_hash().to_string());
    }
    let [mcopy_tx, floor_tx] = &tx_hashes[..] else { unreachable!() };

    for tx_hash in [mcopy_tx, floor_tx] {
        cmd.cast_fuse()
            .args(["run", tx_hash, "--rpc-url", &endpoint, "--evm-version", "prague"])
            .assert_success()
            .stderr_eq(str![[r#"
Executing previous transactions from the block.

"#]]);
    }

    cmd.cast_fuse()
        .args(["run", mcopy_tx, "--rpc-url", &endpoint, "--evm-version", "shanghai"])
        .assert_success()
        .stderr_eq(str![[r#"
Executing previous transactions from the block.
Error: Transaction failed.
Warning: the replay does not match the transaction's receipt: it succeeded on-chain but reverted in the replay. The chain may apply rules the replay does not model; `--debug-trace-transaction` shows the node's own trace if it exposes the `debug` namespace.

"#]]);

    cmd.cast_fuse()
        .args(["run", floor_tx, "--rpc-url", &endpoint, "--evm-version", "cancun"])
        .assert_success()
        .stderr_eq(str![[r#"
Executing previous transactions from the block.
Warning: the replay does not match the transaction's receipt: it used 61000 gas on-chain but 37000 in the replay. The chain may apply rules the replay does not model; `--debug-trace-transaction` shows the node's own trace if it exposes the `debug` namespace.

"#]]);
}

// Forked state reports an account that does not exist as empty, but replay must not refund the
// EIP-7702 authorization of an authority that did not exist before the transaction.
#[casttest]
async fn cast_run_charges_fresh_eip7702_authority(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let authority = PrivateKeySigner::random();
    let authorization = Authorization {
        chain_id: U256::from(31337u64),
        address: address!("0x000000000000000000000000000000000000dEaD"),
        nonce: 0,
    };
    let signature = authority.sign_hash_sync(&authorization.signature_hash()).unwrap();
    let tx = TransactionRequest {
        authorization_list: Some(vec![authorization.into_signed(signature)]),
        ..Default::default()
    }
    .with_from(from)
    .with_to(address!("0x0000000000000000000000000000000000001234"))
    .with_input(hex!("12345678"));
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(receipt.gas_used(), 46_064);

    let output = cmd
        .args([
            "run",
            &receipt.tx_hash().to_string(),
            "--rpc-url",
            &handle.http_endpoint(),
            "--evm-version",
            "prague",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("Gas used: 46064"), "{output}");
}

// Prints the ERC-8021 attribution codes appended to the transaction calldata.
#[casttest]
async fn cast_run_prints_erc8021_attribution(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    // ERC-8021 schema 2 test vector with app, wallet and service codes.
    let tx = TransactionRequest::default().with_from(from).with_to(from).with_input(hex!(
        "a361616762617365617070617765707269767961738269666c617368626f747365746974616e00260280218021802180218021802180218021"
    ));
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    cmd.args([
        "run",
        &receipt.transaction_hash().to_string(),
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]
ERC-8021 attribution: baseapp (app), privy (wallet), flashbots (service), titan (service)

"#]]);
}

// The transaction returned by the RPC must be the one that was requested.
#[casttest]
async fn cast_run_rejects_mismatched_transaction(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let mut tx_hashes = Vec::new();
    for to in [Address::with_last_byte(0xaa), Address::with_last_byte(0xbb)] {
        let receipt = provider
            .send_transaction(TransactionRequest::default().with_from(from).with_to(to).into())
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        tx_hashes.push(receipt.transaction_hash());
    }
    let [requested, returned] = tx_hashes[..] else { unreachable!() };
    let returned_tx = provider.get_transaction_by_hash(returned).await.unwrap().unwrap();
    let (endpoint, _) = spawn_rpc_proxy_canned_method(
        handle.http_endpoint(),
        "eth_getTransactionByHash",
        serde_json::to_value(returned_tx).unwrap(),
    )
    .await;

    for args in [&[][..], &["--debug-trace-transaction"]] {
        cmd.cast_fuse()
            .args(["run", &requested.to_string(), "--rpc-url", &endpoint])
            .args(args)
            .assert_failure()
            .stderr_eq(format!(
                "Error: RPC returned transaction {returned} for requested {requested}\n"
            ));
    }
}

// A forked Anvil replays upstream transactions with the hardfork of the upstream chain: Cancun
// charges 21000 where EIP-2780 charges 15000.
#[casttest]
async fn cast_run_upstream_tx_through_amsterdam_fork(cmd: _) {
    for (hardfork, gas) in
        [(EthereumHardfork::Cancun, 21_000), (EthereumHardfork::Amsterdam, 15_000)]
    {
        let (upstream_api, upstream) =
            anvil::spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let mut accounts = upstream.dev_accounts();
        let tx = TransactionRequest::default()
            .with_from(accounts.next().unwrap())
            .with_to(accounts.next().unwrap());
        let receipt = upstream
            .http_provider()
            .send_transaction(tx.into())
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert_eq!(receipt.gas_used, gas);
        // Fork after the transaction's block, so that the fork serves it as upstream history.
        upstream_api.evm_mine(None).await.unwrap();

        let (_, fork) = anvil::spawn(
            NodeConfig::test()
                .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
                .with_eth_rpc_url(Some(upstream.http_endpoint())),
        )
        .await;
        cmd.cast_fuse()
            .args([
                "run",
                &receipt.transaction_hash.to_string(),
                "--rpc-url",
                &fork.http_endpoint(),
            ])
            .with_no_redact()
            .assert_success()
            .stdout_eq(format!("...\nTransaction successfully executed.\nGas used: {gas}\n"));
    }
}
