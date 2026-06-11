use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;

forgetest_init!(symbolic_precompiles_execute_concrete_inputs, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_precompiles_execute_concrete_inputs because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompiles.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPrecompiles is Test {
    function checkHashPrecompiles(uint256) public {
        assertEq(
            sha256(bytes("abc")),
            0xba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        );
        assertEq(ripemd160(bytes("abc")), bytes20(hex"8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"));
    }

    function checkIdentityPrecompile(uint256) public {
        (bool ok, bytes memory out) = address(4).staticcall(hex"0102030405");
        assert(ok);
        assertEq(out, hex"0102030405");
    }

    function checkModexpPrecompile(uint256) public {
        bytes memory input = abi.encodePacked(uint256(1), uint256(1), uint256(1), bytes1(0x02), bytes1(0x05), bytes1(0x0d));
        (bool ok, bytes memory out) = address(5).staticcall(input);
        assert(ok);
        assertEq(out, hex"06");
    }

    function checkBn254Precompiles(uint256) public {
        bytes memory emptyPointPair = new bytes(128);
        (bool okAdd, bytes memory addOut) = address(6).staticcall(emptyPointPair);
        assert(okAdd);
        assertEq(addOut.length, 64);
        assertEq(keccak256(addOut), keccak256(new bytes(64)));

        bytes memory emptyMul = new bytes(96);
        (bool okMul, bytes memory mulOut) = address(7).staticcall(emptyMul);
        assert(okMul);
        assertEq(mulOut.length, 64);
        assertEq(keccak256(mulOut), keccak256(new bytes(64)));

        (bool okPair, bytes memory pairOut) = address(8).staticcall("");
        assert(okPair);
        assertEq(pairOut, abi.encode(uint256(1)));
    }

    function checkBlake2fPrecompile(uint256) public {
        bytes memory input = new bytes(213);
        (bool ok, bytes memory out) = address(9).staticcall(input);
        assert(ok);
        assertEq(out.length, 64);
        assertEq(keccak256(out), 0x209b39a06473bccc5b67b4f26619611f672409e4480527289d64c51b9a382bb6);

        (bool invalidOk, bytes memory invalidOut) = address(9).staticcall(hex"00");
        assert(!invalidOk);
        assertEq(invalidOut.length, 0);
    }

    function checkEcrecoverPrecompile(uint256) public {
        bytes32 digest = keccak256("foundry-symbolic-precompile");
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, digest);
        assertEq(ecrecover(digest, v, r, s), vm.addr(1));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompiles"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkHashPrecompiles(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkIdentityPrecompile(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkModexpPrecompile(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBn254Precompiles(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBlake2fPrecompile(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkEcrecoverPrecompile(uint256)
"#]],
    );
});

forgetest_init!(symbolic_hash_precompiles_accept_symbolic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_hash_precompiles_accept_symbolic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompileInput.t.sol",
        r#"
contract SymbolicPrecompileInput {
    /// forge-config: default.symbolic.array_lengths = [2]
    function checkSymbolicHashDeterminism(bytes memory data) public {
        bytes32 shaA = sha256(data);
        bytes32 shaB = sha256(data);
        assert(shaA == shaB);

        bytes20 ripemdA = ripemd160(data);
        bytes20 ripemdB = ripemd160(data);
        assert(ripemdA == ripemdB);
    }

    function checkSymbolicEcrecoverDeterminism(
        bytes32 digest,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address first = ecrecover(digest, v, r, s);
        address second = ecrecover(digest, v, r, s);

        assert(first == second);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompileInput"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicHashDeterminism(bytes)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicEcrecoverDeterminism(bytes32,uint8,bytes32,bytes32)
"#]],
    );
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
});

forgetest_init!(symbolic_identity_precompile_accepts_symbolic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_identity_precompile_accepts_symbolic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicIdentityPrecompileInput.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicIdentityPrecompileInput is Test {
    /// forge-config: default.symbolic.array_lengths = [2]
    function checkSymbolicIdentity(bytes memory data) public {
        (bool ok, bytes memory out) = address(4).staticcall(data);
        assert(ok);
        assert(out.length == data.length);
        for (uint256 i; i < data.length; ++i) {
            assert(out[i] == data[i]);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicIdentity"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicIdentity(bytes)
"#]],
    );
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
});

forgetest_init!(symbolic_advanced_precompiles_accept_symbolic_payloads, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_advanced_precompiles_accept_symbolic_payloads because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAdvancedPrecompileInput.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAdvancedPrecompileInput is Test {
    function checkSymbolicModexp(bytes1 base) public {
        bytes memory input = abi.encodePacked(
            uint256(1),
            uint256(1),
            uint256(1),
            base,
            bytes1(0x05),
            bytes1(0x0d)
        );
        (bool okA, bytes memory outA) = address(5).staticcall(input);
        (bool okB, bytes memory outB) = address(5).staticcall(input);

        assert(okA);
        assert(okB);
        assertEq(outA.length, 1);
        assertEq(outA.length, outB.length);
        assertEq(keccak256(outA), keccak256(outB));
    }

    function checkSymbolicBlake2f(bytes1 tweak) public {
        bytes memory input = new bytes(213);
        input[10] = tweak;

        (bool okA, bytes memory outA) = address(9).staticcall(input);
        (bool okB, bytes memory outB) = address(9).staticcall(input);

        assert(okA);
        assert(okB);
        assertEq(outA.length, 64);
        assertEq(outA.length, outB.length);
        assertEq(keccak256(outA), keccak256(outB));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicAdvancedPrecompileInput"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicModexp(bytes1)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicBlake2f(bytes1)
"#]],
    );
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
    assert!(!stdout.contains("symbolic precompile length header"), "{stdout}");
});

forgetest_init!(symbolic_precompiles_accept_symbolic_input_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_precompiles_accept_symbolic_input_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompileInputSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPrecompileInputSize is Test {
    function checkSymbolicIdentityInputSize(uint256 size) public {
        vm.assume(size <= 4);

        bool ok;
        uint256 rdsize;
        bytes32 word;
        assembly {
            let inPtr := mload(0x40)
            let outPtr := add(inPtr, 0x40)
            mstore(inPtr, 0x0102030400000000000000000000000000000000000000000000000000000000)
            mstore(outPtr, 0)
            ok := staticcall(gas(), 4, inPtr, size, outPtr, 4)
            rdsize := returndatasize()
            word := mload(outPtr)
        }

        bytes32 expected;
        if (size > 0) expected |= bytes32(uint256(0x01) << 248);
        if (size > 1) expected |= bytes32(uint256(0x02) << 240);
        if (size > 2) expected |= bytes32(uint256(0x03) << 232);
        if (size > 3) expected |= bytes32(uint256(0x04) << 224);

        assert(ok);
        assertEq(rdsize, size);
        assertEq(word, expected);
    }

    function checkSymbolicShaInputSize(uint256 size) public {
        vm.assume(size <= 4);

        bool okA;
        bool okB;
        bytes32 outA;
        bytes32 outB;
        assembly {
            let inPtr := mload(0x40)
            let outPtrA := add(inPtr, 0x40)
            let outPtrB := add(inPtr, 0x80)
            mstore(inPtr, 0x0102030400000000000000000000000000000000000000000000000000000000)
            okA := staticcall(gas(), 2, inPtr, size, outPtrA, 32)
            okB := staticcall(gas(), 2, inPtr, size, outPtrB, 32)
            outA := mload(outPtrA)
            outB := mload(outPtrB)
        }

        assert(okA);
        assert(okB);
        assertEq(outA, outB);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompileInputSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicIdentityInputSize(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicShaInputSize(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic precompile CALL input size"), "{stdout}");
});
