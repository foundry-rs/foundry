//! Tests for the `cast send` command.

use anvil::{EthereumHardfork, NodeConfig};
use foundry_test_utils::{casttest, str};

// ensure receipt or code is required
casttest!(send_requires_to, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "send",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--rpc-url",
        &endpoint,
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Must specify a recipient address or contract code to deploy

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

    // Send a tx with a gas price of 1200000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "1200000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x4e210ed66dcf63734e7db65c6e250e6cecc7f506d937a194d6973f5a58c0a2d6

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
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: server returned an error response: error code -32003: replacement transaction underpriced

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
- Old gas price: 1000000000 wei
- New gas price: 1100000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 2/3).
- Old gas price: 1100000000 wei
- New gas price: 1200000000 wei
Error: transaction already imported.

Retrying with a 10% gas price increase (attempt 3/3).
- Old gas price: 1200000000 wei
- New gas price: 1300000000 wei
0x8da0c415e090f780cff122e9aaa2655dc532daf828da1b617e4841198a74b85b

"#]]);
});

casttest!(send_bump_gas_price_json, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 1200000000 wei.
    cmd.args([
        "send",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
        "--value",
        "0.001ether",
        "--gas-price",
        "1200000000",
        "--async",
        "0x0000000000000000000000000000000000000000",
        "--json",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x4e210ed66dcf63734e7db65c6e250e6cecc7f506d937a194d6973f5a58c0a2d6

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
            "--json",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x8da0c415e090f780cff122e9aaa2655dc532daf828da1b617e4841198a74b85b

"#]]);
});

casttest!(send_bump_gas_price_no_pending_tx, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Try to replace a stuck transaction that doesn't exist.
    cmd.args([
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
    .assert_failure()
    .stderr_eq(str![[r#"
Error: No pending transactions to replace.

"#]]);
});

casttest!(send_bump_gas_price_max_attempts, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 2000000000 wei.
    cmd.args([
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
    .assert_success()
    .stdout_eq(str![[r#"
0xfe1c1e10784315245b7a409fee421a72e07740f7662d0cde2d3bdb79eca5666f

"#]]);

    // Try to replace the stuck transaction by specifying the `--bump-fee` flag.
    // It will attempt to bump the gas price by 10%, with a maximum of 3 bumps.
    // Since the base fee is 1000000000 wei, it will only manage to bump the price to 1300000000 wei
    // which is not enough to replace the stuck transaction. Thus, it will reach the max bump retry
    // limit.
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
- Old gas price: 1000000000 wei
- New gas price: 1100000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 2/3).
- Old gas price: 1100000000 wei
- New gas price: 1200000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 3/3).
- Old gas price: 1200000000 wei
- New gas price: 1300000000 wei
Error: transaction underpriced.

"#]])
        .stderr_eq(str![[r#"
Error: Max gas price bump attempts reached. Transaction still stuck.

"#]]);
});

casttest!(send_bump_gas_price_limit, async |_prj, cmd| {
    // Create a dummy anvil node that won't mine transaction.
    // The goal is to simulate stuck transactions in the pool.
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_no_mining(true)).await;
    let endpoint = handle.http_endpoint();

    // Send a tx with a gas price of 2000000000 wei.
    cmd.args([
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
    .assert_success()
    .stdout_eq(str![[r#"
0xfe1c1e10784315245b7a409fee421a72e07740f7662d0cde2d3bdb79eca5666f

"#]]);

    // Try to replace the stuck transaction by specifying the `--bump-fee` flag.
    // It will attempt to bump the gas price by 10%, with a maximum of 3 bumps.
    // Since the bump gas limit is quite low, it should hit the limit.
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
            "--gas-price-bump-limit",
            "1200000000",
            "--async",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 1/3).
- Old gas price: 1000000000 wei
- New gas price: 1100000000 wei
Error: transaction underpriced.

Retrying with a 10% gas price increase (attempt 2/3).
- Old gas price: 1100000000 wei
- New gas price: 1200000000 wei

"#]])
        .stderr_eq(str![[r#"
Error: Unable to bump more the gas price. Hit the bump gas limit of 1200000000 wei.

"#]]);
});
