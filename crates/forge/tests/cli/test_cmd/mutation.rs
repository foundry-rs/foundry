// CLI integration tests for mutation testing

use foundry_test_utils::str;

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
  Survived - Mutant survived: tests did not catch this mutation (potential gap)
  Killed - Mutant killed: tests caught this mutation (good coverage)
  Invalid - Mutant invalid: mutation caused compilation error
  Skipped - Mutant skipped: redundant mutation on same expression

Mutation Score: 80.0% (4/5 mutants killed); [ELAPSED]

────────────────────────────────────────────────────────────
⚠ SURVIVED MUTANTS (test suite gaps)
────────────────────────────────────────────────────────────
These mutations were NOT caught by your tests.
Each represents a potential bug that your tests would miss.
...
     number++;
     Mutation:
       - number++
       + ++number
...
────────────────────────────────────────────────────────────
✓ 4 mutants killed (tests caught these mutations)

────────────────────────────────────────────────────────────
ℹ 2 invalid mutants (compilation failures - expected for some mutations)

════════════════════════════════════════════════════════════

"#]]);

    // Run mutation testing with --json - verify the output contains valid mutation JSON
    cmd.forge_fuse().args(["test", "--mutate", "src/Counter.sol", "--mutation-jobs", "1", "--json"]).assert_success().stdout_eq(str![[r#"
...
{"summary":{"total":7,"killed":4,"survived":1,"invalid":2,"skipped":0,"mutation_score":80.0,"duration_secs":[..]},"survived_mutants":{"src/Counter.sol":[{"line":13,"column":9,"original":"number++","mutant":"++number"}]}}

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
  Survived - Mutant survived: tests did not catch this mutation (potential gap)
  Killed - Mutant killed: tests caught this mutation (good coverage)
  Invalid - Mutant invalid: mutation caused compilation error
  Skipped - Mutant skipped: redundant mutation on same expression

Mutation Score: 80.0% (8/10 mutants killed); [ELAPSED]

────────────────────────────────────────────────────────────
⚠ SURVIVED MUTANTS (test suite gaps)
────────────────────────────────────────────────────────────
These mutations were NOT caught by your tests.
Each represents a potential bug that your tests would miss.
...
     return a + b;
     Mutation:
       - a + b
       + a | b
...
     return a + b;
     Mutation:
       - a + b
       + a ^ b
...
────────────────────────────────────────────────────────────
✓ 8 mutants killed (tests caught these mutations)

────────────────────────────────────────────────────────────
ℹ 1 invalid mutants (compilation failures - expected for some mutations)

════════════════════════════════════════════════════════════

"#]]);
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
│ Survived ┆ 1         ┆ 1.6%       │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 51        ┆ 82.3%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 9         ┆ 14.5%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 1         ┆ 1.6%       │
╰──────────┴───────────┴────────────╯
...
Mutation Score: 98.1% (51/52 mutants killed); [ELAPSED]
...
"#]]);
});

// Test that Solidity library code (library keyword) is properly mutated
forgetest_init!(mutation_testing_library_code, |prj, cmd| {
    // A library with internal functions (common pattern like MathLib)
    prj.add_source(
        "MathLib.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library MathLib {
    function mulDivDown(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y) / d;
    }

    function mulDivUp(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y + (d - 1)) / d;
    }

    function min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }
}
"#,
    );

    // A contract that uses the library
    prj.add_source(
        "Calculator.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./MathLib.sol";

contract Calculator {
    using MathLib for uint256;

    function calculateShare(uint256 amount, uint256 totalSupply, uint256 totalAssets) public pure returns (uint256) {
        if (totalSupply == 0) return amount;
        return amount.mulDivDown(totalSupply, totalAssets);
    }

    function calculateShareRoundUp(uint256 amount, uint256 totalSupply, uint256 totalAssets) public pure returns (uint256) {
        if (totalSupply == 0) return amount;
        return amount.mulDivUp(totalSupply, totalAssets);
    }
}
"#,
    );

    prj.add_test(
        "MathLib.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/MathLib.sol";
import "../src/Calculator.sol";

contract MathLibTest is Test {
    Calculator public calc;

    function setUp() public {
        calc = new Calculator();
    }

    // Tests for mulDivDown
    function test_MulDivDown() public pure {
        // 100 * 50 / 200 = 25
        assertEq(MathLib.mulDivDown(100, 50, 200), 25);
    }

    function test_MulDivDownRoundsDown() public pure {
        // 100 * 3 / 7 = 42.857... rounds to 42
        assertEq(MathLib.mulDivDown(100, 3, 7), 42);
    }

    // Tests for mulDivUp  
    function test_MulDivUp() public pure {
        // 100 * 50 / 200 = 25 (no rounding needed)
        assertEq(MathLib.mulDivUp(100, 50, 200), 25);
    }

    function test_MulDivUpRoundsUp() public pure {
        // 100 * 3 / 7 = 42.857... rounds to 43
        assertEq(MathLib.mulDivUp(100, 3, 7), 43);
    }

    // Tests for min
    function test_MinReturnsSmaller() public pure {
        assertEq(MathLib.min(5, 10), 5);
        assertEq(MathLib.min(10, 5), 5);
    }

    function test_MinWithEqual() public pure {
        assertEq(MathLib.min(7, 7), 7);
    }

    // Tests via Calculator contract
    function test_CalculatorShare() public view {
        // 1000 * 500 / 2000 = 250
        assertEq(calc.calculateShare(1000, 500, 2000), 250);
    }
}
"#,
    );

    // Run mutation testing specifically on the library file
    cmd.args(["test", "--mutate", "src/MathLib.sol", "--mutation-jobs", "1"]);
    
    // Library code should generate mutations for:
    // - Binary operators in mulDivDown: x * y, (x * y) / d
    // - Binary operators in mulDivUp: x * y, d - 1, (x * y + (d - 1)) / d
    // - Comparison in min: a < b
    // Expect multiple mutations to be generated and tested
    let output = cmd.assert_success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    
    // Verify mutations were generated (not 0 mutants)
    assert!(!stdout.contains("No mutants generated"), 
        "Library code should generate mutants, but got: {}", stdout);
    
    // Verify mutation testing ran and produced results
    assert!(stdout.contains("MUTATION TESTING RESULTS"), 
        "Should show mutation results for library code");
    
    // Verify we got a reasonable mutation score (library functions should be testable)
    assert!(stdout.contains("Mutation Score:"), 
        "Should calculate mutation score for library");
});
