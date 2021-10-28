pragma solidity ^0.8.0;

import "./DsTest.sol";
import "./console.sol";

contract ConsoleLog is DSTest {
    function test_console_log() public {
        uint256 number = 45;
        string memory word = "greet";
        address addr = address(this);
        console.log("the contract will now %s you", word);
        console.log("The number is %s", number);
        console.log("Address: %s just performed a %s", addr, word);
    }
}
