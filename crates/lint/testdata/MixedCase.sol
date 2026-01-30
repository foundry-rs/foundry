//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC20 {
    function decimals() external view returns(uint8);
}

contract MixedCaseTest {
    uint256 variableMixedCase;
    uint256 _variableMixedCase;
    uint256 variablemixedcase;

    uint256 Variablemixedcase; //~NOTE: mutable variables should use mixedCase
    uint256 VARIABLE_MIXED_CASE; //~NOTE: mutable variables should use mixedCase
    uint256 VariableMixedCase; //~NOTE: mutable variables should use mixedCase

    function foo() public {
        uint256 testVal;
        uint256 testVal123;

        uint256 testVAL; //~NOTE: mutable variables should use mixedCase
        uint256 TestVal; //~NOTE: mutable variables should use mixedCase
        uint256 TESTVAL; //~NOTE: mutable variables should use mixedCase
    }

    function functionMixedCase() public {}
    function _functionMixedCase() internal {}
    function functionmixedcase() public {}

    function Functionmixedcase() public {} //~NOTE: function names should use mixedCase
    function FUNCTION_MIXED_CASE() public {} //~NOTE: function names should use mixedCase
    function FunctionMixedCase() public {} //~NOTE: function names should use mixedCase
    function function_mixed_case() public {} //~NOTE: function names should use mixedCase

    // mixedCase checks are disabled for functions that starting with:
    // `test`, `invariant_`, and `statefulFuzz`
    function test_MixedCase_Disabled() public {}
    function test_mixedcase_disabled() public {}
    function testMixedCaseDisabled() public {}
    function testmixedcasedisabled() public {}

    function testFuzz_MixedCase_Disabled() public {}
    function testFuzz_mixedcase_disabled() public {}
    function testFuzzMixedCaseDisabled() public {}
    function testfuzzmixedcasedisabled() public {}

    function testRevert_MixedCase_Disabled() public {}
    function testRevert_mixedcase_disabled() public {}
    function testRevertMixedCaseDisabled() public {}
    function testrevertmixedcasedisabled() public {}

    function invariant_MixedCase_Disabled() public {}
    function invariant_mixedcase_disabled() public {}
    function invariant_MixedCaseDisabled() public {}
    function invariant_mixedcasedisabled() public {}

    function invariantBalance_MixedCase_Enabled() public {} //~NOTE: function names should use mixedCase
    function invariantbalance_mixedcase_enabled() public {} //~NOTE: function names should use mixedCase
    function invariantBalanceMixedCaseEnabled() public {}
    function invariantbalancemixedcaseenabled() public {}

    function statefulFuzz_MixedCase_Disabled() public {}
    function statefulFuzz_mixedcase_disabled() public {}
    function statefulFuzzMixedCaseDisabled() public {}
    function statefulFuzzmixedcasedisabled() public {}

    // ERC is, by default, an allowed infix
    function rescueERC6909(address token, address to, uint256 tokenId, uint256 amount) public {}
    function ERC20DoSomething() public {}
    function ERC20_DoSomething() public {} // invalid because of the underscore
    //~^NOTE: function names should use mixedCase

    // SCREAMING_SNAKE_CASE is allowed for functions that are most likely constant getters
    function MAX_NUMBER() external view returns (uint256) {}
    function CUSTOM_TYPE_RETURN() external view returns (IERC20) {}
    function HAS_PARAMS(address addr) external view returns (uint256) {} //~NOTE: function names should use mixedCase
    function HAS_NO_RETURN() external view {} //~NOTE: function names should use mixedCase
    function HAS_MORE_THAN_ONE_RETURN() external view returns (uint256, uint256) {} //~NOTE: function names should use mixedCase
    function NOT_ELEMENTARY_RETURN() external view returns (uint256[] memory) {} //~NOTE: function names should use mixedCase
}
