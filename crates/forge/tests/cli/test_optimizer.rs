//! Tests for the `forge test` with preprocessed cache.

// Test cache is invalidated when `forge build` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_build, |prj, cmd| {
    prj.update_config(|config| {
        config.optimize_tests = true;
    });
    // All files are built with optimized tests.
    cmd.args(["build"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.optimize_tests = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);
});

// Test cache is invalidated when `forge test` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_test, |prj, cmd| {
    prj.update_config(|config| {
        config.optimize_tests = true;
    });
    // All files are built with optimized tests.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 20 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.optimize_tests = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 20 files with [..]
...

"#]]);
});

// Counter contract without interface instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_no_interface, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    )
    .unwrap();

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    )
    .unwrap();
    // All 20 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 20 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change Counter implementation to fail single test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 1;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and only one test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 2 != 1] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// Counter contract with interface instantiated in CounterTest
//
// ├── src
// │ ├── Counter.sol
// │ └── interface
// │     └── CounterIf.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_interface, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    )
    .unwrap();
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    )
    .unwrap();
    // All 21 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change only CounterIf interface.
    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function anotherNumber() external returns (uint256);

    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    )
    .unwrap();
    // All 3 files (interface, implementation and test) are compiled.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 3 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);
});

// - Counter contract instantiated in CounterMock
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     ├── Counter.t.sol
//     └── mock
//         └── CounterMock.sol
forgetest_init!(preprocess_mock_without_inheritance, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    )
    .unwrap();

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
        counter.setNumber(newNumber);
    }

    function increment() public {
        counter.increment();
    }

    function number() public returns (uint256) {
        return counter.number();
    }
}
    "#,
    )
    .unwrap();
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    )
    .unwrap();
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change CounterMock contract implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
    }

    function increment() public {
    }

    function number() public returns (uint256) {
        return 1;
    }
}
    "#,
    )
    .unwrap();
    // Assert that mock and test files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_with_inheritance, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    )
    .unwrap();

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    )
    .unwrap();
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    )
    .unwrap();
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public virtual {
        number = 12345;
    }

    function increment() public virtual {
        number++;
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert Counter source contract and CounterTest test contract (as it imports mock) are
    // compiled and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change mock implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
    function setNumber(uint256 newNumber) public override {
        number = newNumber;
    }

    function increment() public override {
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Assert that CounterMock and CounterTest files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_to_non_mock, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    )
    .unwrap();

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    )
    .unwrap();
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    )
    .unwrap();
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Change mock implementation to fail tests, no inherit from Counter.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    uint256 public number;
    function setNumber(uint256 newNumber) public {
        number = 1234;
    }

    function increment() public {
        number = 5678;
    }
}
    "#,
    )
    .unwrap();
    // Assert that CounterMock and CounterTest files are compiled and tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 5678 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 1234 != 1] test_SetNumber() (gas: [..])
...

"#]]);
});

// ├── src
// │ ├── CounterA.sol
// │ ├── Counter.sol
// │ └── v1
// │     └── Counter.sol
// └── test
// └── Counter.t.sol
forgetest_init!(preprocess_multiple_contracts_with_constructors, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.optimize_tests = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
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
    )
    .unwrap();
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";
import "src/CounterA.sol";
import {Counter as CounterV1} from "src/v1/Counter.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = new Counter();
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_Increment_In_Counter_V1() public {
        CounterV1 counter = new CounterV1(1234);
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_A() public {
        CounterA counter = new CounterA(1234, address(this));
        counter.increment();
        assertEq(counter.number(), 1235);
    }
}
    "#,
    )
    .unwrap();
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change v1/Counter to fail test.
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    )
    .unwrap();
    // Only v1/Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterA to fail test.
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    )
    .unwrap();
    // Only CounterA should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change Counter to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    )
    .unwrap();
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12345 != 1] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);
});
