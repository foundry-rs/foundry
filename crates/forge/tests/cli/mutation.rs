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

Mutation Score: 80.0% (4/5 mutants killed)

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

Mutation Score: 80.0% (8/10 mutants killed)

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

Mutation Score: 80.0% (8/10 mutants killed)

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
