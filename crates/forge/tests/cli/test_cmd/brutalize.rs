// CLI integration tests for `forge test --brutalize`

use foundry_test_utils::{str, util::OutputExt};

// Basic brutalize: robust contract should pass all tests under brutalization
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

    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
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
        // Properly masks to 8 bits - robust against dirty upper bits
        assertEq(robust.toUint8(256), 0);
        assertEq(robust.toUint8(255), 255);
    }

    function test_toAddress() public view {
        assertEq(robust.toAddress(1), address(1));
    }

    function test_add() public view {
        assertEq(robust.add(1, 2), 3);
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

// Brutalize with --json: no progress messages pollute stdout
forgetest_init!(brutalize_json_output_clean, |prj, cmd| {
    prj.add_source(
        "Simple.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Simple {
    function id(uint256 x) external pure returns (uint256) {
        return x;
    }
}
"#,
    );

    prj.add_test(
        "Simple.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Simple.sol";

contract SimpleTest {
    Simple public simple;

    function setUp() public {
        simple = new Simple();
    }

    function test_Id() public {
        assert(simple.id(42) == 42);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize", "--json"]);
    let output = cmd.assert_success().get_output().stdout_lossy();
    // Should not contain progress messages
    assert!(!output.contains("Brutalizing"));
    assert!(!output.contains("Brutalized"));
    // Should be valid JSON (starts with { since it's test results)
    let trimmed = output.trim();
    assert!(trimmed.starts_with('{'), "JSON output should start with '{{', got: {trimmed}");
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

// No source files to brutalize: still passes (0 files brutalized)
forgetest_init!(brutalize_no_casts_no_assembly, |prj, cmd| {
    prj.add_source(
        "Plain.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Plain {
    uint256 public value;

    function set(uint256 v) external {
        value = v;
    }
}
"#,
    );

    prj.add_test(
        "Plain.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Plain.sol";

contract PlainTest {
    Plain public plain;

    function setUp() public {
        plain = new Plain();
    }

    function test_Set() public {
        plain.set(42);
        assert(plain.value() == 42);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Brutalized 0 source files, compiling from temp workspace...
...
"#]]);
});

// Brutalize with assembly: memory and FMP injections compile and tests pass
forgetest_init!(brutalize_assembly_function, |prj, cmd| {
    prj.add_source(
        "AsmContract.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract AsmContract {
    function asmAdd(uint256 a, uint256 b) external pure returns (uint256 result) {
        assembly {
            result := add(a, b)
        }
    }
}
"#,
    );

    prj.add_test(
        "AsmContract.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/AsmContract.sol";

contract AsmContractTest {
    AsmContract public c;

    function setUp() public {
        c = new AsmContract();
    }

    function test_AsmAdd() public {
        assert(c.asmAdd(2, 3) == 5);
    }
}
"#,
    );

    cmd.args(["test", "--brutalize"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Brutalized 1 source files, compiling from temp workspace...
...
Suite result: ok. [..] passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

// Brutalize with value casts: type narrowing XOR masks compile and tests pass
forgetest_init!(brutalize_value_casts, |prj, cmd| {
    prj.add_source(
        "Casts.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Casts {
    function narrow8(uint256 x) external pure returns (uint8) {
        return uint8(x);
    }

    function narrow16(uint256 x) external pure returns (uint16) {
        return uint16(x);
    }

    function narrowAddr(uint160 x) external pure returns (address) {
        return address(x);
    }

    function narrowBytes4(bytes32 x) external pure returns (bytes4) {
        return bytes4(x);
    }
}
"#,
    );

    prj.add_test(
        "Casts.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Casts.sol";

contract CastsTest is Test {
    Casts public c;

    function setUp() public {
        c = new Casts();
    }

    function test_Narrow8() public view {
        assertEq(c.narrow8(0x1FF), 0xFF);
        assertEq(c.narrow8(256), 0);
    }

    function test_Narrow16() public view {
        assertEq(c.narrow16(0x1FFFF), 0xFFFF);
    }

    function test_NarrowAddr() public view {
        assertEq(c.narrowAddr(1), address(1));
    }

    function test_NarrowBytes4() public view {
        assertEq(c.narrowBytes4(bytes32(uint256(1) << 248)), bytes4(uint32(1) << 24));
    }
}
"#,
    );

    cmd.args(["test", "--brutalize"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Brutalized 1 source files, compiling from temp workspace...
...
Suite result: ok. [..] passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

// Vulnerable: assembly allocates memory and reads it assuming zero
// The brutalizer fills memory past FMP with junk, so mload returns non-zero
forgetest_init!(brutalize_catches_uninitialized_memory_read, |prj, cmd| {
    prj.add_source(
        "MemVuln.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract MemVuln {
    // BUG: allocates memory via FMP bump then reads it assuming zero.
    // The brutalizer fills memory past FMP with junk before this runs.
    function allocAndRead() external pure returns (uint256 result) {
        assembly {
            let ptr := mload(0x40)
            // Skip 0x200 bytes into the region (well within the 1KB brutalizer fill)
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
