use super::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// KEVM/Kontrol-style opcode semantic smoke test for CREATE2 + RETURNDATACOPY.
forgetest_init!(kevm_create2_returndatacopy_semantics, |prj, cmd| {
    skip_unless_z3!("kevm_create2_returndatacopy_semantics");

    prj.add_test(
        "KevmOpcodeSemantics.t.sol",
        r#"
contract Created {
    function ping() external pure returns (uint256) {
        return 0x42;
    }
}

contract KevmOpcodeSemantics {
    function checkCreate2AndReturndata(bytes32 salt) public {
        address deployed;
        bytes memory initcode = type(Created).creationCode;
        assembly {
            deployed := create2(0, add(initcode, 0x20), mload(initcode), salt)
        }
        require(deployed != address(0));

        (bool ok, bytes memory data) = deployed.staticcall(abi.encodeWithSignature("ping()"));
        require(ok);
        assert(abi.decode(data, (uint256)) == 0x42);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkCreate2AndReturndata"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/KevmOpcodeSemantics.t.sol:KevmOpcodeSemantics
[PASS] checkCreate2AndReturndata(bytes32) ([METRICS])
...
"#]]);
});
