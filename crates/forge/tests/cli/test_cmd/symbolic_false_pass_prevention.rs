use super::symbolic_helpers::assert_relevant_lines;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, util::OutputExt};

forgetest_init!(symbolic_false_pass_prevention, |prj, cmd| {
    skip_unless_z3!("symbolic_false_pass_prevention");

    prj.add_test(
        "SymbolicFalsePassPrevention.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicFalsePassPrevention is Test {
    function checkConcreteAddmodMulmodUseUnboundedIntermediate() public pure {
        uint256 max = type(uint256).max;
        uint256 addResult;
        uint256 mulResult;

        assembly {
            addResult := addmod(max, 1, max)
            mulResult := mulmod(shl(255, 1), 2, max)
        }

        assert(addResult == 1);
        assert(mulResult == 1);
    }

    function checkSymbolicAddmodFailsClosed(uint256 a) public {
        vm.assume(a == type(uint256).max);

        uint256 result;
        assembly {
            result := addmod(a, 1, not(0))
        }

        assert(result == 0);
    }

    function checkSymbolicMulmodFailsClosed(uint256 a) public {
        vm.assume(a == (uint256(1) << 255));

        uint256 result;
        assembly {
            result := mulmod(a, 2, not(0))
        }

        assert(result == 0);
    }

    function checkKzgPrecompileInvalidWitnessCounterexample(bytes32 z) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            z,
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        (bool ok,) = address(0x0a).staticcall(input);
        assert(ok);
    }

    function checkBn254AddPrecompileFailsClosed(uint256 x) public view {
        (bool ok,) = address(0x06).staticcall(abi.encode(x, uint256(0), uint256(0), uint256(0)));
        assert(ok);
    }

    function checkBn254MulPrecompileFailsClosed(uint256 x) public view {
        (bool ok,) = address(0x07).staticcall(abi.encode(x, uint256(0), uint256(0)));
        assert(ok);
    }

    function checkBn254PairingPrecompileFailsClosed(uint256 x) public view {
        bytes memory input = new bytes(192);
        assembly {
            mstore(add(input, 32), x)
        }
        (bool ok,) = address(0x08).staticcall(input);
        assert(ok);
    }

    function checkBlake2fFinalFlagFailsClosed(uint8 flag) public {
        vm.assume(flag > 1);
        bytes memory input = new bytes(213);
        input[212] = bytes1(flag);

        (bool ok,) = address(0x09).staticcall(input);
        assert(ok);
    }

    function checkSetEvmVersionFailsClosed() public {
        vm.setEvmVersion("london");
    }

    function checkGetEvmVersionFailsClosed() public view {
        assertEq(vm.getEvmVersion(), "cancun");
    }

    function checkExpectSafeMemoryFailsClosed() public {
        vm.expectSafeMemory(0x80, 0xA0);
    }

    function checkExpectSafeMemoryCallFailsClosed() public {
        vm.expectSafeMemoryCall(0x80, 0xA0);
    }

    function checkStopExpectSafeMemoryFailsClosed() public {
        vm.stopExpectSafeMemory();
    }

    function checkLastCallGasFailsClosed() public view {
        Vm.Gas memory gas = vm.lastCallGas();
        assert(gas.gasTotalUsed == 0);
    }

    function checkLastFrameGasFailsClosed() public view {
        (bool success, bytes memory data) = address(vm).staticcall(abi.encodeWithSignature("lastFrameGas()"));
        assert(success && data.length != 0);
    }

    function checkSnapshotGasLastCallFailsClosed() public {
        uint256 gasUsed = vm.snapshotGasLastCall("name");
        assert(gasUsed == 0);
    }

    function checkSnapshotGasLastFrameFailsClosed() public {
        (bool success, bytes memory data) =
            address(vm).call(abi.encodeWithSignature("snapshotGasLastFrame(string)", "name"));
        assert(success && data.length != 0);
    }

    function checkStopSnapshotGasFailsClosed() public {
        uint256 gasUsed = vm.stopSnapshotGas();
        assert(gasUsed == 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicFalsePassPrevention"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConcreteAddmodMulmodUseUnboundedIntermediate()
[FAIL: panic: assertion failed (0x01); counterexample:
checkSymbolicAddmodFailsClosed(uint256)
checkSymbolicMulmodFailsClosed(uint256)
checkKzgPrecompileInvalidWitnessCounterexample(bytes32)
unsupported symbolic execution feature: symbolic bn254 precompile validity not modeled
unsupported symbolic execution feature: symbolic blake2f precompile final flag not modeled
unsupported symbolic execution feature: symbolic vm.setEvmVersion not modeled
unsupported symbolic execution feature: symbolic vm.getEvmVersion not modeled
unsupported symbolic execution feature: symbolic vm.expectSafeMemory not modeled
unsupported symbolic execution feature: symbolic vm.expectSafeMemoryCall not modeled
unsupported symbolic execution feature: symbolic vm.stopExpectSafeMemory not modeled
unsupported symbolic execution feature: symbolic vm.lastCallGas not modeled
unsupported symbolic execution feature: symbolic vm.lastFrameGas not modeled
unsupported symbolic execution feature: symbolic vm.snapshotGasLastCall not modeled
unsupported symbolic execution feature: symbolic vm.snapshotGasLastFrame not modeled
unsupported symbolic execution feature: symbolic vm.stopSnapshotGas not modeled
"#]],
    );
    assert!(!stdout.contains("symbolic KZG point-evaluation precompile"), "{stdout}");
});
