// CLI integration tests for mutation testing

use foundry_test_utils::{str, util::OutputExt};

fn mutation_summary(stdout: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(stdout.trim()).unwrap()["summary"].clone()
}

forgetest_init!(can_run_mutation_testing, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
"#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Counter.sol";

contract CounterTest {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function test_Increment() public {
        counter.increment();
        assert(counter.number() == 1);
    }

    function test_SetNumber() public {
        counter.setNumber(42);
        assert(counter.number() == 42);
    }
}
"#,
    );

    // Run mutation testing
    cmd.args(["test", "--mutate", "src/Counter.sol", "--mutation-jobs", "1"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Running mutation tests with 1 parallel workers...
...
════════════════════════════════════════════════════════════
MUTATION TESTING RESULTS
════════════════════════════════════════════════════════════

╭──────────┬───────────┬────────────╮
│ Status   ┆ # Mutants ┆ % of Total │
╞══════════╪═══════════╪════════════╡
│ Survived ┆ 1         ┆ 14.3%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 4         ┆ 57.1%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 2         ┆ 28.6%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 0         ┆ 0.0%       │
╰──────────┴───────────┴────────────╯

Legend:
  Survived - tests did not catch the mutation
  Killed - tests caught the mutation
  Invalid - mutation produced a compilation error
  Skipped - redundant mutation on the same expression
  Timed out - compile/test exceeded the configured timeout

Mutation Score: 80.0% (4/5 mutants killed); [ELAPSED]

────────────────────────────────────────────────────────────
Survived mutants
────────────────────────────────────────────────────────────

...
     number++;
     Mutation:
       - number++
       + ++number
...
────────────────────────────────────────────────────────────
4 mutants killed

────────────────────────────────────────────────────────────
2 mutants invalid

════════════════════════════════════════════════════════════

"#]]);

    // Run mutation testing with --json - verify the output contains valid mutation JSON
    cmd.forge_fuse().args(["test", "--mutate", "src/Counter.sol", "--mutation-jobs", "1", "--json"]).assert_success().stdout_eq(str![[r#"
{"summary":{"total":7,"killed":4,"survived":1,"invalid":2,"skipped":0,"timed_out":0,"mutation_score":80.0,"duration_secs":[..]},"survived_mutants":{"src/Counter.sol":[{"line":13,"column":9,"original":"number++","mutant":"++number"}]}}

"#]]);
});

forgetest_init!(mutation_testing_with_parallel_workers, |prj, cmd| {
    prj.add_source(
        "Simple.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Simple {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
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

    function test_Add() public {
        assert(simple.add(1, 2) == 3);
    }
}
"#,
    );

    // Run mutation testing with 4 workers
    cmd.args(["test", "--mutate", "src/Simple.sol", "--mutation-jobs", "4"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Running mutation tests with 4 parallel workers...
...
════════════════════════════════════════════════════════════
MUTATION TESTING RESULTS
════════════════════════════════════════════════════════════

╭──────────┬───────────┬────────────╮
│ Status   ┆ # Mutants ┆ % of Total │
╞══════════╪═══════════╪════════════╡
│ Survived ┆ 2         ┆ 18.2%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 8         ┆ 72.7%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 1         ┆ 9.1%       │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 0         ┆ 0.0%       │
╰──────────┴───────────┴────────────╯
...
Mutation Score: 80.0% (8/10 mutants killed); [ELAPSED]
...
"#]]);
});

forgetest_init!(mutation_testing_with_show_progress, |prj, cmd| {
    prj.add_source(
        "Simple.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Simple {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
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

    function test_Add() public {
        assert(simple.add(1, 2) == 3);
    }
}
"#,
    );

    // Run mutation testing with progress display (use 4 workers like parallel test for consistency)
    cmd.args(["test", "--mutate", "src/Simple.sol", "--show-progress", "--mutation-jobs", "4"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
════════════════════════════════════════════════════════════
MUTATION TESTING RESULTS
════════════════════════════════════════════════════════════

╭──────────┬───────────┬────────────╮
│ Status   ┆ # Mutants ┆ % of Total │
╞══════════╪═══════════╪════════════╡
│ Survived ┆ 2         ┆ 18.2%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 8         ┆ 72.7%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 1         ┆ 9.1%       │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 0         ┆ 0.0%       │
╰──────────┴───────────┴────────────╯

Legend:
  Survived - tests did not catch the mutation
  Killed - tests caught the mutation
  Invalid - mutation produced a compilation error
  Skipped - redundant mutation on the same expression
  Timed out - compile/test exceeded the configured timeout

Mutation Score: 80.0% (8/10 mutants killed); [ELAPSED]

────────────────────────────────────────────────────────────
Survived mutants
────────────────────────────────────────────────────────────

...
     return a + b;
     Mutation:
       - a + b
...
     return a + b;
     Mutation:
       - a + b
...
────────────────────────────────────────────────────────────
8 mutants killed

────────────────────────────────────────────────────────────
1 mutants invalid

════════════════════════════════════════════════════════════

"#]]);
});

forgetest_init!(mutation_result_cache_invalidates_when_tests_change, |prj, _cmd| {
    prj.add_source(
        "Calculator.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Calculator {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_test(
        "Calculator.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Calculator.sol";

contract CalculatorTest {
    Calculator public calculator;

    function setUp() public {
        calculator = new Calculator();
    }

    function test_Add() public view {
        calculator.add(1, 2);
    }
}
"#,
    );

    let mut weak_cmd = prj.forge_command();
    let weak_stdout = weak_cmd
        .args(["test", "--mutate", "src/Calculator.sol", "--mutation-jobs", "1", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let weak_summary = mutation_summary(&weak_stdout);

    prj.add_test(
        "Calculator.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Calculator.sol";

contract CalculatorTest {
    Calculator public calculator;

    function setUp() public {
        calculator = new Calculator();
    }

    function test_Add() public view {
        assert(calculator.add(1, 2) == 3);
    }
}
"#,
    );

    let mut strong_cmd = prj.forge_command();
    let strong_stdout = strong_cmd
        .args(["test", "--mutate", "src/Calculator.sol", "--mutation-jobs", "1", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let strong_summary = mutation_summary(&strong_stdout);

    assert_eq!(weak_summary["total"], strong_summary["total"]);
    assert!(
        strong_summary["killed"].as_u64().unwrap() > weak_summary["killed"].as_u64().unwrap(),
        "expected changed tests to invalidate cached mutation results: weak={weak_summary}, strong={strong_summary}",
    );
});

// Test require/assert mutation for security-critical patterns
forgetest_init!(mutation_testing_require_mutator, |prj, cmd| {
    // A contract with security-critical require checks (access control, input validation)
    prj.add_source(
        "Vault.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Vault {
    address public owner;
    mapping(address => uint256) public balances;
    bool public paused;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    function deposit() public payable {
        require(!paused, "Contract paused");
        require(msg.value > 0, "Must send ETH");
        balances[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) public {
        require(!paused, "Contract paused");
        require(balances[msg.sender] >= amount, "Insufficient balance");
        balances[msg.sender] -= amount;
        payable(msg.sender).transfer(amount);
    }

    function pause() public onlyOwner {
        paused = true;
    }

    function unpause() public onlyOwner {
        paused = false;
    }
}
"#,
    );

    prj.add_test(
        "Vault.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Vault.sol";

contract VaultTest is Test {
    Vault public vault;

    function setUp() public {
        vault = new Vault();
    }

    // === DEPOSIT TESTS ===

    function test_DepositRequiresValue() public {
        vm.expectRevert("Must send ETH");
        vault.deposit();
    }

    function test_DepositWithValue() public {
        vault.deposit{value: 1 ether}();
        assertEq(vault.balances(address(this)), 1 ether);
    }

    function test_DepositWhenPausedReverts() public {
        vault.pause();
        vm.expectRevert("Contract paused");
        vault.deposit{value: 1 ether}();
    }

    // === WITHDRAW TESTS ===

    function test_WithdrawRequiresBalance() public {
        vm.expectRevert("Insufficient balance");
        vault.withdraw(1 ether);
    }

    function test_WithdrawExactBalance() public {
        // Kills >= -> > and >= -> != mutations
        vault.deposit{value: 1 ether}();
        vault.withdraw(1 ether);
        assertEq(vault.balances(address(this)), 0);
    }

    function test_WithdrawPartialBalance() public {
        vault.deposit{value: 1 ether}();
        vault.withdraw(0.5 ether);
        assertEq(vault.balances(address(this)), 0.5 ether);
    }

    function test_WithdrawWhenPausedReverts() public {
        vault.deposit{value: 1 ether}();
        vault.pause();
        vm.expectRevert("Contract paused");
        vault.withdraw(0.5 ether);
    }

    // === PAUSE/UNPAUSE TESTS ===

    function test_OnlyOwnerCanPause() public {
        vault.pause();
        assertTrue(vault.paused());
    }

    function test_NonOwnerCannotPause() public {
        vm.prank(address(1));
        vm.expectRevert("Not owner");
        vault.pause();
    }

    function test_OnlyOwnerCanUnpause() public {
        vault.pause();
        vault.unpause();
        assertFalse(vault.paused());
    }

    function test_NonOwnerCannotUnpause() public {
        vault.pause();
        vm.prank(address(1));
        vm.expectRevert("Not owner");
        vault.unpause();
    }

    function test_UnpauseAllowsDeposit() public {
        vault.pause();
        vault.unpause();
        vault.deposit{value: 1 ether}();
        assertEq(vault.balances(address(this)), 1 ether);
    }

    // Kills >= mutation on address comparison
    function test_HigherAddressCannotPause() public {
        vm.prank(address(type(uint160).max));
        vm.expectRevert("Not owner");
        vault.pause();
    }

    receive() external payable {}
}
"#,
    );

    // The surviving mutant (msg.value > 0 -> msg.value != 0) is equivalent for uint256
    let mut cmd2 = prj.forge_command();
    cmd2.args(["test", "--mutate", "src/Vault.sol", "--mutation-jobs", "2"]);
    cmd2.assert_success().stdout_eq(str![[r#"
...
Running mutation tests with 2 parallel workers...
...
════════════════════════════════════════════════════════════
MUTATION TESTING RESULTS
════════════════════════════════════════════════════════════

╭──────────┬───────────┬────────────╮
│ Status   ┆ # Mutants ┆ % of Total │
╞══════════╪═══════════╪════════════╡
│ Survived ┆ 3         ┆ 4.8%       │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 45        ┆ 72.6%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 11        ┆ 17.7%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 3         ┆ 4.8%       │
╰──────────┴───────────┴────────────╯
...
Mutation Score: 93.8% (45/48 mutants killed); [ELAPSED]
...
"#]]);
});

forgetest_init!(mutation_testing_assembly_code, |prj, cmd| {
    prj.add_source(
        "AsmMath.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library AsmMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := add(a, b)
        }
    }

    function sub(uint256 a, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := sub(a, b)
        }
    }
}
"#,
    );

    prj.add_test(
        "AsmMath.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/AsmMath.sol";

contract AsmMathTest {
    using AsmMath for uint256;

    function test_Add() public pure {
        assert(uint256(2).add(3) == 5);
        assert(uint256(0).add(0) == 0);
    }

    function test_Sub() public pure {
        assert(uint256(5).sub(3) == 2);
    }
}
"#,
    );

    cmd.args(["test", "--mutate", "src/AsmMath.sol", "--mutation-jobs", "1"]);
    cmd.assert_success().stdout_eq(str![[r#"
...
Running mutation tests with 1 parallel workers...
...
════════════════════════════════════════════════════════════
MUTATION TESTING RESULTS
════════════════════════════════════════════════════════════
...
"#]]);
});
