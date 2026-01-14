use foundry_test_utils::{
    forgetest,
    util::OutputExt,
};

forgetest!(instrumented_complex_control_flow, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "Complex.sol",
        r#"
contract Complex {
    function nested(uint256 x) public pure returns (uint256) {
        if (x > 10) {
            if (x > 20) {
                return 1;
            } else {
                return 2;
            }
        } else {
            for (uint256 i = 0; i < x; i++) {
                if (i == 5) {
                    return 3;
                }
            }
        }
        return 4;
    }

    function loops(uint256 x) public pure returns (uint256) {
        uint256 y = 0;
        while (x > 0) {
            y++;
            x--;
            if (y > 100) break;
        }
        do {
            y++;
        } while (y < 10);
        return y;
    }
}
    "#,
    );

    prj.add_source(
        "ComplexTest.sol",
        r#"
import "./test.sol";
import {Complex} from "./Complex.sol";

contract ComplexTest is DSTest {
    Complex public complex;

    function setUp() public {
        complex = new Complex();
    }

    function test_Nested() public {
        complex.nested(5);
        complex.nested(15);
        complex.nested(25);
    }

    function test_Loops() public {
        complex.loops(5);
    }
}
    "#,
    );

    let output = cmd.arg("coverage")
        .arg("--instrument-source")
        .assert_success();
    
    let stdout = output.get_output().stdout_lossy();
    assert!(stdout.contains("src/Complex.sol"));
});
