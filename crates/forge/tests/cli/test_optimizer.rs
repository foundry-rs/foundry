//! Tests for the `forge test` with preprocessed cache.

// Test cache is invalidated when `forge build` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_build, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["build"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
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
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...

"#]]);
});

// Test cache is invalidated when `forge test` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_test, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
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
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
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
        config.dynamic_test_linking = true;
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
Compiling 21 files with [..]
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
        config.dynamic_test_linking = true;
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
        counter = Counter(address(new Counter()));
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
Compiling 22 files with [..]
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
        config.dynamic_test_linking = true;
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
Compiling 22 files with [..]
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
        config.dynamic_test_linking = true;
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
Compiling 22 files with [..]
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
        config.dynamic_test_linking = true;
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
Compiling 22 files with [..]
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
// │ ├── CounterB.sol
// │ ├── Counter.sol
// │ └── v1
// │     └── Counter.sol
// └── test
// └── Counter.t.sol
forgetest_init!(preprocess_multiple_contracts_with_constructors, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
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
    // Contract with constructor args without name.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 1;
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
import "src/CounterB.sol";
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

    function test_Increment_In_Counter_A_with_named_args() public {
        CounterA counter = new CounterA({_newNumber: 1234, _owner: address(this)});
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_B() public {
        CounterB counter = new CounterB(1234);
        counter.increment();
        assertEq(counter.number(), 2);
    }
}
    "#,
    )
    .unwrap();
    // 22 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 24 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
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
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
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
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 100;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();
    // Only CounterB should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
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
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);
});

// Test preprocessing contracts with payable constructor, value and salt named args.
forgetest_init!(preprocess_contracts_with_payable_constructor_and_salt, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

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
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

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
import {Counter} from "src/Counter.sol";
import {CounterWithSalt} from "src/CounterWithSalt.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = Counter(address(new Counter{value: 111}(1)));
        counter.increment();
        assertEq(counter.number(), 112);
    }

    function test_Increment_In_Counter_With_Salt() public {
        CounterWithSalt counter = new CounterWithSalt{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1);
        assertEq(address(counter), 0x223e63BE3BF01DD04f852d70f1bE217017055f49);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

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
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract with salt to fail test too.
    prj.add_source(
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

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
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 0x11acEfcD29A1BA964A05C0E7F3901054BEfb17c0 != 0x223e63BE3BF01DD04f852d70f1bE217017055f49] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);
});

// Counter contract with constructor reverts and emitted events.
forgetest_init!(preprocess_contract_with_require_and_emit, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert");
        emit CounterCreated(10);
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
    function test_assert_constructor_revert() public {
        vm.expectRevert("ctor revert");
        new Counter(1);
    }

    function test_assert_constructor_emit() public {
        vm.expectEmit(true, true, true, true);
        emit Counter.CounterCreated(10);

        new Counter(11);
    }
}
    "#,
    )
    .unwrap();
    // All 20 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter implementation to revert with different message.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert update");
        emit CounterCreated(10);
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: Error != expected error: ctor revert update != ctor revert] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation and don't revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(10);
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation to emit different event.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(100);
    }
}
    "#,
    )
    .unwrap();
    // Assert that only 1 file is compiled (Counter source contract) and emit test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10312>
forgetest_init!(preprocess_contract_with_constructor_args_struct, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    struct ConstructorArgs {
        uint256 _number;
    }

    constructor(uint256 no) {
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
    function test_assert_constructor_revert() public {
        Counter counter = new Counter(1);
    }
}
    "#,
    )
    .unwrap();
    // All 20 files should properly compile.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
});

// Test preprocessed contracts with decode internal fns.
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(preprocess_contract_with_decode_internal, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        create_counter(0);
    }

    function test_Increment() public {
        create_counter(0);
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function create_counter(uint256 number) internal {
        counter = new Counter();
        counter.setNumber(number);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--decode-internal", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_Increment() ([GAS])
Traces:
  [..] CounterTest::test_Increment()
    ├─ [0] VM::deployCode("src/Counter.sol:Counter")
    │   ├─ [96345] → new Counter@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   │   └─ ← [Return] 481 bytes of code
    │   └─ ← [Return] Counter: [0x2e234DAe75C793f67A35089C9d99245E1C58470b]
    ├─ [..] Counter::setNumber(0)
    │   └─ ← [Stop]
    ├─ [..] Counter::increment()
    │   └─ ← [Stop]
    ├─ [..] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─ [..] StdAssertions::assertEq(1, 1)
    │   └─ ← 
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10492>
// Preprocess test contracts with try constructor statements.
forgetest_init!(preprocess_contract_with_try_ctor_stmt, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 number;
}
    "#,
    )
    .unwrap();
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    )
    .unwrap();
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    )
    .unwrap();

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterA} from "../src/CounterA.sol";
import {CounterB} from "../src/CounterB.sol";
import {CounterC} from "../src/CounterC.sol";

contract CounterTest is Test {
    function test_try_counterA_creation() public {
        try new CounterA() {} catch {
            revert();
        }
    }

    function test_try_counterB_creation() public {
        try new CounterB(1) {} catch {
            revert();
        }
    }

    function test_try_counterB_creation_with_salt() public {
        try new CounterB{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1) {} catch {
            revert();
        }
    }

    function test_try_counterC_creation() public {
        try new CounterC(2) {
            new CounterC(1);
        } catch {
            revert();
        }
    }
}
    "#,
    )
    .unwrap();
    // All 23 files should properly compile, tests pass.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[PASS] test_try_counterB_creation() (gas: [..])
[PASS] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 11, "ctor failure");
        number = a;
    }
}
    "#,
    )
    .unwrap();
    // Only CounterB should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 1, "ctor failure");
        number = a;
    }
}
    "#,
    )
    .unwrap();
    // Only CounterC should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: ctor failure] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 2, "ctor failure");
        number = a;
    }
}
    "#,
    )
    .unwrap();
    // Only CounterC should compile and revert.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterC_creation() (gas: [..])
...

"#]]);
});
