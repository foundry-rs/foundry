use foundry_test_utils::{
    forgetest,
    util::OutputExt,
};

forgetest!(instrumented_basic, |prj, cmd| {
    prj.insert_ds_test();
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
    );

    prj.add_source(
        "CounterTest.sol",
        r#"
import "./test.sol";
import {Counter} from "./Counter.sol";

contract CounterTest is DSTest {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }
}
    "#,
    );

    // Assert coverage with --instrument-source.
    let output = cmd.arg("coverage")
        .arg("--instrument-source")
        .arg("--no-match-coverage")
        .arg("test.sol")
        .assert_success();
    
    let stdout = output.get_output().stdout_lossy();
    assert!(stdout.contains("src/Counter.sol     | 100.00% (4/4)   | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2)"));
    assert!(stdout.contains("src/CounterTest.sol | 100.00% (6/6)   | 100.00% (4/4) | 100.00% (0/0) | 100.00% (2/2)"));
    assert!(stdout.contains("Total               | 100.00% (10/10) | 100.00% (6/6) | 100.00% (0/0) | 100.00% (4/4)"));
});

forgetest!(instrumented_stack_too_deep, |prj, cmd| {
    prj.insert_ds_test();
    // A contract that would normally fail with "Stack Too Deep" when instrumented with legacy method
    // (if legacy method added many vars, but legacy doesn't really add vars, it just has poor source maps).
    // Actually, "Stack Too Deep" is solved by source instrumentation because it doesn't rely on source maps
    // which are often broken by viaIR or complex code.
    prj.add_source(
        "Large.sol",
        r#"
contract Large {
    function manyVars(uint256 a) public pure returns (uint256) {
        uint256 x1 = a + 1;
        uint256 x2 = a + 2;
        uint256 x3 = a + 3;
        uint256 x4 = a + 4;
        uint256 x5 = a + 5;
        uint256 x6 = a + 6;
        uint256 x7 = a + 7;
        uint256 x8 = a + 8;
        uint256 x9 = a + 9;
        uint256 x10 = a + 10;
        return x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10;
    }
}
    "#,
    );

    prj.add_source(
        "LargeTest.sol",
        r#"
import "./test.sol";
import {Large} from "./Large.sol";

contract LargeTest is DSTest {
    Large public large;

    function setUp() public {
        large = new Large();
    }

    function test_ManyVars() public {
        large.manyVars(1);
    }
}
    "#,
    );

    // Assert coverage with --instrument-source works for this contract.
    cmd.arg("coverage").args(["--instrument-source", "--ir-minimum"]).assert_success();
});
