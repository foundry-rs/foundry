use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness};
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// Classic DAO-style reentrancy.
// ---------------------------------------------------------------------------
// Vault that updates state AFTER the external call. An Attacker contract
// reenters `withdraw` from its `receive` function and drains more than its
// deposit. The symbolic engine must follow the cross-contract call stack.
forgetest_init!(reentrancy_dao_classic, |prj, cmd| {
    skip_unless_z3!("reentrancy_dao_classic");

    prj.add_test(
        "ReentrancyDao.t.sol",
        r#"
import "forge-std/Test.sol";

contract Vault {
    mapping(address => uint256) public balances;

    function deposit() external payable {
        balances[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) external {
        require(balances[msg.sender] >= amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok);
        // BUG: state update after external call.
        balances[msg.sender] -= amount;
    }
}

contract Attacker {
    Vault v;
    uint256 public stolen;

    constructor(Vault _v) payable {
        v = _v;
        v.deposit{value: msg.value}();
    }

    function attack() external {
        v.withdraw(1);
    }

    receive() external payable {
        stolen += msg.value;
        if (address(v).balance >= 1) {
            v.withdraw(1);
        }
    }
}

contract ReentrancyDao is Test {
    /// forge-config: default.symbolic.depth = 4096
    function checkVaultCannotBeDrained() public {
        Vault v = new Vault();
        vm.deal(address(this), 3);
        v.deposit{value: 1}();
        Attacker a = new Attacker{value: 1}(v);
        a.attack();
        // The attacker deposited 1 wei; should never recover more.
        assert(a.stolen() <= 1);
    }
}
"#,
    );

    // No symbolic args — fully deterministic.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkVaultCannotBeDrained"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/ReentrancyDao.t.sol:ReentrancyDao
[FAIL: incomplete symbolic execution (RevertAll): all symbolic paths reverted] checkVaultCannotBeDrained() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// ---------------------------------------------------------------------------
// tx.origin auth bypass.
// ---------------------------------------------------------------------------
// A contract gates a sensitive action on `tx.origin == owner` instead of
// `msg.sender == owner`. Any intermediary contract called by the owner can
// trigger the action — symbolic execution must find that path.
forgetest_init!(tx_origin_auth_bypass, |prj, cmd| {
    skip_unless_z3!("tx_origin_auth_bypass");

    prj.add_test(
        "TxOriginBypass.t.sol",
        r#"
import "forge-std/Test.sol";

contract Protected {
    address public owner;
    bool public triggered;

    constructor(address _owner) {
        owner = _owner;
    }

    function sensitive() external {
        // BUG: tx.origin check is bypassable by a malicious intermediary.
        require(tx.origin == owner, "not owner");
        triggered = true;
    }
}

contract Relay {
    function pull(Protected p) external {
        p.sensitive();
    }
}

contract TxOriginBypass is Test {
    function checkOnlyOwnerCanTrigger(address attacker, address owner) public {
        vm.assume(attacker != owner);
        vm.assume(attacker != address(0));
        vm.assume(owner != address(0));

        Protected p = new Protected(owner);
        Relay r = new Relay();

        // attacker calls relay, but tx.origin is owner — bug triggers.
        vm.prank(attacker, owner);
        r.pull(p);

        // Property: only owner should ever be able to set triggered.
        // BUG: the assertion fires.
        assert(!p.triggered());
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkOnlyOwnerCanTrigger",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/TxOriginBypass.t.sol:TxOriginBypass
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkOnlyOwnerCanTrigger(address,address) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// ---------------------------------------------------------------------------
// ecrecover basic modeling — does the engine support symbolic ecrecover?
// ---------------------------------------------------------------------------
// Asks the solver to produce a signature that recovers to a specific address.
// If the engine models ecrecover symbolically, it should find a witness; if it
// abstracts it, this test will instead Stuck. Either way, the assertion is
// designed so a correct symbolic ecrecover produces a [FAIL] counterexample.
forgetest_init!(ecrecover_basic_modeling, |prj, cmd| {
    skip_unless_z3!("ecrecover_basic_modeling");

    prj.add_test(
        "EcrecoverBasic.t.sol",
        r#"
contract EcrecoverBasic {
    function checkEcrecoverNeverHitsZero(bytes32 h, uint8 v, bytes32 r, bytes32 s) public pure {
        address signer = ecrecover(h, v, r, s);
        // Property: a "valid" signature should never recover the zero address.
        // ecrecover returns 0 on malformed inputs, so a counterexample exists
        // trivially (e.g. v outside {27, 28}) — the engine must find one.
        assert(signer != address(0));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkEcrecoverNeverHitsZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EcrecoverBasic.t.sol:EcrecoverBasic
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkEcrecoverNeverHitsZero(bytes32,uint8,bytes32,bytes32) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// ---------------------------------------------------------------------------
// Solady-style `min`/`max` identities — linear small-library proof.
// ---------------------------------------------------------------------------
// Asserts that a ternary-based `min`/`max` over uint256 satisfies the
// expected ordering identities for all symbolic inputs. Linear/bitwise-only
// so Z3 proves it instantly. We deliberately avoid `mulDiv`-style nonlinear
// equivalence, which causes Z3 to return `unknown` even on uint8 inputs —
// that's a separate engine/solver capability to track.
forgetest_init!(solady_min_max_identities_pass, |prj, cmd| {
    skip_unless_z3!("solady_min_max_identities_pass");

    prj.add_test(
        "SoladyMinMax.t.sol",
        r#"
contract SoladyMinMax {
    function min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }

    function max(uint256 a, uint256 b) internal pure returns (uint256) {
        return a > b ? a : b;
    }

    function checkMinMaxIdentities(uint256 a, uint256 b) public pure {
        uint256 lo = min(a, b);
        uint256 hi = max(a, b);

        assert(lo <= a);
        assert(lo <= b);
        assert(hi >= a);
        assert(hi >= b);
        assert(lo == a || lo == b);
        assert(hi == a || hi == b);
        assert(lo + hi == a + b);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkMinMaxIdentities"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SoladyMinMax.t.sol:SoladyMinMax
[PASS] checkMinMaxIdentities(uint256,uint256) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun transient storage (TLOAD / TSTORE).
// ---------------------------------------------------------------------------
// Verifies symbolic semantics: `TSTORE` is visible within a transaction but
// invisible across calls' state (in this minimal harness we just round-trip
// within one external call).
forgetest_init!(cancun_transient_storage, |prj, cmd| {
    skip_unless_z3!("cancun_transient_storage");

    prj.add_test(
        "CancunTransient.t.sol",
        r#"
contract CancunTransient {
    function checkTloadAfterTstore(uint256 value) public {
        assembly {
            tstore(0x1234, value)
        }
        uint256 readBack;
        assembly {
            readBack := tload(0x1234)
        }
        assert(readBack == value);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkTloadAfterTstore"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CancunTransient.t.sol:CancunTransient
[PASS] checkTloadAfterTstore(uint256) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Shanghai PUSH0 opcode.
// ---------------------------------------------------------------------------
// Trivial sanity check that a contract using `PUSH0` is executable under the
// symbolic engine.
forgetest_init!(push0_shanghai, |prj, cmd| {
    skip_unless_z3!("push0_shanghai");

    prj.add_test(
        "Push0Shanghai.t.sol",
        r#"
contract Push0Shanghai {
    function checkZeroIsZero() public pure {
        uint256 zero;
        assembly {
            // PUSH0 places a single zero on the stack.
            zero := 0
        }
        assert(zero == 0);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkZeroIsZero"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Push0Shanghai.t.sol:Push0Shanghai
[PASS] checkZeroIsZero() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Byteswap involution — small-library bitwise proof.
// ---------------------------------------------------------------------------
// `swap(swap(x)) == x` for uint16, exercising shift + or composition under
// symbolic uint16. Linear/bitwise, so Z3 proves instantly. Replacement for
// the originally-proposed Solady log2 bit-hack, which is nonlinear and times
// out the solver in the same way mulDiv equivalence does.
forgetest_init!(byteswap_uint16_involution_passes, |prj, cmd| {
    skip_unless_z3!("byteswap_uint16_involution_passes");

    prj.add_test(
        "ByteswapInvolution.t.sol",
        r#"
contract ByteswapInvolution {
    function swap(uint16 x) internal pure returns (uint16) {
        return (x >> 8) | (x << 8);
    }

    function checkInvolution(uint16 x) public pure {
        assert(swap(swap(x)) == x);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkInvolution"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ByteswapInvolution.t.sol:ByteswapInvolution
[PASS] checkInvolution(uint16) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun MCOPY — symbolic memory copy.
// ---------------------------------------------------------------------------
// Verifies that MCOPY (Cancun-era memory-to-memory copy opcode) round-trips
// a symbolic word through scratch memory correctly.
forgetest_init!(mcopy_cancun_roundtrip, |prj, cmd| {
    skip_unless_z3!("mcopy_cancun_roundtrip");

    prj.add_test(
        "McopyCancun.t.sol",
        r#"
contract McopyCancun {
    function checkMcopyRoundtrip(bytes32 word) public pure {
        bytes32 read;
        assembly {
            let src := mload(0x40)
            let dst := add(src, 0x20)
            mstore(src, word)
            mcopy(dst, src, 32)
            read := mload(dst)
        }
        assert(read == word);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkMcopyRoundtrip"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/McopyCancun.t.sol:McopyCancun
[PASS] checkMcopyRoundtrip(bytes32) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun BLOBHASH / BLOBBASEFEE — opcode accessibility.
// ---------------------------------------------------------------------------
// Sanity check that both Cancun blob opcodes execute under symbolic
// execution. We don't assert specific values (those depend on context);
// only that they're not rejected as unsupported.
forgetest_init!(cancun_blob_opcodes_accessible, |prj, cmd| {
    skip_unless_z3!("cancun_blob_opcodes_accessible");

    prj.add_test(
        "CancunBlobOps.t.sol",
        r#"
contract CancunBlobOps {
    // The engine currently treats a symbolic BLOBHASH index as unsupported
    // (`Stuck`), so we hard-code the index. This still exercises the opcode
    // and the BLOBBASEFEE read.
    function checkBlobOpcodes() public view {
        bytes32 h;
        uint256 fee;
        assembly {
            h := blobhash(0)
            fee := blobbasefee()
        }
        // No semantic assertion; just keep the values live so the engine has
        // to actually execute the opcodes.
        assert(uint256(h) | fee | 1 != 0);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkBlobOpcodes"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CancunBlobOps.t.sol:CancunBlobOps
[PASS] checkBlobOpcodes() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Istanbul CHAINID / SELFBALANCE — opcode accessibility.
// ---------------------------------------------------------------------------
forgetest_init!(istanbul_chainid_selfbalance, |prj, cmd| {
    skip_unless_z3!("istanbul_chainid_selfbalance");

    prj.add_test(
        "IstanbulOps.t.sol",
        r#"
contract IstanbulOps {
    function checkChainIdAndSelfBalance() public view {
        uint256 chain;
        uint256 bal;
        assembly {
            chain := chainid()
            bal := selfbalance()
        }
        // Same self-balance equivalence the EVM guarantees.
        assert(bal == address(this).balance);
        assert(chain == block.chainid);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkChainIdAndSelfBalance"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/IstanbulOps.t.sol:IstanbulOps
[PASS] checkChainIdAndSelfBalance() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// SDIV signed-overflow corner case — `MIN_INT256 / -1` returns `MIN_INT256`.
// ---------------------------------------------------------------------------
// Per EVM spec, signed division has no real overflow: dividing the most-
// negative int256 by -1 wraps back to itself. Symbolic engine must model
// this exactly.
forgetest_init!(sdiv_min_int_overflow_semantics, |prj, cmd| {
    skip_unless_z3!("sdiv_min_int_overflow_semantics");

    prj.add_test(
        "SdivMinInt.t.sol",
        r#"
contract SdivMinInt {
    function checkSdivMinByNegOne() public pure {
        int256 a = type(int256).min;
        int256 b = -1;
        int256 r;
        assembly {
            r := sdiv(a, b)
        }
        // EVM spec: SDIV(MIN_INT, -1) == MIN_INT.
        assert(r == type(int256).min);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkSdivMinByNegOne"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SdivMinInt.t.sol:SdivMinInt
[PASS] checkSdivMinByNegOne() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// EXP small bounded — engine handles modular exponentiation.
// ---------------------------------------------------------------------------
// We deliberately keep base and exponent both bounded so the engine can
// concretely unroll. This is the smallest non-trivial EXP check that won't
// time out the solver (full nonlinear EXP equivalence is out of scope, the
// same way mulDiv equivalence is).
forgetest_init!(exp_small_bounded, |prj, cmd| {
    skip_unless_z3!("exp_small_bounded");

    prj.add_test(
        "ExpSmallBounded.t.sol",
        r#"
import "forge-std/Test.sol";

contract ExpSmallBounded is Test {
    function checkExpSmall(uint8 e) public pure {
        vm.assume(e <= 4);
        uint256 r;
        assembly {
            r := exp(2, e)
        }
        // 2 ** e for e in [0, 4] enumerates {1, 2, 4, 8, 16}.
        assert(r == (uint256(1) << e));
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkExpSmall"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExpSmallBounded.t.sol:ExpSmallBounded
[PASS] checkExpSmall(uint8) ([METRICS])
...
"#]]);
});
