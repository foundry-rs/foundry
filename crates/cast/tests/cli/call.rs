//! CLI tests for call commands.

use super::*;
use alloy_primitives::bytes;

#[forgetest_init]
async fn cast_call_custom_override(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    prj.initialize_default_contracts();
    prj.add_source(
        "Counter",
        r#"
contract Counter {
    uint256 public number;

    function getBalance(address target) public returns (uint256) {
        return target.balance;
    }
}
   "#,
    );

    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);

    // Override balance, `getBalance()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-balance",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x1111",
            "getBalance(address)(uint256)",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4369

"#]]);

    // Override balance, `getBalance()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-balance",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x1111",
            "getBalance(address)(uint256)",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [747] 0x5FbDB2315678afecb367f032d93F642f64180aa3::getBalance(0x5FbDB2315678afecb367f032d93F642f64180aa3)
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001111


Transaction successfully executed.
[GAS]

"#]]);

    // Override code with
    // contract Counter {
    //     uint256 public number1;
    // }
    // Calling `number()` should fail.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "number()(uint256)",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: server returned an error response: error code 3: execution reverted, data: "0x"

"#]]);

    // Override code with
    // contract Counter {
    //     uint256 public number1;
    // }
    // Calling `number()` should revert.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "number()(uint256)",
            "--trace"
        ])
        .assert_success()
        .stderr_eq(str![[r#"
Error: Transaction failed.

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
8738

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
            "--trace"
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number1()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000002222


Transaction successfully executed.
[GAS]

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state-diff",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
8738

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state-diff",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number1()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000002222


Transaction successfully executed.
[GAS]

"#]]);
}

#[casttest]
fn correct_json_serialization(cmd: _) {
    let rpc = next_http_archive_rpc_url();
    // cast calldata "decimals()"
    let calldata = "0x313ce567";
    let tokens = [
        "0xdac17f958d2ee523a2206206994597c13d831ec7", // USDT
        "0x6b175474e89094c44da98b954eedeac495271d0f", // DAI
        "0x6b175474e89094c44da98b954eedeac495271d0f", // WETH
    ];
    let calldata_args = format!(
        "[{}]",
        tokens
            .iter()
            .map(|token| format!("({token},false,{calldata})"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let args = vec![
        "call",
        "--json",
        "--rpc-url",
        rpc.as_str(),
        "0xcA11bde05977b3631167028862bE2a173976CA11",
        "aggregate3((address,bool,bytes)[])((bool,bytes)[])",
        &calldata_args,
    ];
    let expected_output = json!([[
        [true, "0x0000000000000000000000000000000000000000000000000000000000000006"],
        [true, "0x0000000000000000000000000000000000000000000000000000000000000012"],
        [true, "0x0000000000000000000000000000000000000000000000000000000000000012"]
    ]]);
    let output: serde_json::Value =
        serde_json::from_slice(&cmd.args(args).assert_success().get_output().stdout)
            .expect("not valid json");
    assert_eq!(output, expected_output);
}

#[casttest]
fn call_eip7702_auth_disclosure_declined(cmd: _) {
    cmd.args([
        "call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);
}

// Address-based authorizations are signed by the configured signer even when `--from` is set.
#[casttest]
fn call_eip7702_address_auth_with_from_uses_signer(cmd: _) {
    cmd.args([
        "call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);
}

#[casttest]
fn call_eip7702_auth_disclosure_requires_signer(cmd: _) {
    cmd.args([
        "call",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--nonce",
        "0",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: No signer available to sign authorization. Provide a pre-signed authorization (hex-encoded) instead.

"#]]);
}

#[casttest]
async fn call_eip7702_auth_disclosure_accepted_and_forced(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();
    let delegate_code = bytes!("0x602a5f5260205ff3");
    api.anvil_set_code(address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"), delegate_code)
        .await
        .unwrap();
    api.anvil_set_code(
        address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        bytes!("0x602a5f5260205ff3"),
    )
    .await
    .unwrap();
    let args = [
        "call",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ];

    cmd.args(args)
        .arg("--json")
        .stdin("y\n")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]]);

    cmd.cast_fuse()
        .args(args)
        .arg("--force")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![""]);

    cmd.cast_fuse()
        .args(args)
        .args(["--quiet", "--force"])
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![""]);

    let signer: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    let auth = Authorization {
        chain_id: U256::from(31337),
        address: address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        nonce: 0,
    };
    let signature = signer.sign_hash(&auth.signature_hash()).await.unwrap();
    let encoded_auth = hex::encode_prefixed(alloy_rlp::encode(auth.into_signed(signature)));

    cmd.cast_fuse()
        .args([
            "call",
            &signer.address().to_string(),
            "--auth",
            &encoded_auth,
            "--from",
            &signer.address().to_string(),
            "--rpc-url",
            &endpoint,
        ])
        .stdin("y\n")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]]);

    cmd.cast_fuse().args(args).arg("--trace").assert_success().stderr_eq(str![""]);
    cmd.cast_fuse()
        .args(args)
        .args(["--trace", "--access-list", "[]"])
        .assert_success()
        .stderr_eq(str![""]);

    // With a pre-signed authorization, `ETH_FROM` is the sender and `ETH_KEYSTORE` is not used.
    let keystore = format!(
        "{}/tests/fixtures/keystore/UTC--2022-10-30T06-51-20.130356000Z--560d246fcddc9ea98a8b032c9a2f474efb493c28",
        env!("CARGO_MANIFEST_DIR")
    );
    cmd.cast_fuse().env("ETH_KEYSTORE", keystore);
    cmd.env("ETH_FROM", signer.address().to_string());
    cmd.args([
        "call",
        &signer.address().to_string(),
        "--auth",
        &encoded_auth,
        "--force",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
    .stderr_eq(str![""]);
}

#[casttest]
fn call_eip7702_auth_disclosure_routing(cmd: _) {
    let base_args = [
        "call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ];

    cmd.args(base_args)
        .arg("--debug-trace-call")
        .stdin("n\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    cmd.cast_fuse()
        .args(base_args)
        .args(["--trace", "--access-list"])
        .stdin("n\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    cmd.cast_fuse()
        .args(base_args)
        .arg("--quiet")
        .assert_failure()
        .stderr_eq(str![[r#"
Error: EIP-7702 authorization disclosure requires confirmation; pass `--force` to continue with `--quiet`

"#]]);

    cmd.cast_fuse().args(base_args).arg("--curl").assert_failure().stdout_eq(str![""]).stderr_eq(
        str![[r#"
Error: --auth cannot be combined with --curl

"#]],
    );
}

// https://github.com/foundry-rs/foundry/issues/11521
// `--delegate` must run the destination's code against the sender's storage. The destination's
// runtime returns slot 0, and only the sender holds a value there, so a plain call returns zero
// and the delegated call returns the sender's value.
#[casttest]
async fn cast_call_delegate_uses_sender_storage(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d0";
    let to = "0x00000000000000000000000000000000000000d1";

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--delegate",
            // runtime: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
            "--override-code",
            &format!("{to}:0x60005460005260206000f3"),
            "--override-state",
            &format!("{from}:0x0:0x1234"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Without `--delegate` the same call runs against the destination's own empty storage.
    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--override-code",
            &format!("{to}:0x60005460005260206000f3"),
            "--override-state",
            &format!("{from}:0x0:0x1234"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0

"#]]);
}

// A code override on the sender is what `--delegate` installs, so the two cannot both win.
#[casttest]
async fn cast_call_delegate_rejects_sender_code_override(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d2";
    let to = "0x00000000000000000000000000000000000000d3";

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "--from",
            from,
            "--delegate",
            "--override-code",
            &format!("{to}:0x60005460005260206000f3,{from}:0x00"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: `--delegate` conflicts with `--override-code` for the sender 0x00000000000000000000000000000000000000D2

"#]]);
}

// The primary `--delegate` path reads the destination's runtime code from the node: no override
// flags are involved, the delegated code and the sender's storage both come from chain state.
#[casttest]
async fn cast_call_delegate_fetches_code_from_node(cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d4";
    let to = "0x00000000000000000000000000000000000000d5";

    // runtime: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), bytes!("0x60005460005260206000f3")).await.unwrap();
    api.anvil_set_storage_at(from.parse().unwrap(), U256::ZERO, B256::from(U256::from(0x1234)))
        .await
        .unwrap();

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--delegate",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // A delegated call that returns no data must not trip the empty-code warning: the sender has
    // no on-chain code, but the delegate override guarantees executable code.
    let void = "0x00000000000000000000000000000000000000d6";
    // runtime: STOP
    api.anvil_set_code(void.parse().unwrap(), bytes!("0x00")).await.unwrap();
    cmd.cast_fuse()
        .args(["call", void, "--from", from, "--delegate", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
0x

"#]])
        .stderr_eq(str![""]);
}

// Documents the identity semantics: the delegated code observes the sender itself as
// `msg.sender`, not the delegating contract's caller as an on-chain `delegatecall` would.
#[casttest]
async fn cast_call_delegate_msg_sender_is_from(cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d9";
    let to = "0x00000000000000000000000000000000000000da";

    // runtime: CALLER PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), bytes!("0x3360005260206000f3")).await.unwrap();

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "sender()(address)",
            "--from",
            from,
            "--delegate",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x00000000000000000000000000000000000000D9

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/17388
// `--from` is the sender even when a keystore is configured.
#[casttest]
async fn cast_call_from_skips_keystore_unlock(cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d9";
    let to = "0x00000000000000000000000000000000000000da";
    let keystore = format!(
        "{}/tests/fixtures/keystore/UTC--2022-10-30T06-51-20.130356000Z--560d246fcddc9ea98a8b032c9a2f474efb493c28",
        env!("CARGO_MANIFEST_DIR")
    );

    // runtime: CALLER PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), bytes!("0x3360005260206000f3")).await.unwrap();

    cmd.cast_fuse().env("ETH_KEYSTORE", keystore);
    cmd.args([
        "call",
        to,
        "sender()(address)",
        "--from",
        from,
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x00000000000000000000000000000000000000D9

"#]]);
}

// A Tempo access key keeps its root account as the sender even when `--from` is set.
#[casttest]
async fn cast_call_tempo_access_key_root_account_overrides_from(cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let to = "0x00000000000000000000000000000000000000da";

    // runtime: CALLER PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), bytes!("0x3360005260206000f3")).await.unwrap();

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "sender()(address)",
            "--tempo.access-key",
            "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0",
            "--tempo.root-account",
            "0x00000000000000000000000000000000000000d8",
            "--from",
            "0x00000000000000000000000000000000000000d9",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x00000000000000000000000000000000000000d8

"#]]);
}

// `--delegate` needs runtime code at the destination; a codeless address is an explicit error.
#[casttest]
async fn cast_call_delegate_no_code_destination(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000db",
            "number()(uint256)",
            "--from",
            "0x00000000000000000000000000000000000000dc",
            "--delegate",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: `--delegate` destination 0x00000000000000000000000000000000000000db has no code to delegate to

"#]]);
}

// `--curl` builds the request offline, but the delegate override needs the destination's code
// from the node.
#[casttest]
fn cast_call_delegate_rejects_curl(cmd: _) {
    cmd.args([
        "call",
        "0x00000000000000000000000000000000000000dd",
        "--from",
        "0x00000000000000000000000000000000000000de",
        "--delegate",
        "--curl",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: --delegate cannot be combined with --curl

"#]]);
}

// `dirs::home_dir()` ignores `HOME` on Windows, so the signature cache cannot be isolated there.
#[cfg(not(windows))]
#[casttest]
async fn cast_call_decodes_custom_error(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    let signature = "RequestLimitExceeded(uint256,uint256)";
    let selector = keccak256(signature);
    let mut revert_data = selector[..4].to_vec();
    revert_data.extend((U256::from(5), U256::from(3)).abi_encode());

    // Runtime bytecode that copies the appended revert payload into memory and reverts with it.
    let payload_len = u8::try_from(revert_data.len()).unwrap();
    let mut runtime =
        vec![0x60, payload_len, 0x60, 0x0a, 0x5f, 0x39, 0x60, payload_len, 0x5f, 0xfd];
    runtime.extend(revert_data);

    // Isolate and seed the signature cache so decoding is deterministic and offline.
    let home = prj.root().join("home");
    let cache_dir = home.join(".foundry/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let selector = hex::encode_prefixed(&selector[..4]);
    let mut errors = serde_json::Map::new();
    errors.insert(selector, json!(signature));
    fs::write(
        cache_dir.join("signatures"),
        serde_json::to_vec(&json!({
            "functions": {},
            "errors": errors,
            "events": {},
        }))
        .unwrap(),
    )
    .unwrap();

    let target = "0x000000000000000000000000000000000000dead";
    let code_override = format!("{target}:{}", hex::encode_prefixed(runtime));
    let endpoint = handle.http_endpoint();

    cmd.env("HOME", &home);
    cmd.env("FOUNDRY_OFFLINE", "true");
    cmd.args([
        "call",
        target,
        "--data",
        "0x",
        "--override-code",
        &code_override,
        "--rpc-url",
        &endpoint,
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: execution reverted: RequestLimitExceeded(5, 3)

Context:
- server returned an error response:[..]

"#]]);

    cmd.cast_fuse();
    cmd.env("HOME", &home);
    cmd.env("FOUNDRY_OFFLINE", "true");
    cmd.args([
            "call",
            target,
            "--data",
            "0x",
            "--override-code",
            &code_override,
            "--rpc-url",
            &endpoint,
            "--json",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
{"schema_version":1,"success":false,"data":null,"errors":[{"level":"error","code":"cast.error","message":"execution reverted: RequestLimitExceeded(5, 3)"},{"level":"error","code":"cast.error.context","message":"server returned an error response:[..]"}],"warnings":[]}

"#]])
        .stderr_eq(str![""]);
}

// <https://github.com/foundry-rs/foundry/issues/10705>
#[casttest]
fn cast_call_return_array_of_tuples(cmd: _) {
    cmd.args([
        "call",
        "0x198FC70Dfe05E755C81e54bd67Bff3F729344B9b",
        "facets() returns ((address,bytes4[])[])",
        "--rpc-url",
        "https://rpc.viction.xyz",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[[..]]

"#]]);
}

// <https://github.com/foundry-rs/foundry/issues/7541>
#[casttest]
fn cast_call_on_contract_with_no_code_prints_warning(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stderr_eq(str![[r#"
Warning: Contract code is empty

"#]])
    .stdout_eq(str![[r#"
0x

"#]]);
}

// tests that cast call properly applies state diff override
// <https://github.com/foundry-rs/foundry/issues/10930>
#[casttest]
fn cast_call_can_override_state_diff(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "--data",
        "0x",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B",
        "--override-state-diff",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B:1:1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x1337

"#]]);
    cmd.args(["--trace"]).assert_success().stdout_eq(str![[r#"
Traces:
  [7681] 0x1EA77b250eF79e917A5A637D5BB82D0980653F1B::fallback()
    ├─ [2275] 0xe537cb8a46Bd179c0C36aB7E3Fdecd759C8B80fc::fallback() [delegatecall]
    │   └─ ← [Return] 0x1337
    └─ ← [Return] 0x1337


Transaction successfully executed.
[GAS]

"#]]);
}

// Test that cast call accepts negative numbers as function arguments
#[casttest]
fn cast_call_negative_numbers(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    // Test with negative int parameter - should not treat -456789 as a flag
    cmd.args([
        "call",
        "0xAbCdEf1234567890aBcDeF1234567890aBcDeF12",
        "processValue(int128)",
        "-456789",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
}

// Test negative numbers with multiple parameters
#[casttest]
fn cast_call_multiple_negative_numbers(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
        "calculateDelta(int64,int32,uint16)",
        "-987654321",
        "-42",
        "65535",
    ])
    .assert_success();
}

// Test negative numbers mixed with flags
#[casttest]
fn cast_call_negative_with_flags(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--trace", // flag before
        "0x9876543210FeDcBa9876543210FeDcBa98765432",
        "updateBalance(int256)",
        "-777888",
        "--rpc-url",
        rpc.as_str(), // flag after
    ])
    .assert_success();
}

// Test that actual invalid flags are still caught
#[casttest]
fn cast_call_invalid_flag_still_caught(cmd: _) {
    cmd.args([
        "call",
        "--invalid-flag", // This should be caught as invalid
        "0x5555555555555555555555555555555555555555",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
error: unexpected argument '--invalid-flag' found

  tip: to pass '--invalid-flag' as a value, use '-- --invalid-flag'

Usage: cast[..] call [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]

For more information, try '--help'.

"#]]);
}

// tests that cast call properly applies multiple state diff overrides
// <https://github.com/foundry-rs/foundry/issues/11551>
#[casttest]
fn cast_call_can_override_several_state_diff(cmd: _) {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "call",
        "--trace",
        "--from",
        "0xf6F444fD3B0088c1375671c05A7513661beFa4e6",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332",
        "--rpc-url",
        rpc.as_str(),
        "--block",
        "23290753",
        "--data",
        "0xe75235b8",
        "--override-state-diff",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xf0af0268363540b847b4c07f2f9a0401c607c1b11ebca511724a71755dfd4137:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:4:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8:0,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xb104e0b93118902c651344349b610029d694cfdec91c589c91ebafbcd0289947:0",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
  [..] 0x5EA1d9A6dDC3A0329378a327746D71A2019eC332::getThreshold()
...

"#]]);

    cmd.cast_fuse().args([
        "call",
        "--trace",
        "--from",
        "0x2066901073a33ba2500274704aB04763875cF210",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332",
        "--rpc-url",
        rpc.as_str(),
        "--block",
        "23290753",
        "--data",
        "0x2f54bf6e0000000000000000000000002066901073a33ba2500274704ab04763875cf210",
        "--override-state-diff",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xf0af0268363540b847b4c07f2f9a0401c607c1b11ebca511724a71755dfd4137:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:4:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8:0,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xb104e0b93118902c651344349b610029d694cfdec91c589c91ebafbcd0289947:0",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
  [..] 0x5EA1d9A6dDC3A0329378a327746D71A2019eC332::isOwner(0x2066901073a33ba2500274704aB04763875cF210)
...
"#]]);
}

// tests that the --jwt-secret flag outputs a valid curl command with Authorization header
#[casttest]
fn curl_call_with_jwt(cmd: _) {
    let rpc = "https://eth.example.com";
    let jwt_secret = "cabee703106087906e50f3e75a6ddbab60809f980511d1d1548d449d52220795";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args([
            "call",
            to,
            "balanceOf(address)(uint256)",
            to,
            "--rpc-url",
            rpc,
            "--jwt-secret",
            jwt_secret,
            "--curl",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("-H 'Content-Type: application/json'"));
    assert!(output.contains("eth_call"));
    assert!(output.contains("jsonrpc"));
    assert!(output.contains(rpc));

    let jwt = output
        .split("Authorization: Bearer ")
        .nth(1)
        .expect("missing Authorization header")
        .split('\'')
        .next()
        .expect("malformed Authorization header");
    let secret = JwtSecret::from_hex(jwt_secret).unwrap();
    secret.validate(jwt).unwrap();
}

// tests that the --curl flag outputs a valid curl command for cast call
#[casttest]
fn curl_call(cmd: _) {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["call", to, "balanceOf(address)(uint256)", to, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
}

#[casttest]
fn curl_call_eip1559_fees(cmd: _) {
    cmd.args([
        "call",
        "0xdead000000000000000000000000000000000000",
        "--rpc-url",
        "http://127.0.0.1:1",
        "--curl",
        "--gas-price",
        "2gwei",
        "--priority-gas-price",
        "1gwei",
        "--from",
        "0xbeef000000000000000000000000000000000000",
        "--value",
        "1",
        "--gas-limit",
        "100000",
        "--nonce",
        "7",
        "--data",
        "0x1234",
    ])
    .assert_success()
    .stderr_eq(str![""])
    .stdout_eq(str![[r#"
curl -X POST -H 'Content-Type: application/json' --data-raw '{"jsonrpc":"2.0","method":"eth_call","params":[{"from":"0xbeef000000000000000000000000000000000000","to":"0xdead000000000000000000000000000000000000","maxFeePerGas":"0x77359400","maxPriorityFeePerGas":"0x3b9aca00","gas":"0x186a0","value":"0x1","data":"0x1234","nonce":"0x7"},"latest"],"id":1}' 'http://127.0.0.1:1'

"#]]);
}

#[casttest]
fn curl_call_legacy_fees(prj: _, cmd: _) {
    for configured in [false, true] {
        if configured {
            prj.create_file("foundry.toml", "[profile.default]\nchain_id = \"fantom\"\n");
        }
        cmd.cast_fuse().current_dir(prj.root()).args([
            "call",
            "0xdead000000000000000000000000000000000000",
            "--rpc-url",
            "http://127.0.0.1:1",
            "--curl",
            "--gas-price",
            "2gwei",
            "--priority-gas-price",
            "1gwei",
        ]);
        if !configured {
            cmd.arg("--legacy");
        }
        cmd.assert_success().stderr_eq(str![""]).stdout_eq(str![[r#"
curl -X POST -H 'Content-Type: application/json' --data-raw '{"jsonrpc":"2.0","method":"eth_call","params":[{"to":"0xdead000000000000000000000000000000000000","gasPrice":"0x77359400","data":"0x"},"latest"],"id":1}' 'http://127.0.0.1:1'

"#]]);
    }
}

#[casttest]
fn curl_call_accepts_named_chain_config(prj: _, cmd: _) {
    let rpc = "https://eth.example.com";
    prj.create_file("foundry.toml", "[profile.default]\nchain_id = \"sepolia\"\n");

    cmd.current_dir(prj.root())
        .args(["call", "0xdead000000000000000000000000000000000000", "--rpc-url", rpc, "--curl"])
        .assert_success()
        .stderr_eq(str![""]);
}

// https://github.com/foundry-rs/foundry/issues/11584
// Tests that invalid hex with uppercase 0X prefix also produces clear error
#[casttest]
fn cast_call_invalid_hex_uppercase_prefix(cmd: _) {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0xdead000000000000000000000000000000000000",
        "--data",
        "0X1", // Invalid: odd length hex with uppercase prefix
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: Invalid hex calldata '0X1': odd number of digits

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/11584
// Tests that invalid hex calldata (odd length) produces a clear error message
#[casttest]
fn cast_call_invalid_hex_calldata_error(cmd: _) {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0xdead000000000000000000000000000000000000",
        "--data",
        "0x0", // Invalid: odd length hex
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: Invalid hex calldata '0x0': odd number of digits

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/11584
// Tests that valid hex calldata works correctly
#[casttest]
fn cast_call_valid_hex_calldata(cmd: _) {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0xdead000000000000000000000000000000000000",
        "--data",
        "0x00", // Valid: even length hex
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
}

#[casttest]
fn curl_call_rejects_browser_wallet(cmd: _) {
    let stderr = cmd
        .args(["call", "0xdead000000000000000000000000000000000000", "--browser", "--curl"])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("--browser cannot be combined with --curl; use --from <ADDRESS>"),
        "unexpected stderr:\n{stderr}"
    );
}

#[casttest]
async fn curl_call_rejects_signed_authorization(cmd: _) {
    let signer: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    let auth = Authorization {
        chain_id: U256::from(31337),
        address: address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        nonce: 0,
    };
    let signature = signer.sign_hash(&auth.signature_hash()).await.unwrap();
    let encoded_auth = hex::encode_prefixed(alloy_rlp::encode(auth.into_signed(signature)));

    for trace_args in [&[][..], &["--debug-trace-call"][..]] {
        cmd.cast_fuse()
            .args([
                "call",
                "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
                "--auth",
                &encoded_auth,
                "--rpc-url",
                "http://127.0.0.1:1",
                "--curl",
            ])
            .args(trace_args)
            .assert_failure()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Error: --auth cannot be combined with --curl

"#]]);
    }
}
