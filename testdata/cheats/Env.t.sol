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

    uint256 constant numEnvBoolTests = 4;

    function testEnvBool() public {
        string memory key = "_foundryCheatcodeEnvBoolTestKey";
        string[numEnvBoolTests] memory values = ["true", "false", "True", "False"];
        bool[numEnvBoolTests] memory expected = [true, false, true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            cheats.setEnv(key, values[i]);
            bool output = cheats.envBool(key);
            require(output == expected[i], "envBool failed");
        }
    }

    uint256 constant numEnvUintTests = 4;

    function testEnvUint() public {
        string memory key = "_foundryCheatcodeEnvUintTestKey";
        string[numEnvUintTests] memory values = [
            "0",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        uint256[numEnvUintTests] memory expected =
            [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];
        for (uint256 i = 0; i < numEnvUintTests; ++i) {
            cheats.setEnv(key, values[i]);
            uint256 output = cheats.envUint(key);
            require(output == expected[i], "envUint failed");
        }
    }

    uint256 constant numEnvIntTests = 4;

    function testEnvInt() public {
        string memory key = "_foundryCheatcodeEnvIntTestKey";
        string[numEnvIntTests] memory values = [
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
            "57896044618658097711785492504343953926634992332820282019728792003956564819967",
            "-0x8000000000000000000000000000000000000000000000000000000000000000",
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        int256[numEnvIntTests] memory expected =
            [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        for (uint256 i = 0; i < numEnvIntTests; ++i) {
            cheats.setEnv(key, values[i]);
            int256 output = cheats.envInt(key);
            require(output == expected[i], "envInt failed");
        }
    }

    uint256 constant numEnvAddressTests = 2;

    function testEnvAddress() public {
        string memory key = "_foundryCheatcodeEnvAddressTestKey";
        string[numEnvAddressTests] memory values =
            ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x0000000000000000000000000000000000000000"];
        address[numEnvAddressTests] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        for (uint256 i = 0; i < numEnvAddressTests; ++i) {
            cheats.setEnv(key, values[i]);
            address output = cheats.envAddress(key);
            require(output == expected[i], "envAddress failed");
        }
    }

    uint256 constant numEnvBytes32Tests = 2;

    function testEnvBytes32() public {
        string memory key = "_foundryCheatcodeEnvBytes32TestKey";
        string[numEnvBytes32Tests] memory values = ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x00"];
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            cheats.setEnv(key, values[i]);
            bytes32 output = cheats.envBytes32(key);
            require(output == expected[i], "envBytes32 failed");
        }
    }

    uint256 constant numEnvStringTests = 2;

    function testEnvString() public {
        string memory key = "_foundryCheatcodeEnvStringTestKey";
        string[numEnvStringTests] memory values = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            cheats.setEnv(key, values[i]);
            string memory output = cheats.envString(key);
            assertEq(output, expected[i], "envString failed");
        }
    }

    uint256 constant numEnvBytesTests = 2;

    function testEnvBytes() public {
        string memory key = "_foundryCheatcodeEnvBytesTestKey";
        string[numEnvBytesTests] memory values = ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x00"];
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";
        for (uint256 i = 0; i < numEnvBytesTests; ++i) {
            cheats.setEnv(key, values[i]);
            bytes memory output = cheats.envBytes(key);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))), "envBytes failed"
            );
        }
    }

    function testEnvBoolArr() public {
        string memory key = "_foundryCheatcodeEnvBoolArrTestKey";
        string memory value = "true, false, True, False";
        bool[4] memory expected = [true, false, true, false];

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = cheats.envBool(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envBoolArr failed");
    }

    function testEnvUintArr() public {
        string memory key = "_foundryCheatcodeEnvUintArrTestKey";
        string memory value = "0," "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint256[4] memory expected = [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = cheats.envUint(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envUintArr failed");
    }

    function testEnvIntArr() public {
        string memory key = "_foundryCheatcodeEnvIntArrTestKey";
        string memory value = "-57896044618658097711785492504343953926634992332820282019728792003956564819968,"
            "57896044618658097711785492504343953926634992332820282019728792003956564819967,"
            "-0x8000000000000000000000000000000000000000000000000000000000000000,"
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        int256[4] memory expected = [type(int256).min, type(int256).max, type(int256).min, type(int256).max];

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = cheats.envInt(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envIntArr failed");
    }

    function testEnvAddressArr() public {
        string memory key = "_foundryCheatcodeEnvAddressArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = cheats.envAddress(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envAddressArr failed"
        );
    }

    function testEnvBytes32Arr() public {
        string memory key = "_foundryCheatcodeEnvBytes32ArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes32[2] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = cheats.envBytes32(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envBytes32Arr failed"
        );
    }

    function testEnvStringArr() public {
        string memory key = "_foundryCheatcodeEnvStringArrTestKey";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];

        cheats.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = cheats.envString(key, delimiter);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envStringArr failed"
            );
        }
    }

    function testEnvBytesArr() public {
        string memory key = "_foundryCheatcodeEnvBytesArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = cheats.envBytes(key, delimiter);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envBytesArr failed"
            );
        }
    }
}
