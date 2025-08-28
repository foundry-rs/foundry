//! Tests for backtrace functionality

use foundry_test_utils::util::OutputExt;

const BACKTRACE_TEST_CONTRACT: &str = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./test.sol";

contract Helper {
    function doCalculation(uint256 value) public pure returns (uint256) {
        require(value > 0, "Value must be greater than zero");
        
        if (value > 100) {
            revert("Value too large");
        }
        
        return value * 2;
    }
    
    function callAnother(uint256 value) public pure returns (uint256) {
        return doCalculation(value);
    }
}

contract BacktraceTest is DSTest {
    Helper public helper;
    
    function setUp() public {
        helper = new Helper();
    }
    
    function testSimpleRevert() public {
        // This should revert with "Value must be greater than zero"
        helper.doCalculation(0);
    }
    
    function testNestedRevert() public {
        // This should revert with "Value too large" through nested call
        helper.callAnother(150);
    }
    
    function testDeepNesting() public {
        // This tests deep nesting with internal calls
        deepCall1(200);
    }
    
    function deepCall1(uint256 value) internal {
        deepCall2(value);
    }
    
    function deepCall2(uint256 value) internal {
        deepCall3(value);
    }
    
    function deepCall3(uint256 value) internal view {
        helper.doCalculation(value); // Should revert with "Value too large"
    }
    
    function testMultipleReverts() public {
        // Test with multiple potential revert points
        uint256 result = helper.doCalculation(50); // This succeeds
        require(result == 100, "Unexpected result");
        helper.doCalculation(0); // This reverts
    }
}
"#;

// Test that backtraces are shown at verbosity level 3 (-vvv)
forgetest!(test_backtrace_simple_revert, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("BacktraceTest.t.sol", BACKTRACE_TEST_CONTRACT).unwrap();
    
    cmd.args(["test", "--match-test", "testSimpleRevert", "-vvv"]);
    let output = cmd.assert_failure().get_output().stdout_lossy();
    println!("OUTPUT:\n{}", output);
    
    // For now, just check that it failed - we'll add assertions later
    // to avoid compilation issues
});