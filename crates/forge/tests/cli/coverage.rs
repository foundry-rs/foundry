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
    prj.inner()
        .add_source(
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

    prj.inner()
        .add_source(
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
    assert!(lcov_data.lines().any(|line| re.captures(line).map_or(false, |caps| caps
        .get(1)
        .unwrap()
        .as_str()
        .parse::<i32>()
        .unwrap() >
        0)));
});
