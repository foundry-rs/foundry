//! CLI tests for storage commands.

use super::*;

// test that `cast impl` works correctly for both the implementation slot and the beacon slot
casttest!(impl_slot, |_prj, cmd| {
    let eth_rpc_url = next_http_archive_rpc_url();

    // Call `cast impl` for the implementation slot (AAVE Proxy)
    cmd.args([
        "impl",
        "0x4965f6FA20fE9728deCf5165016fc338a5a85aBF",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--block",
        "21422087",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xb61306c8eb34a2104d9eb8d84f1bb1001067fa4b

"#]]);
});

casttest!(impl_slot_beacon, |_prj, cmd| {
    let eth_rpc_url = next_http_archive_rpc_url();

    // Call `cast impl` for the beacon slot
    cmd.args([
        "impl",
        "0xc63d9f0040d35f328274312fc8771a986fc4ba86",
        "--beacon",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--block",
        "21422087",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xa748ae65ba11606492a9c57effa0d4b7be551ec2

"#]]);
});

casttest!(storage, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args(["storage", "vitalik.eth", "1", "--rpc-url", &rpc]).assert_success().stdout_eq(str![
        [r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]
    ]);

    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args(["storage", "vitalik.eth", "0x01", "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_archive_rpc_url();
    let usdt = "0xdac17f958d2ee523a2206206994597c13d831ec7";
    let decimals_slot = "0x09";
    cmd.cast_fuse()
        .args(["storage", usdt, decimals_slot, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000006

"#]]);

    let rpc = next_http_archive_rpc_url();
    let total_supply_slot = "0x01";
    let block_before = "4634747";
    let block_after = "4634748";
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_before])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_after])
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000174876e800

"#]]);

    let decimal_slot_offset_from_total_supply_slot = "0x08";
    let decimal_slot_offset_from_total_supply_slot_uint = "8";
    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args([
            "storage",
            usdt,
            total_supply_slot,
            decimal_slot_offset_from_total_supply_slot,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000006

"#]]);

    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args([
            "storage",
            usdt,
            total_supply_slot,
            decimal_slot_offset_from_total_supply_slot_uint,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000006

"#]]);
});

casttest!(flaky_storage_with_valid_solc_version_1, |_prj, cmd| {
    cmd.args([
        "storage",
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
        "--solc-version",
        "0.8.10",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
    ])
    .assert_success();
});

casttest!(flaky_storage_with_valid_solc_version_2, |_prj, cmd| {
    cmd.args([
        "storage",
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
        "--solc-version",
        "0.8.23",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
    ])
    .assert_success();
});

casttest!(flaky_storage_with_invalid_solc_version_1, |_prj, cmd| {
    let output = cmd
        .args([
            "storage",
            "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
            "--solc-version",
            "0.4.0",
            "--rpc-url",
            next_http_archive_rpc_url().as_str(),
            "--etherscan-api-key",
            next_etherscan_api_key().as_str(),
        ])
        .assert_failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&output);
    assert!(
        stderr.contains(
            "Warning: The provided --solc-version is 0.4.0 while the minimum version for storage layouts is 0.6.5"
        ),
        "stderr did not contain expected warning. Full stderr:\n{stderr}"
    );
});

casttest!(flaky_storage_with_invalid_solc_version_2, |_prj, cmd| {
    cmd.args([
        "storage",
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
        "--solc-version",
        "0.8.2",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: Encountered invalid compiler version in contracts/Create2Deployer.sol: No compiler version exists that matches the version requirement: ^0.8.9

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/6319>
casttest!(flaky_storage_layout_simple, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------╮
| Name    | Type    | Slot | Offset | Bytes | Value | Hex Value                                                          | Contract                                      |
+========================================================================================================================================================================+
| _owner  | address | 0    | 0      | 20    | 0     | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/Create2Deployer.sol:Create2Deployer |
|---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------|
| _paused | bool    | 0    | 20     | 1     | 0     | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/Create2Deployer.sol:Create2Deployer |
╰---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------╯


"#]]);
});

// <https://github.com/foundry-rs/foundry/pull/9332>
casttest!(flaky_storage_layout_simple_json, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
        "--json",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/storage_layout_simple.json": Json]);
});

// <https://github.com/foundry-rs/foundry/issues/6319>
casttest!(flaky_storage_layout_complex, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xBA12222222228d8Ba445958a75a0704d566BF2C8",
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------╮
| Name                          | Type                                                               | Slot | Offset | Bytes | Value                                            | Hex Value                                                          | Contract                        |
+======================================================================================================================================================================================================================================================================================+
| _status                       | uint256                                                            | 0    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _generalPoolsBalances         | mapping(bytes32 => struct EnumerableMap.IERC20ToBytes32Map)        | 1    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _nextNonce                    | mapping(address => uint256)                                        | 2    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _paused                       | bool                                                               | 3    | 0      | 1     | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _authorizer                   | contract IAuthorizer                                               | 3    | 1      | 20    | 549683469959765988649777481110995959958745616871 | 0x0000000000000000000000006048a8c631fb7e77eca533cf9c29784e482391e7 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _approvedRelayers             | mapping(address => mapping(address => bool))                       | 4    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _isPoolRegistered             | mapping(bytes32 => bool)                                           | 5    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _nextPoolNonce                | uint256                                                            | 6    | 0      | 32    | 1760                                             | 0x00000000000000000000000000000000000000000000000000000000000006e0 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _minimalSwapInfoPoolsBalances | mapping(bytes32 => mapping(contract IERC20 => bytes32))            | 7    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _minimalSwapInfoPoolsTokens   | mapping(bytes32 => struct EnumerableSet.AddressSet)                | 8    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _twoTokenPoolTokens           | mapping(bytes32 => struct TwoTokenPoolsBalance.TwoTokenPoolTokens) | 9    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _poolAssetManagers            | mapping(bytes32 => mapping(contract IERC20 => address))            | 10   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _internalTokenBalance         | mapping(address => mapping(contract IERC20 => uint256))            | 11   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
╰-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------╯


"#]]);
});

casttest!(flaky_storage_layout_complex_md, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xBA12222222228d8Ba445958a75a0704d566BF2C8",
        "--md",
    ])
    .assert_success()
    .stdout_eq(str![[r#"

| Name                          | Type                                                               | Slot | Offset | Bytes | Value                                            | Hex Value                                                          | Contract                        |
|-------------------------------|--------------------------------------------------------------------|------|--------|-------|--------------------------------------------------|--------------------------------------------------------------------|---------------------------------|
| _status                       | uint256                                                            | 0    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/vault/Vault.sol:Vault |
| _generalPoolsBalances         | mapping(bytes32 => struct EnumerableMap.IERC20ToBytes32Map)        | 1    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _nextNonce                    | mapping(address => uint256)                                        | 2    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _paused                       | bool                                                               | 3    | 0      | 1     | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _authorizer                   | contract IAuthorizer                                               | 3    | 1      | 20    | 549683469959765988649777481110995959958745616871 | 0x0000000000000000000000006048a8c631fb7e77eca533cf9c29784e482391e7 | contracts/vault/Vault.sol:Vault |
| _approvedRelayers             | mapping(address => mapping(address => bool))                       | 4    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _isPoolRegistered             | mapping(bytes32 => bool)                                           | 5    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _nextPoolNonce                | uint256                                                            | 6    | 0      | 32    | 1760                                             | 0x00000000000000000000000000000000000000000000000000000000000006e0 | contracts/vault/Vault.sol:Vault |
| _minimalSwapInfoPoolsBalances | mapping(bytes32 => mapping(contract IERC20 => bytes32))            | 7    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _minimalSwapInfoPoolsTokens   | mapping(bytes32 => struct EnumerableSet.AddressSet)                | 8    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _twoTokenPoolTokens           | mapping(bytes32 => struct TwoTokenPoolsBalance.TwoTokenPoolTokens) | 9    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _poolAssetManagers            | mapping(bytes32 => mapping(contract IERC20 => address))            | 10   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
| _internalTokenBalance         | mapping(address => mapping(contract IERC20 => uint256))            | 11   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |


"#]]);
});

casttest!(flaky_storage_layout_complex_proxy, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_rpc_endpoint(NamedChain::Sepolia).as_str(),
        "--block",
        "7857852",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xE2588A9CAb7Ea877206E35f615a39f84a64A7A3b",
        "--proxy",
        "0x29fcb43b46531bca003ddc8fcb67ffe91900c762"
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------╮
| Name                       | Type                                            | Slot | Offset | Bytes | Value                                            | Hex Value                                                          | Contract                    |
+============================================================================================================================================================================================================================================================+
| singleton                  | address                                         | 0    | 0      | 20    | 239704109775411986678417050956533140837380441954 | 0x00000000000000000000000029fcb43b46531bca003ddc8fcb67ffe91900c762 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| modules                    | mapping(address => address)                     | 1    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| owners                     | mapping(address => address)                     | 2    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| ownerCount                 | uint256                                         | 3    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| threshold                  | uint256                                         | 4    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| nonce                      | uint256                                         | 5    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| _deprecatedDomainSeparator | bytes32                                         | 6    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| signedMessages             | mapping(bytes32 => uint256)                     | 7    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| approvedHashes             | mapping(address => mapping(bytes32 => uint256)) | 8    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
╰----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------╯


"#]]);
});

casttest!(flaky_storage_layout_complex_json, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xBA12222222228d8Ba445958a75a0704d566BF2C8",
        "--json",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/storage_layout_complex.json": Json]);
});

casttest!(storage_root_empty, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "storage-root",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421\n");
});

casttest!(implementation_empty, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "implementation",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq("0x0000000000000000000000000000000000000000\n");
});

casttest!(admin_empty, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "admin",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq("0x0000000000000000000000000000000000000000\n");
});

casttest!(index_mapping, |_prj, cmd| {
    cmd.args(["index", "uint256", "42", "6"])
        .assert_success()
        .stdout_eq("0xfc808b0f31a1e6b9cf25ff6289feae9b51017b392cc8e25620a94a38dcdafcc1\n");
    cmd.cast_fuse()
        .args(["index", "string", "hello", "1"])
        .assert_success()
        .stdout_eq("0x8404bb4d805e9ca2bd5dd5c43a107e935c8ec393caa7851b353b3192cd5379ae\n");
    cmd.cast_fuse()
        .args(["index", "address", "0xD0074F4E6490ae3f888d1d4f7E3E43326bD3f0f5", "2"])
        .assert_success()
        .stdout_eq("0x9525a448a9000053a4d151336329d6563b7e80b24f8e628e95527f218e8ab5fb\n");
});
