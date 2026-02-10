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
...
     return a + b;
     Mutation:
       - a + b
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

// Seeded gap test: access control bypass detection.
// A weak test suite that only tests the happy path (owner calls) but never
// tests that non-owners are rejected. The require(msg.sender == owner)
// mutation to require(true) should SURVIVE, proving the gap.
forgetest_init!(mutation_detects_access_control_gap, |prj, cmd| {
    prj.add_source(
        "Owned.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Owned {
    address public owner;

    constructor() {
        owner = msg.sender;
    }

    function changeOwner(address newOwner) public {
        require(msg.sender == owner, "Not owner");
        owner = newOwner;
    }
}
"#,
    );

    // Weak test: only tests that the owner CAN call changeOwner.
    // Never tests that a non-owner is rejected.
    prj.add_test(
        "Owned.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Owned.sol";

contract OwnedTest is Test {
    Owned public owned;

    function setUp() public {
        owned = new Owned();
    }

    function test_OwnerCanChangeOwner() public {
        address newOwner = address(0x1234);
        owned.changeOwner(newOwner);
        assertEq(owned.owner(), newOwner);
    }
}
"#,
    );

    // The require(msg.sender == owner) -> require(true) mutation should survive
    // because we never test that non-owners are rejected.
    cmd.args(["test", "--mutate", "src/Owned.sol", "--mutation-jobs", "1", "--json"]);
    let output = cmd.assert_success().get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let survived = json["summary"]["survived"].as_u64().unwrap();
    assert!(survived > 0, "Expected surviving mutants (access control gap not detected)");

    // Verify that a require-related mutation survived
    let survived_mutants = json["survived_mutants"]["src/Owned.sol"].as_array().unwrap();
    let has_require_survivor = survived_mutants.iter().any(|m| {
        let mutant_text = m["mutant"].as_str().unwrap_or("");
        mutant_text.contains("true") || mutant_text.contains("false") || mutant_text.contains("!=")
    });
    assert!(
        has_require_survivor,
        "Expected a require/condition mutation to survive (access control gap), but survivors were: {:?}",
        survived_mutants
    );
});

// Seeded gap test: boundary condition detection.
// A test suite that tests withdrawal with amounts well below the balance
// but never tests the exact boundary (amount == balance). The >= to >
// mutation should SURVIVE.
forgetest_init!(mutation_detects_boundary_condition_gap, |prj, cmd| {
    prj.add_source(
        "Balance.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Balance {
    mapping(address => uint256) public balances;

    function deposit() public payable {
        balances[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) public {
        require(balances[msg.sender] >= amount, "Insufficient");
        balances[msg.sender] -= amount;
        payable(msg.sender).transfer(amount);
    }
}
"#,
    );

    // Weak test: deposits 1 ether but only withdraws 0.5 ether.
    // Never tests withdrawing the exact balance (boundary).
    prj.add_test(
        "Balance.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/Balance.sol";

contract BalanceTest is Test {
    Balance public b;

    function setUp() public {
        b = new Balance();
    }

    function test_DepositAndPartialWithdraw() public {
        b.deposit{value: 1 ether}();
        assertEq(b.balances(address(this)), 1 ether);
        b.withdraw(0.5 ether);
        assertEq(b.balances(address(this)), 0.5 ether);
    }

    function test_WithdrawTooMuch() public {
        b.deposit{value: 1 ether}();
        vm.expectRevert("Insufficient");
        b.withdraw(2 ether);
    }

    receive() external payable {}
}
"#,
    );

    // The >= to > mutation should survive because we never test amount == balance
    cmd.args(["test", "--mutate", "src/Balance.sol", "--mutation-jobs", "1", "--json"]);
    let output = cmd.assert_success().get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let survived = json["summary"]["survived"].as_u64().unwrap();
    assert!(survived > 0, "Expected surviving mutants (boundary condition gap not detected)");

    let score = json["summary"]["mutation_score"].as_f64().unwrap();
    assert!(score < 100.0, "Score should be < 100% with boundary gap, got {score}");
});

// Seeded gap test: arithmetic operator detection.
// A test suite that only tests one input, so swapping + for - is not caught.
forgetest_init!(mutation_detects_arithmetic_gap, |prj, cmd| {
    prj.add_source(
        "Math.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Math {
    function multiply(uint256 a, uint256 b) public pure returns (uint256) {
        return a * b;
    }
}
"#,
    );

    // Weak test: only tests multiply(0, 5) which returns 0 for both * and +
    // (0 * 5 == 0, 0 + 5 != 0 but 0 * anything == 0 regardless of operator for one operand)
    // Actually let's use multiply(1, 1) where 1*1==1 and 1+1==2, so + would be caught.
    // Use multiply(2, 2) where 2*2==4 but 2+2==4 too! So + survives.
    prj.add_test(
        "Math.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Math.sol";

contract MathTest {
    Math public m;

    function setUp() public {
        m = new Math();
    }

    function test_MultiplySymmetric() public {
        // 2*2 == 4, but also 2+2 == 4, so the + mutation survives!
        assert(m.multiply(2, 2) == 4);
    }
}
"#,
    );

    cmd.args(["test", "--mutate", "src/Math.sol", "--mutation-jobs", "1", "--json"]);
    let output = cmd.assert_success().get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    let survived = json["summary"]["survived"].as_u64().unwrap();
    assert!(survived > 0, "Expected surviving mutants (arithmetic gap not detected)");

    // The * -> + mutation should survive since 2*2 == 2+2
    let score = json["summary"]["mutation_score"].as_f64().unwrap();
    assert!(score < 100.0, "Score should be < 100% with weak arithmetic test, got {score}");
});
