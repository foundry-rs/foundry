// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract EnvTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testSetEnv() public {
        string memory key = "_foundryCheatcodeSetEnvTestKey";
        string memory val = "_foundryCheatcodeSetEnvTestVal";
        cheats.setEnv(key, val);
    }

    uint constant numEnvBoolTests = 4;
    function testEnvBool() public {
        string memory key = "_foundryCheatcodeEnvBoolTestKey";
        string[numEnvBoolTests] memory values = ["true", "false", "True", "False"];
        bool[numEnvBoolTests] memory expected = [true, false, true, false];
        for(uint i = 0; i < numEnvBoolTests; ++i){
            cheats.setEnv(key, values[i]);
            bool output = cheats.envBool(key);
            require(output == expected[i], "envBool failed");
        }
    }

    uint constant numEnvUintTests = 4;
    function testEnvUint() public {
        string memory key = "_foundryCheatcodeEnvUintTestKey";
        string[numEnvUintTests] memory values = [
            "0",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            type(uint256).min,
            type(uint256).max
        ];
        for(uint i = 0; i < numEnvUintTests; ++i){
            cheats.setEnv(key, values[i]);
            uint256 output = cheats.envUint(key);
            require(output == expected[i], "envUint failed");
        }
    }

    uint constant numEnvIntTests = 4;
    function testEnvInt() public {
        string memory key = "_foundryCheatcodeEnvIntTestKey";
        string[numEnvIntTests] memory values = [
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
            "57896044618658097711785492504343953926634992332820282019728792003956564819967",
            "-0x8000000000000000000000000000000000000000000000000000000000000000",
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        int256[numEnvIntTests] memory expected = [
            type(int256).min,
            type(int256).max,
            type(int256).min,
            type(int256).max
        ];
        for(uint i = 0; i < numEnvIntTests; ++i){
            cheats.setEnv(key, values[i]);
            int256 output = cheats.envInt(key);
            require(output == expected[i], "envInt failed");
        }
    }

    uint constant numEnvAddressTests = 1;
    function testEnvAddress() public {
        string memory key = "_foundryCheatcodeEnvAddressTestKey";
        string[numEnvAddressTests] memory values = [
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"
        ];
        address[numEnvAddressTests] memory expected = [
            0x7109709ECfa91a80626fF3989D68f67F5b1DD12D
        ];
        for(uint i = 0; i < numEnvAddressTests; ++i){
            cheats.setEnv(key, values[i]);
            address output = cheats.envAddress(key);
            require(output == expected[i], "envAddress failed");
        }
    }

    uint constant numEnvBytes32Tests = 1;
    function testEnvBytes32() public {
        string memory key = "_foundryCheatcodeEnvBytes32TestKey";
        string[numEnvBytes32Tests] memory values = [
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"
        ];
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000)
        ];
        for(uint i = 0; i < numEnvBytes32Tests; ++i){
            cheats.setEnv(key, values[i]);
            bytes32 output = cheats.envBytes32(key);
            require(output == expected[i], "envBytes32 failed");
        }
    }

    uint constant numEnvStringTests = 2;
    function testEnvString() public {
        string memory key = "_foundryCheatcodeEnvStringTestKey";
        string[numEnvStringTests] memory values = [
            "hello, world!",
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"
        ];
        string[numEnvStringTests] memory expected = [
            "hello, world!",
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"
        ];
        for(uint i = 0; i < numEnvStringTests; ++i){
            cheats.setEnv(key, values[i]);
            string memory output = cheats.envString(key);
            assertEq(output, expected[i], "envString failed");
        }
    }

    uint constant numEnvBytesTests = 1;
    function testEnvBytes() public {
        string memory key = "_foundryCheatcodeEnvBytesTestKey";
        string[numEnvBytesTests] memory values = [
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"
        ];
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        for(uint i = 0; i < numEnvBytesTests; ++i){
            cheats.setEnv(key, values[i]);
            bytes memory output = cheats.envBytes(key);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))),
                "envBytes failed"
            );
        }
    }

    function testEnvBoolArr() public {
        string memory key = "_foundryCheatcodeEnvBoolArrTestKey";
        string memory value = "true, false, True, False";
        bool[4] memory expected = [true, false, true, false];

        cheats.setEnv(key, value);
        bool[] memory output = cheats.envBoolArr(key);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envBoolArr failed"
        );
    }

    function testEnvUintArr() public {
        string memory key = "_foundryCheatcodeEnvUintArrTestKey";
        string memory value = 
            "0,"
            "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint[4] memory expected = [
            type(uint256).min,
            type(uint256).max,
            type(uint256).min,
            type(uint256).max
        ];

        cheats.setEnv(key, value);
        uint[] memory output = cheats.envUintArr(key);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envUintArr failed"
        );
    }

    function testEnvIntArr() public {
        string memory key = "_foundryCheatcodeEnvIntArrTestKey";
        string memory value =
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968,"
            "57896044618658097711785492504343953926634992332820282019728792003956564819967,"
            "-0x8000000000000000000000000000000000000000000000000000000000000000,"
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        int[4] memory expected = [
            type(int256).min,
            type(int256).max,
            type(int256).min,
            type(int256).max
        ];

        cheats.setEnv(key, value);
        int[] memory output = cheats.envIntArr(key);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envIntArr failed"
        );
    }

    function testEnvAddressArr() public {
        string memory key = "_foundryCheatcodeEnvAddressArrTestKey";
        string memory value =
            "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D,"
            "0x0000000000000000000000000000000000000000";
        address[2] memory expected = [
            0x7109709ECfa91a80626fF3989D68f67F5b1DD12D,
            0x0000000000000000000000000000000000000000
        ];

        cheats.setEnv(key, value);
        address[] memory output = cheats.envAddressArr(key);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envAddressArr failed"
        );
    }
}
