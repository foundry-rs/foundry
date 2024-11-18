// tests complete bind-json workflow
// ensures that we can run forge-bind even if files are depending on yet non-existent bindings and
// that generated bindings are correct
forgetest_init!(test_bind_json, |prj, cmd| {
    prj.add_test(
        "JsonBindings",
        r#"
import {JsonBindings} from "utils/JsonBindings.sol";
import {Test} from "forge-std/Test.sol";

struct TopLevelStruct {
    uint256 param1;
    int8 param2;
}

contract BindJsonTest is Test {
    using JsonBindings for *;

    struct ContractLevelStruct {
        address[][] param1;
        address addrParam;
    }

    function testTopLevel() public {
        string memory json = '{"param1": 1, "param2": -1}';
        TopLevelStruct memory topLevel = json.deserializeTopLevelStruct();
        assertEq(topLevel.param1, 1);
        assertEq(topLevel.param2, -1);

        json = topLevel.serialize();
        TopLevelStruct memory deserialized = json.deserializeTopLevelStruct();
        assertEq(keccak256(abi.encode(deserialized)), keccak256(abi.encode(topLevel)));
    }

    function testContractLevel() public {
        ContractLevelStruct memory contractLevel = ContractLevelStruct({
            param1: new address[][](2),
            addrParam: address(0xBEEF)
        });

        string memory json = contractLevel.serialize();
        assertEq(json, '{"param1":[[],[]],"addrParam":"0x000000000000000000000000000000000000bEEF"}');

        ContractLevelStruct memory deserialized = json.deserializeContractLevelStruct();
        assertEq(keccak256(abi.encode(deserialized)), keccak256(abi.encode(contractLevel)));
    }
}
"#,
    )
    .unwrap();

    cmd.arg("bind-json").assert_success();
    cmd.forge_fuse().args(["test"]).assert_success();
});
