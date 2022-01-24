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
		console.logAddress(address(0x1111111111111111111111111111111111111111));
	}

	function test_log_types_bytes() public {
		console.logBytes("logBytes");
		console.logBytes(hex"fba3a4b5");
		console.logBytes1(hex"fb");
		console.logBytes2(hex"fba3");
		console.logBytes3(hex"fba3a4");
		console.logBytes4(hex"fba3a4b5");
		console.logBytes5(hex"fba3a4b5");
		console.logBytes6(hex"fba3a4b5");
		console.logBytes7(hex"fba3a4b5");
		console.logBytes8(hex"fba3a4b5");
		console.logBytes9(hex"fba3a4b5");
		console.logBytes10(hex"fba3a4b5");
		console.logBytes11(hex"fba3a4b5");
		console.logBytes12(hex"fba3a4b5");
		console.logBytes13(hex"fba3a4b5");
		console.logBytes14(hex"fba3a4b5");
		console.logBytes15(hex"fba3a4b5");
		console.logBytes16(hex"fba3a4b5");
		console.logBytes17(hex"fba3a4b5");
		console.logBytes18(hex"fba3a4b5");
		console.logBytes19(hex"fba3a4b5");
		console.logBytes20(hex"fba3a4b5");
		console.logBytes21(hex"fba3a4b5");
		console.logBytes22(hex"fba3a4b5");
		console.logBytes23(hex"fba3a4b5");
		console.logBytes24(hex"fba3a4b5");
		console.logBytes25(hex"fba3a4b5");
		console.logBytes26(hex"fba3a4b5");
		console.logBytes27(hex"fba3a4b5");
		console.logBytes28(hex"fba3a4b5");
		console.logBytes29(hex"fba3a4b5");
		console.logBytes30(hex"fba3a4b5");
		console.logBytes31(hex"fba3a4b5");
		console.logBytes32(hex"fba3a4b5");
	}}
