//! CLI tests for source commands.

use super::*;
use axum::{Json, Router, routing::get};

// tests that `cast interface` excludes the constructor
// <https://github.com/alloy-rs/core/issues/555>
casttest!(interface_no_constructor, |prj, cmd| {
    let interface = include_str!("../fixtures/interface.json");

    let path = prj.root().join("interface.json");
    fs::write(&path, interface).unwrap();
    // Call `cast find-block`
    cmd.arg("interface").arg(&path).assert_success().stdout_eq(str![[
        r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

library IIntegrationManager {
    type SpendAssetsHandleType is uint8;
}

interface Interface {
    function getIntegrationManager() external view returns (address integrationManager_);
    function lend(address _vaultProxy, bytes memory, bytes memory _assetData) external;
    function parseAssetsForAction(address, bytes4 _selector, bytes memory _actionData)
        external
        view
        returns (
            IIntegrationManager.SpendAssetsHandleType spendAssetsHandleType_,
            address[] memory spendAssets_,
            uint256[] memory spendAssetAmounts_,
            address[] memory incomingAssets_,
            uint256[] memory minIncomingAssetAmounts_
        );
    function redeem(address _vaultProxy, bytes memory, bytes memory _assetData) external;
}

"#
    ]]);
});

// tests that `cast interface --flatten` inlines inherited struct types into the interface
// <https://github.com/foundry-rs/foundry/issues/9960>
casttest!(interface_flatten, |prj, cmd| {
    let interface = include_str!("../fixtures/interface_inherited_struct.json");

    let path = prj.root().join("interface_inherited_struct.json");
    fs::write(&path, interface).unwrap();

    // Without --flatten, a separate library is generated for the struct
    cmd.arg("interface").arg(&path).assert_success().stdout_eq(str![[
        r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

library IBase {
    struct TestStruct {
        address asset;
    }
}

interface Interface {
    function test(IBase.TestStruct memory param) external;
}

"#
    ]]);

    // With --flatten, the struct is inlined into the interface
    cmd.cast_fuse().arg("interface").arg("--flatten").arg(&path).assert_success().stdout_eq(str![
        [r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

interface Interface {
    // Types from `IBase`
    struct TestStruct {
        address asset;
    }

    function test(TestStruct memory param) external;
}

"#]
    ]);
});

casttest!(interface_with_function_pointer_in_struct, |prj, cmd| {
    let abi = r#"[
        {
            "anonymous": false,
            "inputs": [
                {
                    "components": [
                        {"internalType": "uint256", "name": "id", "type": "uint256"},
                        {
                            "internalType": "function (uint256) external",
                            "name": "callback",
                            "type": "function"
                        }
                    ],
                    "indexed": false,
                    "internalType": "struct StructWithFunctionEvent.Action",
                    "name": "action",
                    "type": "tuple"
                }
            ],
            "name": "ActionLogged",
            "type": "event"
        }
    ]"#;
    let path = prj.root().join("function_event_abi.json");
    fs::write(&path, abi).unwrap();

    cmd.arg("interface")
        .arg(&path)
        .arg("--name")
        .arg("StructWithFunctionEvent")
        .assert_success()
        .stdout_eq(str![[r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

interface StructWithFunctionEvent {
    struct Action {
        uint256 id;
        function(uint256) external callback;
    }

    event ActionLogged(Action action);
}

"#]]);
});

casttest!(interface_local_contract_does_not_write_artifacts, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "InterfaceTarget",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract InterfaceTarget {
    event ValueSet(uint256 value);

    function setValue(uint256 value) external {
        emit ValueSet(value);
    }
}
    "#,
    );

    let source = prj.root().join("src/InterfaceTarget.sol");
    let artifact = prj.root().join("out/InterfaceTarget.sol/InterfaceTarget.json");
    let output =
        cmd.cast_fuse().arg("interface").arg(&source).assert_success().get_output().stdout_lossy();
    assert!(output.contains("interface InterfaceTarget"), "{output}");
    assert!(output.contains("event ValueSet(uint256 value);"), "{output}");
    assert!(!artifact.exists());

    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, b"sentinel").unwrap();

    cmd.cast_fuse().arg("interface").arg(&source).assert_success();
    let after = fs::read(&artifact).unwrap();
    assert_eq!(after, b"sentinel");
});

// tests that fetches WETH interface from etherscan
// <https://etherscan.io/token/0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2>
casttest!(flaky_fetch_weth_interface_from_etherscan, |_prj, cmd| {
    cmd.args([
        "interface",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

interface WETH9 {
    event Approval(address indexed src, address indexed guy, uint256 wad);
    event Deposit(address indexed dst, uint256 wad);
    event Transfer(address indexed src, address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);

    fallback() external payable;

    function allowance(address, address) external view returns (uint256);
    function approve(address guy, uint256 wad) external returns (bool);
    function balanceOf(address) external view returns (uint256);
    function decimals() external view returns (uint8);
    function deposit() external payable;
    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
    function totalSupply() external view returns (uint256);
    function transfer(address dst, uint256 wad) external returns (bool);
    function transferFrom(address src, address dst, uint256 wad) external returns (bool);
    function withdraw(uint256 wad) external;
}

"#]]);
});

// tests that fetches a sample contract creation code
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(flaky_fetch_creation_code_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "creation-code",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x0923cad07f06b2d0e5e49e63b8b35738d4156b95",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x60566050600b82828239805160001a6073146043577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600080fdfea264697066735822122074c61e8e4eefd410ca92eec26e8112ec6e831d0a4bf35718fdd78b45d68220d064736f6c63430008070033

"#]]);
});

// tests that fetches a sample contract creation args bytes
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(flaky_fetch_creation_code_only_args_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "creation-code",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x6982508145454ce325ddbe47a25d4ec3d2311933",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--only-args",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x00000000000000000000000000000000000014bddab3e51a57cff87a50000000

"#]]);
});

// tests that displays a sample contract creation args
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(flaky_fetch_constructor_args_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "constructor-args",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x6982508145454ce325ddbe47a25d4ec3d2311933",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x00000000000000000000000000000000000014bddab3e51a57cff87a50000000 → Uint(420690000000000000000000000000000, 256)

"#]]);
});

// tests that displays a sample contract artifact
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(flaky_fetch_artifact_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "artifact",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x0923cad07f06b2d0e5e49e63b8b35738d4156b95",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"{
  "abi": [],
  "bytecode": {
    "object": "0x60566050600b82828239805160001a6073146043577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600080fdfea264697066735822122074c61e8e4eefd410ca92eec26e8112ec6e831d0a4bf35718fdd78b45d68220d064736f6c63430008070033"
  }
}

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/4776>
casttest!(flaky_fetch_src_blockscout, |_prj, cmd| {
    let url = "https://eth.blockscout.com/api";

    let weth = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

    cmd.args([
        "source",
        &weth.to_string(),
        "--chain-id",
        "1",
        "--explorer-api-url",
        url,
        "--flatten",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
contract WETH9 {
    string public name     = "Wrapped Ether";
    string public symbol   = "WETH";
    uint8  public decimals = 18;
..."#]]);
});

casttest!(flaky_fetch_src_default, |_prj, cmd| {
    let weth = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let etherscan_api_key = next_etherscan_api_key();

    cmd.args(["source", &weth.to_string(), "--flatten", "--etherscan-api-key", &etherscan_api_key])
        .assert_success()
        .stdout_eq(str![[r#"
...
contract WETH9 {
    string public name     = "Wrapped Ether";
    string public symbol   = "WETH";
    uint8  public decimals = 18;
..."#]]);
});

casttest!(source_plain_and_directory, async |prj, cmd| {
    let source = "pragma solidity ^0.8.0; contract Example {}";
    let response = json!({
        "status": "1", "message": "OK", "result": [{
            "SourceCode": source, "ABI": "[]", "ContractName": "Example",
            "CompilerVersion": "v0.8.30+commit.73712a01", "OptimizationUsed": "0",
            "Runs": "200", "EVMVersion": "Default", "Proxy": "0"
        }]
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/api",
        get(move || {
            let response = response.clone();
            async move { Json(response) }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let args = [
        "source",
        "0x0000000000000000000000000000000000000001",
        "--explorer-api-url",
        &format!("{url}/api"),
        "--explorer-url",
        &url,
    ];
    cmd.args(args).assert_success().stdout_eq(format!("{source}\n"));
    let directory = prj.root().join("sources");
    cmd.cast_fuse().args(args).arg("-d").arg(&directory).assert_empty_stdout();
    assert_eq!(fs::read_to_string(directory.join("Example/Contract.sol")).unwrap(), source);
    server.abort();
});
