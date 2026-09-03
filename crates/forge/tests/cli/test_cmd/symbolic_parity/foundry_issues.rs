use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// foundry-rs/foundry#9782 — omitted struct-hash field.
// ---------------------------------------------------------------------------
// `toPoolId` omits `extension`. Breaking the property requires the four hashed
// fields to match while `extension` differs. Random fuzzing missed this narrow
// case, while symbolic execution solves it without searching for a hash collision.
forgetest_init!(issue_9782_struct_hash_field_omission, |prj, cmd| {
    skip_unless_z3!("issue_9782_struct_hash_field_omission");

    prj.add_source(
        "PoolId.sol",
        r#"
struct PoolKey {
    address token0;
    address token1;
    uint128 fee;
    uint32 tickSpacing;
    address extension;
}

function toPoolId(PoolKey memory key) pure returns (bytes32 result) {
    // Intentionally broken: omits `extension` from the hash.
    result = keccak256(abi.encode(key.token0, key.token1, key.fee, key.tickSpacing));
}
"#,
    );

    prj.add_test(
        "PoolId.t.sol",
        r#"
import "forge-std/Test.sol";
import {PoolKey, toPoolId} from "../src/PoolId.sol";

contract PoolIdSymbolicTest is Test {
    using {toPoolId} for PoolKey;

    function check_toPoolId_aligns_with_eq(PoolKey memory pk0, PoolKey memory pk1) external pure {
        bytes32 pk0Id = pk0.toPoolId();
        bytes32 pk1Id = pk1.toPoolId();

        assertEq(
            pk0.token0 == pk1.token0 && pk0.token1 == pk1.token1 && pk0.fee == pk1.fee
                && pk0.tickSpacing == pk1.tickSpacing && pk0.extension == pk1.extension,
            pk0Id == pk1Id
        );
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "check_toPoolId"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/PoolId.t.sol:PoolIdSymbolicTest
[FAIL: assertion failed: false != true; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] check_toPoolId_aligns_with_eq((address,address,uint128,uint32,address),(address,address,uint128,uint32,address)) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
