use regex::Regex;

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

    let lcov_info = prj.root().join("lcov.info");
    cmd.arg("coverage").args([
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        lcov_info.to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
    assert!(lcov_info.exists());

    let lcov_data = std::fs::read_to_string(lcov_info).unwrap();
    // AContract.init must be hit at least once
    let re = Regex::new(r"FNDA:(\d+),AContract\.init").unwrap();
    let valid_line = |line| {
        re.captures(line)
            .map_or(false, |caps| caps.get(1).unwrap().as_str().parse::<i32>().unwrap() > 0)
    };
    assert!(lcov_data.lines().any(valid_line), "{lcov_data}");
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

    let lcov_info = prj.root().join("lcov.info");
    cmd.arg("coverage").args([
        "--no-match-coverage".to_string(),
        "AContract".to_string(), // Filter out `AContract`
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        lcov_info.to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
    assert!(lcov_info.exists());

    let lcov_data = std::fs::read_to_string(lcov_info).unwrap();
    // BContract.init must be hit at least once
    let re = Regex::new(r"FNDA:(\d+),BContract\.init").unwrap();
    let valid_line = |line| {
        re.captures(line)
            .map_or(false, |caps| caps.get(1).unwrap().as_str().parse::<i32>().unwrap() > 0)
    };
    assert!(lcov_data.lines().any(valid_line), "{lcov_data}");

    // AContract.init must not be hit
    let re = Regex::new(r"FNDA:(\d+),AContract\.init").unwrap();
    let valid_line = |line| {
        re.captures(line)
            .map_or(false, |caps| caps.get(1).unwrap().as_str().parse::<i32>().unwrap() > 0)
    };
    assert!(!lcov_data.lines().any(valid_line), "{lcov_data}");
});
