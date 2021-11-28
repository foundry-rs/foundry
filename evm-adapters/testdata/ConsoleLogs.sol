pragma solidity ^0.8.0;

import "./console.sol";

contract ConsoleLogs {
    function test_log() public {
		console.log(0x1111111111111111111111111111111111111111);
		console.log("Hi");
		console.log("Hi", "Hi");
    }
	function test_log_elsewhere() public {
		_OtherContract _otherContract = new _OtherContract();
		_otherContract.test_log();
    }
}

contract _OtherContract {
    function test_log() public {
		console.log(0x1111111111111111111111111111111111111111);
		console.log("Hi");
		console.log("Hi", "Hi");
    }
}