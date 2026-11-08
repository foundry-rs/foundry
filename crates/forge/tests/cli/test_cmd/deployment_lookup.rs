use super::*;
use alloy_primitives::Bloom;

#[forgetest_init]
fn deployment_lookup_by_label(prj: _, cmd: _) {
    prj.insert_vm();
    prj.insert_ds_test();

    let deployments = [
        ("Counter", Some("UsdcPool"), "CREATE"),
        ("Counter", Some("WethPool"), "CREATE2"),
        ("Legacy", None, "CREATE"),
        ("Duplicate", Some("Pool"), "CREATE"),
        ("Duplicate", Some("Pool"), "CREATE"),
        ("Counter", Some("Collision"), "CREATE"),
        ("Collision", None, "CREATE"),
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
    let dir = prj.root().join("broadcast/Counter.s.sol/31337");
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
            "chain": 31337,
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
        vm.chainId(31337);
        assertEq(vm.getDeployment("UsdcPool"), address(1));
        assertEq(vm.getDeployment("UsdcPool", 31337), address(1));
        assertEq(vm.getDeployment("WethPool"), address(2));
        assertEq(vm.getDeployment("WethPool", 31337), address(2));
    }

    function test_contract_names() public {
        assertEq(vm.getDeployment("Counter", 31337), address(6));
        assertEq(vm.getDeployment("Legacy", 31337), address(3));
        assertEq(vm.getDeployments("Counter", 31337).length, 3);
        assertEq(vm.getDeployments("UsdcPool", 31337).length, 0);
    }

    function test_duplicate_labels() public {
        vm.chainId(31337);
        vm._expectCheatcodeRevert("ambiguous deployment for Pool on chain 31337");
        vm.getDeployment("Pool");
        vm._expectCheatcodeRevert("ambiguous deployment for Pool on chain 31337");
        vm.getDeployment("Pool", 31337);
    }

    function test_name_collision() public {
        vm.chainId(31337);
        vm._expectCheatcodeRevert("ambiguous deployment for Collision on chain 31337");
        vm.getDeployment("Collision");
        vm._expectCheatcodeRevert("ambiguous deployment for Collision on chain 31337");
        vm.getDeployment("Collision", 31337);
    }
}
"#,
    );

    cmd.args(["test", "--mc", "DeploymentLookupTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 4 tests for test/DeploymentLookup.sol:DeploymentLookupTest
[PASS] test_contract_names() ([GAS])
[PASS] test_duplicate_labels() ([GAS])
[PASS] test_labels() ([GAS])
[PASS] test_name_collision() ([GAS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
