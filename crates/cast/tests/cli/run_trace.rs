//! CLI tests for run trace commands.

use super::*;

// <https://github.com/foundry-rs/foundry/issues/3473>
casttest!(flaky_test_non_mainnet_traces, |prj, cmd| {
    prj.clear();
    cmd.args([
        "run",
        "0xa003e419e2d7502269eb5eda56947b580120e00abfd5b5460d08f8af44a0c24f",
        "--rpc-url",
        next_rpc_endpoint(NamedChain::Optimism).as_str(),
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [33841] FiatTokenProxy::fallback(0x111111125421cA6dc452d289314280a0f8842A65, 164054805 [1.64e8])
    ├─ [26673] FiatTokenV2_2::approve(0x111111125421cA6dc452d289314280a0f8842A65, 164054805 [1.64e8]) [delegatecall]
    │   ├─ emit Approval(owner: 0x9a95Af47C51562acfb2107F44d7967DF253197df, spender: 0x111111125421cA6dc452d289314280a0f8842A65, amount: 164054805 [1.64e8])
    │   └─ ← [Return] true
    └─ ← [Return] true
...

"#]])
    .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);
});

// tests cast can decode traces when using project artifacts
forgetest_async!(decode_traces_with_project_artifacts, |prj, cmd| {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "LocalProjectContract",
        r#"
contract LocalProjectContract {
    event LocalProjectContractCreated(address owner);

    constructor() {
        emit LocalProjectContractCreated(msg.sender);
    }
}
   "#,
    );
    prj.add_script(
        "LocalProjectScript",
        r#"
import "forge-std/Script.sol";
import {LocalProjectContract} from "../src/LocalProjectContract.sol";

contract LocalProjectScript is Script {
    function run() public {
        vm.startBroadcast();
        new LocalProjectContract();
        vm.stopBroadcast();
    }
}
   "#,
    );

    cmd.args([
        "script",
        "--no-dynamic-test-linking",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "LocalProjectScript",
    ]);

    cmd.assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Assert cast with local artifacts from outside the project.
    cmd.cast_fuse()
        .args(["run", "--la", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
Nothing to compile

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);

    // Run cast from project dir.
    cmd.cast_fuse().set_current_dir(prj.root());

    // Assert cast without local artifacts cannot decode traces.
    cmd.cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] → new <unknown>@0x5FbDB2315678afecb367f032d93F642f64180aa3
    ├─  emit topic 0: 0xa7263295d3a687d750d1fd377b5df47de69d7db8decc745aaa4bbee44dc1688d
    │           data: 0x000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266
    └─ ← [Return] 62 bytes of code


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);

    // Assert cast with local artifacts can decode traces.
    cmd.cast_fuse()
        .args(["run", "--la", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped
Traces:
  [..] → new LocalProjectContract@0x5FbDB2315678afecb367f032d93F642f64180aa3
    ├─ emit LocalProjectContractCreated(owner: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
    └─ ← [Return] 62 bytes of code


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);
});

// `cast run` must replay a transaction's block prefix without changing the trace requested for the
// selected transaction. A single block covers a deployment in the first position, a revert in the
// middle, and an internally traced state change in the last position.
forgetest_async!(cast_run_fork_traces_only_target_transaction, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "ReplayTarget",
        r#"
contract ReplayTarget {
    uint256 public number;

    constructor() {
        number = 1;
    }

    function revertingIncrement() external {
        number++;
        revert("expected revert");
    }

    function increment() external {
        _increment();
    }

    function _increment() internal {
        number++;
    }
}
"#,
    );
    let bytecode = cmd
        .forge_fuse()
        .args(["inspect", "ReplayTarget", "bytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let bytecode = Bytes::from_str(bytecode.trim()).unwrap();

    api.anvil_set_auto_mine(false).await.unwrap();
    let nonce = provider.get_transaction_count(sender).await.unwrap();
    let target = sender.create(nonce);
    let deployment = provider
        .send_transaction(
            TransactionRequest::default()
                .from(sender)
                .with_deploy_code(bytecode)
                .nonce(nonce)
                .gas_limit(1_000_000)
                .into(),
        )
        .await
        .unwrap();
    let revert = provider
        .send_transaction(
            TransactionRequest::default()
                .from(sender)
                .to(target)
                .with_input(Bytes::copy_from_slice(&keccak256(b"revertingIncrement()")[..4]))
                .nonce(nonce + 1)
                .gas_limit(1_000_000)
                .into(),
        )
        .await
        .unwrap();
    let increment = provider
        .send_transaction(
            TransactionRequest::default()
                .from(sender)
                .to(target)
                .with_input(Bytes::copy_from_slice(&keccak256(b"increment()")[..4]))
                .nonce(nonce + 2)
                .gas_limit(1_000_000)
                .into(),
        )
        .await
        .unwrap();
    let tx_hashes = [*deployment.tx_hash(), *revert.tx_hash(), *increment.tx_hash()];
    api.mine_one().await.unwrap();

    for (index, tx_hash) in tx_hashes.iter().enumerate() {
        let block_tx = api
            .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(index))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(block_tx.tx_hash(), *tx_hash);
    }
    assert!(!api.transaction_receipt(tx_hashes[1]).await.unwrap().unwrap().status());

    cmd.set_current_dir(prj.root());
    let deployment_output = cmd
        .cast_fuse()
        .args(["run", &tx_hashes[0].to_string(), "--rpc-url", &endpoint, "--with-local-artifacts"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(deployment_output.contains("new ReplayTarget"));
    assert!(deployment_output.contains("Transaction successfully executed."));

    let revert_output = cmd
        .cast_fuse()
        .args(["run", &tx_hashes[1].to_string(), "--rpc-url", &endpoint, "--with-local-artifacts"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(revert_output.contains("ReplayTarget::revertingIncrement()"));
    assert!(revert_output.contains("expected revert"));

    let increment_output = cmd
        .cast_fuse()
        .args(["run", &tx_hashes[2].to_string(), "--rpc-url", &endpoint, "--with-local-artifacts"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(increment_output.contains("ReplayTarget::increment()"));
    assert!(increment_output.contains("Transaction successfully executed."));

    let decoded_output = cmd
        .cast_fuse()
        .args([
            "run",
            &tx_hashes[2].to_string(),
            "--rpc-url",
            &endpoint,
            "--with-local-artifacts",
            "--decode-internal",
            "-vvvvv",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(decoded_output.contains("ReplayTarget::_increment()"));
    assert!(decoded_output.contains("@ 0: 1 → 2"));
});

// `cast run --prestate-tracer` uses the prestate tracer when the node exposes the debug API
// (Anvil does), skipping the block replay while still producing correct traces. The block replay
// message must be absent from stderr.
forgetest_async!(cast_run_prestate_tracer, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let tx_hash = deploy_counter_and_set_number(&prj, &mut cmd, &api, &endpoint).await;

    let assert = cmd
        .cast_fuse()
        .args(["run", "--prestate-tracer", format!("{tx_hash}").as_str(), "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]]);
    assert!(
        !assert
            .get_output()
            .stderr_lossy()
            .contains("Executing previous transactions from the block."),
        "prestate tracer path should not replay previous block transactions"
    );
});

// The prestate tracer path produces the same traces as the block replay path, proving the prestate
// is applied correctly before execution.
forgetest_async!(cast_run_prestate_tracer_matches_block_replay, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let tx_hash = deploy_counter_and_set_number(&prj, &mut cmd, &api, &endpoint).await;

    let replay = cmd
        .cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let prestate = cmd
        .cast_fuse()
        .args(["run", "--prestate-tracer", format!("{tx_hash}").as_str(), "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_eq!(replay, prestate, "prestate tracer traces must match block replay traces");
});

// https://github.com/foundry-rs/foundry/issues/12336
// `cast run --debug-trace-transaction` fetches the transaction's trace from the node via
// `debug_traceTransaction` (callTracer) and renders it with the same decoding/rendering machinery
// as the local replay, skipping local execution entirely: the block replay message must be absent
// from stderr.
forgetest_async!(cast_run_debug_trace_transaction, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let tx_hash = deploy_counter_and_set_number(&prj, &mut cmd, &api, &endpoint).await;

    fs::write(
        prj.root().join("foundry.toml"),
        r#"[labels]
        0x0000000000000000000000000000000000000001 = "unused"

        [tracing]
        decode_internal = true
        "#,
    )
    .unwrap();

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    let assert = cmd
        .args([
            "run",
            "--debug-trace-transaction",
            format!("{tx_hash}").as_str(),
            "--rpc-url",
            &endpoint,
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    └─ ← [Return]


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
Warning: Key `[labels]` is being deprecated in favor of `[tracing.labels]`. It will be removed in future versions.

"#]]);
    assert!(
        !assert
            .get_output()
            .stderr_lossy()
            .contains("Executing previous transactions from the block."),
        "debug_traceTransaction path should not replay previous block transactions"
    );
    let receipt = api.transaction_receipt(tx_hash).await.unwrap().unwrap();
    assert!(
        assert.get_output().stdout_lossy().contains(&format!("Gas used: {}", receipt.gas_used())),
        "debug_traceTransaction summary should report the receipt gas used"
    );
});

// `cast run --debug-trace-transaction` must render a multi-node trace: the parent runtime emits a
// LOG0 and then CALLs a child whose runtime reverts (the parent ignores the failure), exercising
// the log/sub-call interleaving, nesting and revert rendering through the real pipeline.
casttest!(cast_run_debug_trace_transaction_renders_nested_call_and_revert, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    // Parent runtime: LOG0(0,0); CALL(gas, 0x..bb, 0,0,0,0,0); POP; STOP.
    api.anvil_set_code(
        address!("0x00000000000000000000000000000000000000aa"),
        "0x60006000a0600060006000600060007300000000000000000000000000000000000000bb5af15000"
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();
    // Child runtime: PUSH1 0 PUSH1 0 REVERT.
    api.anvil_set_code(
        address!("0x00000000000000000000000000000000000000bb"),
        "0x60006000fd".parse().unwrap(),
    )
    .await
    .unwrap();

    cmd.cast_fuse()
        .args([
            "send",
            "0x00000000000000000000000000000000000000aa",
            "run()",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    cmd.cast_fuse()
        .args([
            "run",
            "--debug-trace-transaction",
            format!("{tx_hash}").as_str(),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x00000000000000000000000000000000000000AA::run()
    ├─           data: 0x
    ├─ [..] 0x00000000000000000000000000000000000000bb::fallback()
    │   └─ ← [Revert] execution reverted
    └─ ← [Return]


Transaction successfully executed.
[GAS]

"#]]);
});

// `cast run --debug-trace-transaction --with-local-artifacts` labels the target contract by its
// local artifact name (Counter::) instead of the raw address: the RPC path has no local executor,
// so the bytecode for artifact matching must be fetched over RPC at the transaction's block.
forgetest_async!(cast_run_debug_trace_transaction_with_local_artifacts, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let tx_hash = deploy_counter_and_set_number(&prj, &mut cmd, &api, &endpoint).await;

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    cmd.args([
        "run",
        "--debug-trace-transaction",
        "--la",
        format!("{tx_hash}").as_str(),
        "--rpc-url",
        &endpoint,
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Traces:
  [..] Counter::setNumber(111)
    └─ ← [Return]


Transaction successfully executed.
[GAS]

"#]]);
});

// `--debug-trace-transaction` fetches the trace from the node, so the local-execution-only flags
// must be rejected by clap.
casttest!(cast_run_debug_trace_transaction_conflicts_with_debug, |_prj, cmd| {
    cmd.args([
        "run",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
        "--debug-trace-transaction",
        "--debug",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
error: the argument '--debug-trace-transaction' cannot be used with '--debug'
...
"#]]);
});

// tests cast can decode traces when running with verbosity level > 4
forgetest_async!(show_state_changes_in_traces, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();
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

    // Send tx to change counter storage value.
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "setNumber(uint256)",
            "111",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Assert cast with verbosity displays storage changes.
    cmd.cast_fuse()
        .args([
            "run",
            format!("{tx_hash}").as_str(),
            "-vvvvv",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    ├─  storage changes:
    │   @ 0: 0 → 111
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
...
Executing previous transactions from the block.
...

"#]]);
});

// Tests that `cast trace --raw` reports transaction types Foundry cannot encode instead of
// panicking, e.g. Arbitrum's `ArbitrumInternalTx`.
casttest!(trace_raw_json_unsupported_tx_type, |_prj, cmd| {
    let tx = r#"{"type":"0x6a","chainId":"0xa4b1","nonce":"0x0","gasPrice":"0x0","gas":"0x0","to":"0x00000000000000000000000000000000000a4b05","value":"0x0","input":"0x6bf6a42d","r":"0x0","s":"0x0","v":"0x0","hash":"0xe5ad4cc44e5cd67a464c038af87169fde2bd475f2c00306bd2d55ca2c5e4452e","blockHash":"0x0ce1511da42af573bac6870ef058d63bc4c8552440e97c149d4d539c482b5f7a","blockNumber":"0x1dc83ddc","transactionIndex":"0x0","from":"0x00000000000000000000000000000000000a4b05"}"#;

    cmd.args(["trace", "--raw", tx, "--trace"]).assert_failure().stderr_eq(str![[r#"
Error: Cannot EIP-2718 encode transaction type 0x6a

Context:
- conversion error: Unknown transaction type: 0x6A

"#]]);
});

// tests that displays a sample beacon block traces in Cancun
// https://github.com/foundry-rs/foundry/issues/12435
casttest!(test_beacon_block_root_in_cancun, |prj, cmd| {
    prj.clear();
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "run",
        "0xae290fe8c89c3e83dff20eeb2b8e3261bcdce0d66441c7056918dfb5fafe6d96",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [45054] 0xB731392c0EB5BF2092f9f7B520DA551f70Ea9131::Claim{value: 46698476594582387}()
    ├─ [4320] 0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02::00000000(00000000000000000000000000000000000000000000000069091d4b) [staticcall]
    │   └─ ← [Return] 0x70c7855161ec07af782df915fb3e81702df40f34972da3d740cdfc132ac926f6
    ├─ emit NvStuck(param0: 0x6e6C36B970f8862bA3F148DEdAB8F98f5ed8b426, param1: 46698476594582387 [4.669e16], param2: 1762205003 [1.762e9])
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]]);
});
