pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

contract ReturnsNothing {
    function func() public pure {}
}

contract ReturnsString {
    function func() public pure returns (string memory) {
        return "string";
    }
}

contract ReturnsUint {
    function func() public pure returns (uint256) {
        return 1;
    }
}

contract ConflictingSignaturesTest is DSTest {
    ReturnsNothing retsNothing;
    ReturnsString retsString;
    ReturnsUint retsUint;

    function setUp() public {
        retsNothing = new ReturnsNothing();
        retsString = new ReturnsString();
        retsUint = new ReturnsUint();
    }

    /// Tests that traces are decoded properly when multiple
    /// functions have the same 4byte signature, but different
    /// return values.
    function testTraceWithConflictingSignatures() public {
        retsNothing.func();
        retsString.func();
        retsUint.func();
    }
}
