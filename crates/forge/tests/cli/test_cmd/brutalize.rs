// CLI integration tests for `forge test --brutalize`

use std::str::FromStr;

use foundry_compilers::artifacts::remappings::Remapping;
use foundry_test_utils::{str, util::OutputExt};

// Robust contract with casts + assembly passes under brutalization
forgetest_init!(brutalize_robust_contract_passes, |prj, cmd| {
    prj.add_source(
        "Robust.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Robust {
    function toUint8(uint256 x) external pure returns (uint8) {
        return uint8(x);
    }

    function toAddress(uint160 x) external pure returns (address) {
        return address(x);
    }

    function addressThis() external view returns (address) {
        return address(this);
    }

    function addressToUint160(address x) external pure returns (uint160) {
        return uint160(x);
    }

    function negativeToInt16() external pure returns (int16) {
        return int16(-1);
    }

    function uint32ToBytes4(uint32 x) external pure returns (bytes4) {
        return bytes4(x);
    }

    function bytes32ToBytes31(bytes32 x) external pure returns (bytes31) {
        return bytes31(x);
    }

    function asmAdd(uint256 a, uint256 b) external pure returns (uint256 result) {
        assembly { result := add(a, b) }
    }

    function hashPair(uint256 a, uint256 b) external pure returns (bytes32 result) {
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            result := keccak256(0x00, 0x40)
        }
    }

    // Mixed Solidity + assembly: Solidity manages memory, assembly reads it.
    // Robust because it uses abi.encodePacked (which properly allocates via FMP)
    // then reads back through the pointer Solidity returned.
    function mixedHash(uint256 a, uint256 b) external pure returns (bytes32) {
        bytes memory packed = abi.encodePacked(a, b);
        bytes32 result;
        assembly {
            result := keccak256(add(packed, 0x20), mload(packed))
        }
        return result;
    }

    // Assembly allocates properly via FMP, then Solidity uses the result.
    function asmAllocThenSolidity(uint256 val) external pure returns (bytes32) {
        bytes32 hash;
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, val)
            hash := keccak256(ptr, 0x20)
            mstore(0x40, add(ptr, 0x20))
        }
        return keccak256(abi.encodePacked(hash));
    }
}
"#,
    );

    prj.add_test(
        "Robust.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Robust.sol";

contract RobustTest is Test {
    Robust public robust;

    function setUp() public {
        robust = new Robust();
    }

    function test_toUint8() public view {
        assertEq(robust.toUint8(256), 0);
        assertEq(robust.toUint8(255), 255);
    }

    function test_toAddress() public view {
        assertEq(robust.toAddress(1), address(1));
    }

    function test_addressThis() public view {
        assertEq(robust.addressThis(), address(robust));
    }

    function test_addressToUint160() public view {
        assertEq(robust.addressToUint160(address(1)), 1);
    }

    function test_negativeToInt16() public view {
        assertEq(robust.negativeToInt16(), -1);
    }

    function test_uint32ToBytes4() public view {
        assertEq(robust.uint32ToBytes4(0x12345678), bytes4(uint32(0x12345678)));
    }

    function test_bytes32ToBytes31() public view {
        bytes32 value = hex"ffffffffffffffffffffffffffffffffffffffffffffffff00000000000000aa";
        assertEq(robust.bytes32ToBytes31(value), bytes31(value));
    }

    function test_asmAdd() public view {
        assertEq(robust.asmAdd(2, 3), 5);
    }

    function test_hashPair() public view {
        bytes32 expected = keccak256(abi.encodePacked(uint256(1), uint256(2)));
        assertEq(robust.hashPair(1, 2), expected);
    }

    function test_mixedHash() public view {
        bytes32 expected = keccak256(abi.encodePacked(uint256(10), uint256(20)));
        assertEq(robust.mixedHash(10, 20), expected);
    }

    function test_asmAllocThenSolidity() public view {
        bytes32 inner = keccak256(abi.encodePacked(uint256(42)));
        bytes32 expected = keccak256(abi.encodePacked(inner));
        assertEq(robust.asmAllocThenSolidity(42), expected);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Suite result: ok. [..] passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

// --brutalize and --mutate are mutually exclusive
forgetest_init!(brutalize_conflicts_with_mutate, |prj, cmd| {
    prj.add_source(
        "Dummy.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;
contract Dummy { function f() external pure returns (uint256) { return 1; } }
"#,
    );

    cmd.args(["test", "--brutalize", "--mutate", "src/Dummy.sol"]);
    cmd.assert_failure().stderr_eq(str![[r#"
...
error: the argument '--brutalize' cannot be used with '--mutate [<PATH>...]'
...
"#]]);
});

// Catches uninitialized memory: assembly reads past FMP assuming zero
forgetest_init!(brutalize_catches_uninitialized_memory_read, |prj, cmd| {
    prj.add_source(
        "MemVuln.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract MemVuln {
    function allocAndRead() external pure returns (uint256 result) {
        assembly {
            let ptr := mload(0x40)
            result := mload(add(ptr, 0x200))
            mstore(0x40, add(ptr, 0x220))
        }
    }
}
"#,
    );

    prj.add_test(
        "MemVuln.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/MemVuln.sol";

contract MemVulnTest is Test {
    MemVuln public c;

    function setUp() public {
        c = new MemVuln();
    }

    function test_AllocAndRead() public view {
        assertEq(c.allocAndRead(), 0);
    }
}
"#,
    );

    // Normal test passes — freshly allocated memory is zero
    cmd.args(["test", "--mc", "MemVulnTest"]);
    cmd.assert_success();

    // Brutalized test fails — memory past FMP is filled with junk
    cmd.forge_fuse().args(["test", "--brutalize", "--mc", "MemVulnTest"]);
    cmd.assert_failure();
});

// Catches dirty scratch space: reading 0x00 without writing first
forgetest_init!(brutalize_catches_dirty_scratch_space, |prj, cmd| {
    prj.add_source(
        "ScratchVuln.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ScratchVuln {
    function readScratch() external pure returns (uint256 result) {
        assembly {
            result := mload(0x00)
        }
    }
}
"#,
    );

    prj.add_test(
        "ScratchVuln.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/ScratchVuln.sol";

contract ScratchVulnTest is Test {
    ScratchVuln public c;

    function setUp() public {
        c = new ScratchVuln();
    }

    function test_readScratch() public view {
        assertEq(c.readScratch(), 0);
    }
}
"#,
    );

    cmd.args(["test", "--mc", "ScratchVulnTest"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["test", "--brutalize", "--mc", "ScratchVulnTest"]);
    cmd.assert_failure();
});

// --brutalize works with --match-test filter (regression for .sanitized() config fix)
forgetest_init!(brutalize_with_filter, |prj, cmd| {
    prj.add_source(
        "FilterTarget.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract FilterTarget {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_test(
        "FilterTarget.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/FilterTarget.sol";

contract FilterTargetTest is Test {
    FilterTarget public c;

    function setUp() public {
        c = new FilterTarget();
    }

    function test_add() public view {
        assertEq(c.add(1, 2), 3);
    }

    function test_addZero() public view {
        assertEq(c.add(0, 0), 0);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--mt", "test_add"]);
    cmd.assert_success();
});

// --brutalize must preserve CLI/config overrides when compiling from the temp workspace.
forgetest_init!(brutalize_preserves_fuzz_runs_override, |prj, cmd| {
    prj.add_source(
        "FuzzTarget.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract FuzzTarget {
    function identity(uint256 x) external pure returns (uint256) {
        return x;
    }
}
"#,
    );

    prj.add_test(
        "FuzzTarget.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/FuzzTarget.sol";

contract FuzzTargetTest is Test {
    FuzzTarget public c;

    function setUp() public {
        c = new FuzzTarget();
    }

    function test_fuzzIdentity(uint256 x) public view {
        assertEq(c.identity(x), x);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--brutalize", "--fuzz-runs", "1", "--mt", "test_fuzzIdentity"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("test_fuzzIdentity(uint256) (runs: 1,"), "{stdout}");
});

// --brutalize status output must not corrupt JUnit XML on stdout.
forgetest_init!(brutalize_junit_stdout_is_xml, |prj, cmd| {
    prj.add_source(
        "JUnitTarget.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract JUnitTarget {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_test(
        "JUnitTarget.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/JUnitTarget.sol";

contract JUnitTargetTest is Test {
    function test_add() public {
        assertEq(new JUnitTarget().add(1, 2), 3);
    }
}
"#,
    );

    let stdout =
        cmd.args(["test", "--brutalize", "--junit"]).assert_success().get_output().stdout_lossy();
    assert!(stdout.trim_start().starts_with("<?xml"), "{stdout}");
    assert!(!stdout.contains("Brutalizing source files"), "{stdout}");
    assert!(!stdout.contains("Brutalized 1 source files"), "{stdout}");
});

// Project-local remappings must resolve to the copied/brutalized temp workspace, not the original.
forgetest_init!(brutalize_rebases_project_local_remappings, |prj, cmd| {
    prj.update_config(|config| {
        config.auto_detect_remappings = false;
        config.remappings = vec![Remapping::from_str("@src/=src/").unwrap().into()];
    });

    prj.add_source(
        "RemappedTarget.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract RemappedTarget {
    function freeMemoryPointer() external pure returns (uint256 ptr) {
        assembly {
            ptr := mload(0x40)
        }
    }
}
"#,
    );

    prj.add_test(
        "RemappedTarget.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "@src/RemappedTarget.sol";

contract RemappedTargetTest {
    function test_remappedImportUsesBrutalizedTempSource() public {
        if (new RemappedTarget().freeMemoryPointer() == 0x80) {
            revert("remapping used original source");
        }
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--mt", "test_remappedImportUsesBrutalizedTempSource"]);
    cmd.assert_success();
});

// Nested casts should not produce overlapping replacements.
forgetest_init!(brutalize_nested_casts_compile, |prj, cmd| {
    prj.add_source(
        "NestedCasts.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract NestedCasts {
    function nested(uint256 x) external pure returns (uint8) {
        return uint8(uint16(x));
    }
}
"#,
    );

    prj.add_test(
        "NestedCasts.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/NestedCasts.sol";

contract NestedCastsTest is Test {
    NestedCasts public c;

    function setUp() public {
        c = new NestedCasts();
    }

    function test_nested() public view {
        assertEq(c.nested(0x1234), 0x34);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--mc", "NestedCastsTest"]);
    cmd.assert_success();
});
