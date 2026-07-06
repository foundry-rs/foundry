// CLI integration tests for `forge test --brutalize`

use std::{fs, str::FromStr};

use foundry_compilers::artifacts::remappings::Remapping;
use foundry_config::fs_permissions::PathPermission;
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

    function transferTo(address x) external {
        payable(x).transfer(0);
    }

    function dirtyInt16MinusOne() external pure returns (int16 value, uint256 raw) {
        value = int16(-1);
        assembly {
            raw := value
        }
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

    function test_transferTo() public {
        robust.transferTo(address(0xBEEF));
    }

    function test_dirtyInt16MinusOne() public view {
        (int16 value, uint256 raw) = robust.dirtyInt16MinusOne();
        assertEq(value, -1);
        assertTrue(raw != type(uint256).max);
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

forgetest_init!(brutalize_copies_fs_permission_fixtures, |prj, cmd| {
    let fixtures = prj.root().join("fixtures");
    fs::create_dir_all(&fixtures).unwrap();
    fs::write(fixtures.join("data.txt"), "fixture-data").unwrap();
    prj.update_config(|config| config.fs_permissions.add(PathPermission::read("./fixtures")));

    prj.add_test(
        "FixtureRead.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract FixtureReadTest is Test {
    function test_readFixture() public view {
        assertEq(vm.readFile("./fixtures/data.txt"), "fixture-data");
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--mt", "test_readFixture"]);
    cmd.assert_success();
});

forgetest_init!(brutalize_creates_write_permission_dirs, |prj, cmd| {
    let writes = prj.root().join("writes");
    fs::create_dir_all(&writes).unwrap();
    prj.update_config(|config| config.fs_permissions.add(PathPermission::write("./writes")));

    prj.add_test(
        "FixtureWrite.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract FixtureWriteTest is Test {
    function test_writeFixture() public {
        vm.writeFile("./writes/output.txt", "fixture-data");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "test_writeFixture"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["test", "--brutalize", "--mt", "test_writeFixture"]);
    cmd.assert_success();
});

#[cfg(not(target_os = "windows"))]
forgetest_init!(brutalize_copies_in_root_symlinked_source_dirs, |prj, cmd| {
    fs::create_dir_all(prj.root().join(".shared/src")).unwrap();
    fs::create_dir_all(prj.root().join(".shared/test")).unwrap();
    fs::create_dir_all(prj.root().join("src")).unwrap();
    fs::create_dir_all(prj.root().join("test")).unwrap();
    std::os::unix::fs::symlink(std::path::Path::new("../.shared/src"), prj.root().join("src/core"))
        .unwrap();
    std::os::unix::fs::symlink(
        std::path::Path::new("../.shared/test"),
        prj.root().join("test/suite"),
    )
    .unwrap();

    fs::write(
        prj.root().join(".shared/src/SymlinkTarget.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract SymlinkTarget {
    function readScratch() external pure returns (uint256 result) {
        assembly {
            result := mload(0x00)
        }
    }
}
"#,
    )
    .unwrap();

    fs::write(
        prj.root().join(".shared/test/SymlinkTarget.t.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../../src/core/SymlinkTarget.sol";

contract SymlinkTargetTest {
    function test_symlinkedSourceIsBrutalized() external {
        require(new SymlinkTarget().readScratch() != 0, "source was not brutalized");
    }
}
"#,
    )
    .unwrap();

    let stdout = cmd
        .args(["test", "--brutalize", "--mt", "test_symlinkedSourceIsBrutalized"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] test_symlinkedSourceIsBrutalized()"), "{stdout}");
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

// Catches dirty scratch space in the high half of the scratch word.
forgetest_init!(brutalize_catches_dirty_scratch_space_high_half, |prj, cmd| {
    prj.add_source(
        "ScratchHighVuln.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ScratchHighVuln {
    function readScratchHighHalf() external pure returns (uint256 result) {
        assembly {
            result := shr(128, mload(0x00))
        }
    }
}
"#,
    );

    prj.add_test(
        "ScratchHighVuln.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/ScratchHighVuln.sol";

contract ScratchHighVulnTest is Test {
    ScratchHighVuln public c;

    function setUp() public {
        c = new ScratchHighVuln();
    }

    function test_readScratchHighHalf() public view {
        assertEq(c.readScratchHighHalf(), 0);
    }
}
"#,
    );

    cmd.args(["test", "--mc", "ScratchHighVulnTest"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["test", "--brutalize", "--mc", "ScratchHighVulnTest"]);
    cmd.assert_failure();
});

// Catches dirty upper bits from narrow value casts.
forgetest_init!(brutalize_catches_dirty_value_bits, |prj, cmd| {
    prj.add_source(
        "ValueBitsVuln.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ValueBitsVuln {
    function rawBytes4(uint32 x) external pure returns (uint256 raw) {
        bytes4 y = bytes4(x);
        assembly {
            raw := y
        }
    }
}
"#,
    );

    prj.add_test(
        "ValueBitsVuln.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/ValueBitsVuln.sol";

contract ValueBitsVulnTest is Test {
    ValueBitsVuln public c;

    function setUp() public {
        c = new ValueBitsVuln();
    }

    function test_rawBytes4IsClean() public view {
        bytes4 expected = bytes4(uint32(0x12345678));
        assertEq(c.rawBytes4(0x12345678), uint256(bytes32(expected)));
    }
}
"#,
    );

    cmd.args(["test", "--mc", "ValueBitsVulnTest"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["test", "--brutalize", "--mc", "ValueBitsVulnTest"]);
    cmd.assert_failure();
});

// With src = ".", brutalization should skip tests, scripts, and libraries.
forgetest_init!(brutalize_flat_layout_only_brutalizes_sources, |prj, cmd| {
    prj.update_config(|config| {
        config.src = ".".into();
        config.test = ".".into();
    });

    fs::write(
        prj.root().join("FlatTarget.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract FlatTarget {
    function readScratch() external pure returns (uint256 result) {
        assembly {
            result := mload(0x00)
        }
    }
}
"#,
    )
    .unwrap();

    fs::write(
        prj.root().join("FlatTarget.t.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./FlatTarget.sol";

contract FlatTargetTest {
    function test_readScratch() external {
        assembly {
            let scratch := mload(0x00)
            pop(scratch)
        }
        require(new FlatTarget().readScratch() != 0, "source was not brutalized");
    }
}
"#,
    )
    .unwrap();

    let stderr = cmd
        .args(["test", "--brutalize", "--mt", "test_readScratch"])
        .assert_success()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("Brutalized 1 source files"), "{stderr}");
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

// --brutalize --rerun must read the original project's persisted failure list.
forgetest_init!(brutalize_rerun_uses_original_failure_cache, |prj, cmd| {
    prj.add_source(
        "RerunTarget.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract RerunTarget {
    function ok() external pure returns (uint256) {
        return 1;
    }
}
"#,
    );

    prj.add_test(
        "RerunTarget.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract RerunTargetTest {
    function test_fail() public pure {
        require(false, "record me");
    }

    function test_pass() public pure {
        require(true, "do not rerun");
    }
}
"#,
    );

    cmd.args(["test"]).assert_failure();
    assert!(prj.root().join("cache/test-failures").exists());

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--brutalize", "--rerun"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL: record me] test_fail()"), "{stdout}");
    assert!(!stdout.contains("[PASS] test_pass()"), "{stdout}");
});

forgetest_init!(brutalize_persists_original_failure_cache, |prj, cmd| {
    prj.add_test(
        "BrutalizeFailureCache.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BrutalizeFailureCacheTest {
    function test_failBrutalizedRun() public pure {
        require(false, "record brutalized failure");
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--mt", "test_failBrutalizedRun"]).assert_failure();

    let failures = fs::read_to_string(prj.root().join("cache/test-failures")).unwrap();
    assert!(failures.contains("test_failBrutalizedRun"), "{failures}");
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
