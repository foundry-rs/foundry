use super::*;
use alloy_primitives::Bloom;

#[forgetest_init]
fn deployment_lookup_by_label(prj: _, cmd: _) {
    prj.insert_vm();
    prj.insert_ds_test();

    let deployments = [
        ("Counter", Some("UsdcPool"), "CREATE"),
        ("Counter", Some("WethPool"), "CREATE2"),
        ("Counter", Some("Counter"), "CREATE"),
        ("Counter", Some("Token"), "CREATE"),
        ("Token", None, "CREATE"),
    ];
    let mut transactions = Vec::new();
    let mut receipts = Vec::new();
    for (index, (name, label, kind)) in deployments.into_iter().enumerate() {
        let address = Address::with_last_byte(index as u8 + 1);
        let hash = B256::with_last_byte(index as u8 + 1);
        let mut transaction = serde_json::json!({
            "hash": hash,
            "transactionType": kind,
            "contractName": name,
            "contractAddress": address,
            "function": null,
            "arguments": null,
            "transaction": {"type": "0x0", "from": Address::ZERO},
        });
        if let Some(label) = label {
            transaction["label"] = label.into();
        }
        transactions.push(transaction);
        receipts.push(serde_json::json!({
            "type": "0x0",
            "status": "0x1",
            "cumulativeGasUsed": "0x5208",
            "logs": [],
            "logsBloom": Bloom::ZERO,
            "transactionHash": hash,
            "transactionIndex": "0x0",
            "blockHash": B256::ZERO,
            "blockNumber": format!("{:#x}", index + 1),
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x1",
            "from": Address::ZERO,
            "to": null,
            "contractAddress": address,
        }));
    }
    let dir = prj.root().join("broadcast/Counter.s.sol/1");
    std::fs::create_dir_all(&dir).unwrap();
    foundry_common::fs::write_json_file(
        &dir.join("run-1.json"),
        &serde_json::json!({
            "transactions": transactions,
            "receipts": receipts,
            "libraries": [],
            "pending": [],
            "returns": {},
            "timestamp": 1,
            "chain": 1,
        }),
    )
    .unwrap();

    prj.add_test(
        "DeploymentLookup",
        r#"
import {Vm} from "../src/Vm.sol";
import {DSTest} from "../src/test.sol";

contract DeploymentLookupTest is DSTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_labels() public {
        vm.chainId(1);
        assertEq(vm.getDeployment("UsdcPool"), address(1));
        assertEq(vm.getDeployment("WethPool", 1), address(2));
    }

    function test_contract_names() public {
        vm.chainId(1);
        assertEq(vm.getDeployment("Counter"), address(4));
        assertEq(vm.getDeployments("Counter", 1).length, 4);
        assertEq(vm.getDeployments("UsdcPool", 1).length, 0);
    }

    function test_label_naming_another_contract() public {
        vm._expectCheatcodeRevert(
            "Token is the label of 0x0000000000000000000000000000000000000004 and the contract name of 0x0000000000000000000000000000000000000005 on chain 1"
        );
        vm.getDeployment("Token", 1);
    }
}
"#,
    );

    cmd.args(["test", "--mc", "DeploymentLookupTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 3 tests for test/DeploymentLookup.sol:DeploymentLookupTest
[PASS] test_contract_names() ([GAS])
[PASS] test_label_naming_another_contract() ([GAS])
[PASS] test_labels() ([GAS])
Suite result: ok. 3 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
