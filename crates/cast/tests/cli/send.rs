//! Tests for the `cast send` command.

use anvil::{EthereumHardfork, NodeConfig};
use foundry_test_utils::{casttest, str};

// ensure receipt or code is required
casttest!(send_requires_to, |_prj, cmd| {
    cmd.args([
        "send",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
Must specify a recipient address or contract code to deploy

"#]]);
});

casttest!(send_eip7702, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::PragueEOF.into())))
            .await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "send",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success();

    cmd.cast_fuse()
        .args(["code", "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
0xef010070997970c51812dc3a010c7d01b50e0d17dc79c8

"#]]);
});

casttest!(send_bump_gas_price, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 2500000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "2500000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x940a9649489d3d261581d173a16e78db0d99a329aef97f7cf09e633ca14b94f2

"#]]);

    // Now try to replace the stuck transaction.
    // This will not work since the gas price specified is lower than the original gas price.
    cmd.cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--value",
            "0.001ether",
            "--gas-price",
            "2000000000",
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: 
server returned an error response: error code -32003: replacement transaction underpriced

"#]]);

    // Replace the stuck transaction by specifying the `--bump-fee` flag.
    cmd.cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--value",
            "0.001ether",
            "--bump-fee",
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 1/3).
- Old gas price: 2000000000 wei
- New gas price: 2200000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 2/3).
- Old gas price: 2200000000 wei
- New gas price: 2400000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 3/3).
- Old gas price: 2400000000 wei
- New gas price: 2600000000 wei
0x462e482cb3585783a6ec333b5afce38af4169fc17452e2f89633604b0fc80ac8

"#]]);
});

casttest!(send_bump_gas_price_json, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 2500000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "2500000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x940a9649489d3d261581d173a16e78db0d99a329aef97f7cf09e633ca14b94f2

"#]]);

    // Replace the stuck transaction by specifying the `--bump-fee` flag.
    // Format the output using `--json`.
    cmd.cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--value",
            "0.001ether",
            "--bump-fee",
            "--async",
            "--json",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x462e482cb3585783a6ec333b5afce38af4169fc17452e2f89633604b0fc80ac8

"#]]);
});

casttest!(send_bump_gas_price_max_attempts, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 3000000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "3000000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xadea2efb4253217232d1b4c780d700d00ec0e2d417091cf3359b3a5981a09db2

"#]]);

    // Try to replace the stuck transaction by specifying the `--bump-fee` flag.
    // Since it will incrementally bump the gas price by 10% with a maximum of 3 bumps, it won't
    // be able to replace the stuck transaction, and it should reach the max bump retry limit.
    cmd.cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--value",
            "0.001ether",
            "--bump-fee",
            "--gas-price-increment-percentage",
            "10",
            "--max-gas-price-bumps",
            "3",
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 1/3).
- Old gas price: 2000000000 wei
- New gas price: 2200000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 2/3).
- Old gas price: 2200000000 wei
- New gas price: 2400000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 3/3).
- Old gas price: 2400000000 wei
- New gas price: 2600000000 wei
Error: transaction underpriced.

"#]])
        .stderr_eq(str![[r#"
Error: 
Max gas price bump attempts reached. Transaction still stuck.

"#]]);
});

casttest!(send_bump_gas_price_limit, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 2200000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "2200000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x46e6334ef6ebc0242d2397192805ae9973f85091f898a915e762be7462b0f18b

"#]]);

    // Try to replace the stuck transaction by specifying the `--bump-fee` flag.
    // The gas price bump limit is set to 2400000000 wei. Since the gas is bumped by 10% each time,
    // it should hit the gas price bump limit on the second retry (it starts at the initial base
    // fee which should be around 2000000000 wei).
    cmd.cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--value",
            "0.001ether",
            "--bump-fee",
            "--gas-price-increment-percentage",
            "10",
            "--max-gas-price-bumps",
            "3",
            "--gas-price-bump-limit",
            "2400000000",
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 1/3).
- Old gas price: 2000000000 wei
- New gas price: 2200000000 wei
Error: transaction already imported.

"#]])
        .stderr_eq(str![[r#"
Error: 
Unable to bump more the gas price. Hit the bump gas limit of 2400000000 wei.

"#]]);
});
