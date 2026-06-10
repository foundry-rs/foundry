// CLI integration tests for mutation testing

use foundry_test_utils::{str, util::OutputExt};
use std::fs;

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

forgetest_init!(mutation_testing_rejects_all_skipped_baseline, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;

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

import "forge-std/Test.sol";
import "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        vm.skip(true);
        counter = new Counter();
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }
}
"#,
    );

    let output = cmd.args(["test", "--mutate", "src/Counter.sol"]).assert_failure();
    let stderr = output.get_output().stderr_lossy();

    assert!(
        stderr.contains("Mutation testing requires at least one passing baseline test"),
        "unexpected stderr:\n{stderr}"
    );
});

forgetest_init!(mutation_testing_rejects_empty_mutate_path_selection, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;

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
    function test_Increment() public {
        Counter counter = new Counter();
        counter.increment();
        assert(counter.number() == 1);
    }
}
"#,
    );

    let output =
        cmd.args(["test", "--mutate", "--mutate-path", "src/Missing*.sol"]).assert_failure();
    let stderr = output.get_output().stderr_lossy();

    assert!(
        stderr.contains("no source matched --mutate-path pattern"),
        "unexpected stderr:\n{stderr}"
    );
});

forgetest_init!(mutation_testing_rejects_empty_mutate_contract_selection, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;

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
    function test_Increment() public {
        Counter counter = new Counter();
        counter.increment();
        assert(counter.number() == 1);
    }
}
"#,
    );

    let output = cmd.args(["test", "--mutate", "--mutate-contract", "Missing"]).assert_failure();
    let stderr = output.get_output().stderr_lossy();

    assert!(
        stderr.contains("no source matched --mutate-contract pattern"),
        "unexpected stderr:\n{stderr}"
    );
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

forgetest_init!(mutation_result_cache_invalidates_when_match_test_changes, |prj, _cmd| {
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

    function test_Weak() public view {
        calculator.add(1, 2);
    }

    function test_Strong() public view {
        assert(calculator.add(1, 2) == 3);
    }
}
"#,
    );

    let mut weak_cmd = prj.forge_command();
    let weak_stdout = weak_cmd
        .args([
            "test",
            "--mutate",
            "src/Calculator.sol",
            "--mutation-jobs",
            "1",
            "--match-test",
            "test_Weak",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let weak_summary = mutation_summary(&weak_stdout);

    let mut strong_cmd = prj.forge_command();
    let strong_stdout = strong_cmd
        .args([
            "test",
            "--mutate",
            "src/Calculator.sol",
            "--mutation-jobs",
            "1",
            "--match-test",
            "test_Strong",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let strong_summary = mutation_summary(&strong_stdout);

    assert_eq!(weak_summary["total"], strong_summary["total"]);
    assert!(
        strong_summary["killed"].as_u64().unwrap() > weak_summary["killed"].as_u64().unwrap(),
        "expected --match-test to invalidate cached mutation results: weak={weak_summary}, strong={strong_summary}",
    );
});

forgetest_init!(mutation_result_cache_invalidates_when_match_path_changes, |prj, _cmd| {
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
        "Weak.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Calculator.sol";

contract WeakTest {
    Calculator public calculator;

    function setUp() public {
        calculator = new Calculator();
    }

    function test_Weak() public view {
        calculator.add(1, 2);
    }
}
"#,
    );

    prj.add_test(
        "Strong.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Calculator.sol";

contract StrongTest {
    Calculator public calculator;

    function setUp() public {
        calculator = new Calculator();
    }

    function test_Strong() public view {
        assert(calculator.add(1, 2) == 3);
    }
}
"#,
    );

    let mut weak_cmd = prj.forge_command();
    let weak_stdout = weak_cmd
        .args([
            "test",
            "--mutate",
            "src/Calculator.sol",
            "--mutation-jobs",
            "1",
            "--match-path",
            "test/Weak.t.sol",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let weak_summary = mutation_summary(&weak_stdout);

    let mut strong_cmd = prj.forge_command();
    let strong_stdout = strong_cmd
        .args([
            "test",
            "--mutate",
            "src/Calculator.sol",
            "--mutation-jobs",
            "1",
            "--match-path",
            "test/Strong.t.sol",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let strong_summary = mutation_summary(&strong_stdout);

    assert_eq!(weak_summary["total"], strong_summary["total"]);
    assert!(
        strong_summary["killed"].as_u64().unwrap() > weak_summary["killed"].as_u64().unwrap(),
        "expected --match-path to invalidate cached mutation results: weak={weak_summary}, strong={strong_summary}",
    );
});

forgetest_init!(mutation_honors_match_path_at_compile_time, |prj, cmd| {
    prj.add_source(
        "Foo.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Foo {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_test(
        "FooSelected.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Foo.sol";

contract FooSelectedTest {
    Foo internal foo;

    function setUp() public {
        foo = new Foo();
    }

    function test_Add() public view {
        assert(foo.add(2, 3) == 5);
    }
}
"#,
    );

    prj.add_test(
        "FooBroken.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Foo.sol";

contract FooBrokenTest {
    function test_Broken() public pure {
        NonExistent x = NonExistent(0);
        x.doSomething();
    }
}
"#,
    );

    cmd.forge_fuse().args(["test", "--match-path", "test/FooSelected.t.sol"]).assert_success();

    cmd.forge_fuse().args([
        "test",
        "--mutate",
        "src/Foo.sol",
        "--match-path",
        "test/FooSelected.t.sol",
        "--mutation-jobs",
        "1",
        "--json",
    ]);

    let out = cmd.assert_success().get_output().stdout_lossy();
    let summary = mutation_summary(&out);

    let total = summary["total"].as_u64().unwrap_or(0);
    let invalid = summary["invalid"].as_u64().unwrap_or(u64::MAX);
    let killed = summary["killed"].as_u64().unwrap_or(0);
    let survived = summary["survived"].as_u64().unwrap_or(0);

    assert!(
        invalid < total,
        "filtered-out FooBroken.t.sol must not make every mutant Invalid; summary={summary}"
    );
    assert!(
        killed + survived >= 1,
        "expected at least one Killed/Survived mutant from arithmetic ops; summary={summary}"
    );
});

forgetest_init!(mutation_compiles_dynamic_linking_artifacts_for_selected_tests, |prj, cmd| {
    prj.add_source(
        "Target.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Target {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}
"#,
    );

    prj.add_test(
        "helpers/LinkedHelper.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library LinkedHelper {
    function adjust(uint256 value) public pure returns (uint256) {
        return value + 1;
    }
}
"#,
    );

    prj.add_test(
        "BaseLinked.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/Target.sol";
import "./helpers/LinkedHelper.sol";

contract BaseLinkedTest {
    Target internal target;
    uint256 internal helperValue;

    function setUp() public {
        target = new Target();
        helperValue = LinkedHelper.adjust(1);
    }
}
"#,
    );

    prj.add_test(
        "SelectedLinked.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./BaseLinked.t.sol";

contract SelectedLinkedTest is BaseLinkedTest {
    function test_AddsHelperValue() public view {
        assert(target.add(helperValue, 1) == 3);
    }
}
"#,
    );

    let out = cmd
        .args([
            "test",
            "--mutate",
            "src/Target.sol",
            "--match-contract",
            "SelectedLinkedTest",
            "--mutation-jobs",
            "1",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let summary = mutation_summary(&out);

    let total = summary["total"].as_u64().unwrap_or(0);
    let invalid = summary["invalid"].as_u64().unwrap_or(u64::MAX);
    let killed = summary["killed"].as_u64().unwrap_or(0);
    let survived = summary["survived"].as_u64().unwrap_or(0);

    assert!(total > 0, "expected mutation testing to generate mutants: summary={summary}");
    assert!(
        invalid < total,
        "dynamic-linking helper artifacts should compile inside mutant workspaces: summary={summary}"
    );
    assert!(
        killed + survived >= 1,
        "expected at least one Killed/Survived mutant from arithmetic ops; summary={summary}"
    );
});

forgetest_init!(mutation_workspace_copies_include_paths, |prj, cmd| {
    let include_dir = prj.root().join("include");
    fs::create_dir_all(&include_dir).unwrap();
    fs::write(
        include_dir.join("Shared.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library Shared {
    function value() internal pure returns (uint256) {
        return 1;
    }
}
"#,
    )
    .unwrap();

    prj.update_config(|config| {
        config.include_paths = vec![include_dir.clone()];
    });

    prj.add_source(
        "UsesShared.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "Shared.sol";

contract UsesShared {
    function value() public pure returns (uint256) {
        return Shared.value() + 1;
    }
}
"#,
    );

    prj.add_test(
        "UsesShared.t.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/UsesShared.sol";

contract UsesSharedTest {
    function test_Value() public {
        UsesShared usesShared = new UsesShared();
        assert(usesShared.value() == 2);
    }
}
"#,
    );

    let out = cmd
        .args(["test", "--mutate", "src/UsesShared.sol", "--mutation-jobs", "1", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let summary = mutation_summary(&out);
    let total = summary["total"].as_u64().unwrap_or(0);
    let invalid = summary["invalid"].as_u64().unwrap_or(u64::MAX);

    assert!(total > 0, "expected mutation testing to generate mutants: summary={summary}");
    assert!(
        invalid < total,
        "include_paths imports should compile inside mutant workspaces: summary={summary}"
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
│ Survived ┆ 3         ┆ 5.0%       │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Killed   ┆ 48        ┆ 80.0%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Invalid  ┆ 7         ┆ 11.7%      │
├╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ Skipped  ┆ 2         ┆ 3.3%       │
╰──────────┴───────────┴────────────╯
...
Mutation Score: 94.1% (48/51 mutants killed); [ELAPSED]
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
