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

    function checkKzgPointEvaluationPrecompile(uint256) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            hex"73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        (bool ok, bytes memory out) = address(0x0a).staticcall(input);
        assert(ok);
        assertEq(
            out,
            hex"000000000000000000000000000000000000000000000000000000000000100073eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001"
        );

        bytes memory shortInput = new bytes(191);
        (bool shortOk, bytes memory shortOut) = address(0x0a).staticcall(shortInput);
        assert(!shortOk);
        assertEq(shortOut.length, 0);

        input[0] = 0x02;
        (bool mismatchOk, bytes memory mismatchOut) = address(0x0a).staticcall(input);
        assert(!mismatchOk);
        assertEq(mismatchOut.length, 0);
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
[PASS] checkKzgPointEvaluationPrecompile(uint256)
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

forgetest_init!(symbolic_kzg_precompile_models_symbolic_witnesses, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_kzg_precompile_models_symbolic_witnesses because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicKzgPrecompileInput.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicKzgPrecompileInput is Test {
    function checkSymbolicKzgInvalidWitnessReturnsCounterexample(bytes32 z) public {
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

    function checkSymbolicKzgSuccessWitnessReturnsCounterexample(
        bytes32 versionedHash,
        bytes32 z,
        bytes32 y,
        bytes32 commitmentPrefix,
        bytes16 commitmentSuffix,
        bytes32 proofPrefix,
        bytes16 proofSuffix
    ) public {
        bytes memory input = abi.encodePacked(
            versionedHash,
            z,
            y,
            commitmentPrefix,
            commitmentSuffix,
            proofPrefix,
            proofSuffix
        );

        (bool ok,) = address(0x0a).staticcall(input);
        if (ok) assert(false);
    }

    function checkSymbolicKzgVersionedHashMismatchReturnsCounterexample(bytes31 digestTail) public {
        bytes memory input = abi.encodePacked(
            bytes1(0x01),
            digestTail,
            hex"73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        (bool ok,) = address(0x0a).staticcall(input);
        assert(ok);
    }

    function checkSymbolicKzgProofInvalidWitnessReturnsCounterexample(
        bytes32 proofPrefix,
        bytes16 proofSuffix
    ) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            hex"73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            proofPrefix,
            proofSuffix
        );

        (bool ok,) = address(0x0a).staticcall(input);
        assert(ok);
    }

    function checkSymbolicKzgCommitmentMismatchReturnsCounterexample(
        bytes32 commitmentPrefix,
        bytes16 commitmentSuffix
    ) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            hex"73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            commitmentPrefix,
            commitmentSuffix,
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        (bool ok,) = address(0x0a).staticcall(input);
        assert(ok);
    }

    function checkSymbolicKzgKnownVersionMismatchFails(bytes32 z) public {
        bytes memory input = abi.encodePacked(
            hex"02e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            z,
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        (bool ok, bytes memory out) = address(0x0a).staticcall(input);
        assert(!ok);
        assertEq(out.length, 0);
    }

    function checkTaikoStylePackedBytes1ArrayKzgCallReturnsCounterexample(uint256) public {
        bytes1[48] memory commitment;
        bytes1[48] memory proof;
        bytes memory commitmentBytes =
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7";
        bytes memory proofBytes =
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c";

        for (uint256 i; i < 48; ++i) {
            commitment[i] = commitmentBytes[i];
            proof[i] = proofBytes[i];
        }

        (bool ok,) = address(0x0a).staticcall(
            abi.encodePacked(
                hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
                uint256(0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000),
                uint256(0x1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9),
                commitment,
                proof
            )
        );

        assert(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicKzgPrecompileInput"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicKzgKnownVersionMismatchFails(bytes32)
[FAIL: panic: assertion failed (0x01); counterexample:
checkSymbolicKzgCommitmentMismatchReturnsCounterexample(bytes32,bytes16)
checkSymbolicKzgInvalidWitnessReturnsCounterexample(bytes32)
checkSymbolicKzgProofInvalidWitnessReturnsCounterexample(bytes32,bytes16)
checkSymbolicKzgSuccessWitnessReturnsCounterexample(bytes32,bytes32,bytes32,bytes32,bytes16,bytes32,bytes16)
checkSymbolicKzgVersionedHashMismatchReturnsCounterexample(bytes31)
checkTaikoStylePackedBytes1ArrayKzgCallReturnsCounterexample(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic KZG point-evaluation precompile"), "{stdout}");
});

forgetest_init!(symbolic_kzg_precompile_explores_invalid_symbolic_length, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_kzg_precompile_explores_invalid_symbolic_length because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicKzgInvalidLength.t.sol",
        r#"
contract SymbolicKzgInvalidLength {
    function checkKzgInvalidLengthIsNotDropped(uint8 size) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            hex"73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000",
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        bool ok;
        assembly {
            ok := staticcall(gas(), 0x0a, add(input, 0x20), size, 0, 0)
        }

        if (!ok) assert(size == 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicKzgInvalidLength"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL: panic: assertion failed (0x01); counterexample:
checkKzgInvalidLengthIsNotDropped(uint8)
"#]],
    );
});

forgetest_init!(symbolic_kzg_precompile_inactive_before_cancun, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_kzg_precompile_inactive_before_cancun because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPreCancunKzg.t.sol",
        r#"
contract SymbolicPreCancunKzg {
    function checkAddress0aIsEmptyAccountBeforeCancun(uint256) public {
        bytes memory input = new bytes(191);
        (bool ok, bytes memory out) = address(0x0a).staticcall(input);

        assert(ok);
        assert(out.length == 0);

        uint256 codeSize;
        bytes32 codeHash;
        assembly {
            codeSize := extcodesize(0x0a)
            codeHash := extcodehash(0x0a)
        }

        assert(codeSize == 0);
        assert(codeHash == bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--evm-version",
            "shanghai",
            "--match-contract",
            "SymbolicPreCancunKzg",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkAddress0aIsEmptyAccountBeforeCancun(uint256)
"#]],
    );
});

forgetest_init!(symbolic_kzg_precompile_residual_reports_incomplete, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_kzg_precompile_residual_reports_incomplete because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicKzgResidual.t.sol",
        r#"
contract SymbolicKzgResidual {
    function checkUnmodeledKzgResidual(bytes32 z) public {
        bytes memory input = abi.encodePacked(
            hex"01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b",
            z,
            hex"1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9",
            hex"8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7",
            hex"a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        );

        address(0x0a).staticcall(input);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicKzgResidual"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(
        stdout.contains("symbolic KZG point-evaluation precompile residual not modeled"),
        "{stdout}"
    );
});
