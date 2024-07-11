use foundry_test_utils::str;

forgetest!(basic_coverage, |_prj, cmd| {
    cmd.args(["coverage"]);
    cmd.assert_success();
});

forgetest!(report_file_coverage, |prj, cmd| {
    cmd.arg("coverage").args([
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        prj.root().join("lcov.info").to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
});

forgetest!(test_setup_coverage, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    )
    .unwrap();

    // Assert 100% coverage (init function coverage called in setUp is accounted).
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
| Total             | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |

"#]]);
});

forgetest!(test_no_match_coverage, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "BContract.sol",
        r#"
contract BContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "BContractTest.sol",
        r#"
import "./test.sol";
import {BContract} from "./BContract.sol";

contract BContractTest is DSTest {
    BContract a;

    function setUp() public {
        a = new BContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    )
    .unwrap();

    // Assert AContract is not included in report.
    cmd.arg("coverage")
        .args([
            "--no-match-coverage".to_string(),
            "AContract".to_string(), // Filter out `AContract`
        ])
        .assert_success()
        .stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/BContract.sol | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
| Total             | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |

"#]]);
});

forgetest!(test_assert_require_coverage, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function checkA() external pure returns (bool) {
        assert(10 > 2);
        require(10 > 2, "true");
        return true;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    function testA() external {
        AContract a = new AContract();
        bool result = a.checkA();
        assertTrue(result);
    }
}
    "#,
    )
    .unwrap();

    // Assert 100% coverage (assert and require properly covered).
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (3/3) | 100.00% (3/3) | 100.00% (0/0) | 100.00% (1/1) |
| Total             | 100.00% (3/3) | 100.00% (3/3) | 100.00% (0/0) | 100.00% (1/1) |

"#]]);
});
