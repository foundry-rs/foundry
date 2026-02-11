// CLI integration tests for `forge test --brutalize`

use foundry_test_utils::str;

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
Brutalizing source files...
Brutalized 1 source files, compiling from temp workspace...
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
