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

    uint256 Variablemixedcase; //~NOTE: mutable variable name is not `mixedCase`
    uint256 VARIABLE_MIXED_CASE; //~NOTE: mutable variable name is not `mixedCase`
    uint256 VariableMixedCase; //~NOTE: mutable variable name is not `mixedCase`

    function foo() public {
        uint256 testVal;
        uint256 testVal123;

        uint256 testVAL; //~NOTE: mutable variable name is not `mixedCase`
        uint256 TestVal; //~NOTE: mutable variable name is not `mixedCase`
        uint256 TESTVAL; //~NOTE: mutable variable name is not `mixedCase`
    }

    function functionMixedCase() public {}
    function _functionMixedCase() internal {}
    function functionmixedcase() public {}

    function Functionmixedcase() public {} //~NOTE: function name is not `mixedCase`
    function FUNCTION_MIXED_CASE() public {} //~NOTE: function name is not `mixedCase`
    function FunctionMixedCase() public {} //~NOTE: function name is not `mixedCase`
    function function_mixed_case() public {} //~NOTE: function name is not `mixedCase`

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

    function invariantBalance_MixedCase_Enabled() public {} //~NOTE: function name is not `mixedCase`
    function invariantbalance_mixedcase_enabled() public {} //~NOTE: function name is not `mixedCase`
    function invariantBalanceMixedCaseEnabled() public {}
    function invariantbalancemixedcaseenabled() public {}

    function statefulFuzz_MixedCase_Disabled() public {}
    function statefulFuzz_mixedcase_disabled() public {}
    function statefulFuzzMixedCaseDisabled() public {}
    function statefulFuzzmixedcasedisabled() public {}

    // ERC is, by default, an allowed infix
    function rescueERC6909(address token, address to, uint256 tokenId, uint256 amount) public {}
    function ERC20DoSomething() public {}
    function _rescueERC20() public {} // a preserved leading underscore keeps the exception
    function rescueERC20_() public {} // a preserved trailing underscore keeps the exception
    function __rescueERC20() public {} //~NOTE: function name is not `mixedCase`
    function rescueERC20__() public {} //~NOTE: function name is not `mixedCase`
    function ERC20_DoSomething() public {} // invalid because of the underscore
    //~^NOTE: function name is not `mixedCase`

    // Common abbreviations are allowed by default: ID, URL, URI, API, JSON, XML, HTML, HTTP, HTTPS
    uint256 marketID;
    uint256 userID;
    uint256 optionID;
    uint256 apiURL;
    uint256 baseURL;
    function parseJSON() public {}
    function fetchAPIData() public {}
    function processHTML() public {}
    function sendHTTPRequest() public {}
    function handleHTTPSConnection() public {}
    function getXMLData() public {}

    // SCREAMING_SNAKE_CASE is allowed for functions that are most likely constant getters
    function MAX_NUMBER() external view returns (uint256) {}
    function CUSTOM_TYPE_RETURN() external view returns (IERC20) {}
    function HAS_PARAMS(address addr) external view returns (uint256) {} //~NOTE: function name is not `mixedCase`
    function HAS_NO_RETURN() external view {} //~NOTE: function name is not `mixedCase`
    function HAS_MORE_THAN_ONE_RETURN() external view returns (uint256, uint256) {} //~NOTE: function name is not `mixedCase`
    function NOT_ELEMENTARY_RETURN() external view returns (uint256[] memory) {} //~NOTE: function name is not `mixedCase`
}

contract PublicConstantGetters {
    bytes32 private separator;

    // Public getters follow the same naming convention as external getters.
    function DOMAIN_SEPARATOR() public view returns (bytes32) { return separator; }
    function PUBLIC_CUSTOM_TYPE() public view returns (IERC20) { return IERC20(address(0)); }

    function PUBLIC_WITH_PARAM(uint256 value) public view returns (uint256) { return value; } //~NOTE: function name is not `mixedCase`
    function INTERNAL_GETTER() internal view returns (bytes32) { return separator; } //~NOTE: function name is not `mixedCase`
    function PUBLIC_MUTATOR() public returns (bytes32) { separator = bytes32(uint256(1)); return separator; } //~NOTE: function name is not `mixedCase`
}
