pragma solidity ^0.8.0;

import "./console.sol";

contract ConsoleLogs {
    function test_log() public {
		console.log(0x1111111111111111111111111111111111111111);
		console.log("Hi");
		console.log("Hi", "Hi");
		console.log(1337);
		console.log(1337, 1245);
		console.log("Hi", 1337);
    }

	function test_log_types() public {
		console.logString("String");
		console.logInt(1337);
		console.logInt(-20);
		console.logUint(1245);
		console.logBool(true);
	}
}
