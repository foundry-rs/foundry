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

    uint256 constant numEnvUintTests = 6;

    function testEnvUint() public {
        string memory key = "_foundryCheatcodeEnvUintTestKey";
        string[numEnvUintTests] memory values = [
            "0",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0x01",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            1,
            77814517325470205911140941194401928579557062014761831930645393041380819009408,
            type(uint256).min,
            type(uint256).max
        ];
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

    function testEnvWithDefaultBoolKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBoolTestKey";
        string[numEnvBoolTests] memory values = ["true", "false", "True", "False"];
        bool[numEnvBoolTests] memory expected = [true, false, true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            cheats.setEnv(key, values[i]);
            bool output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultBoolKey failed");
        }
    }

    function testEnvWithDefaultBoolDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBoolTestDefault";
        bool[numEnvBoolTests] memory expected = [true, false, true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            bool output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultBoolDefault failed");
        }
    }

    function testEnvWithDefaultUintKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultUintTestKey";
        string[numEnvUintTests] memory values = [
            "0",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0x01",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            1,
            77814517325470205911140941194401928579557062014761831930645393041380819009408,
            type(uint256).min,
            type(uint256).max
        ];
        for (uint256 i = 0; i < numEnvUintTests; ++i) {
            cheats.setEnv(key, values[i]);
            uint256 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultUintKey failed");
        }
    }

    function testEnvWithDefaultUintDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultUintTestDefault";
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            1,
            77814517325470205911140941194401928579557062014761831930645393041380819009408,
            type(uint256).min,
            type(uint256).max
        ];
        for (uint256 i = 0; i < numEnvUintTests; ++i) {
            uint256 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultUintDefault failed");
        }
    }

    function testEnvWithDefaultIntKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultIntTestKey";
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
            int256 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultIntKey failed");
        }
    }

    function testEnvWithDefaultIntDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultIntTestDefault";
        int256[numEnvIntTests] memory expected =
            [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        for (uint256 i = 0; i < numEnvIntTests; ++i) {
            int256 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultIntDefault failed");
        }
    }

    function testEnvWithDefaultAddressKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultAddressTestKey";
        string[numEnvAddressTests] memory values =
            ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x0000000000000000000000000000000000000000"];
        address[numEnvAddressTests] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        for (uint256 i = 0; i < numEnvAddressTests; ++i) {
            cheats.setEnv(key, values[i]);
            address output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultAddressKey failed");
        }
    }

    function testEnvWithDefaultAddressDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultAddressTestDefault";
        address[numEnvAddressTests] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        for (uint256 i = 0; i < numEnvAddressTests; ++i) {
            address output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultAddressDefault failed");
        }
    }

    function testEnvWithDefaultBytes32Key() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytes32TestKey";
        string[numEnvBytes32Tests] memory values = ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x00"];
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            cheats.setEnv(key, values[i]);
            bytes32 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultBytes32Key failed");
        }
    }

    function testEnvWithDefaultBytes32Default() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytes32TestDefault";
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            bytes32 output = cheats.envOr(key, expected[i]);
            require(output == expected[i], "envWithDefaultBytes32Default failed");
        }
    }

    function testEnvWithDefaultStringKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultStringTestKey";
        string[numEnvStringTests] memory values = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            cheats.setEnv(key, values[i]);
            string memory output = cheats.envOr(key, expected[i]);
            assertEq(output, expected[i], "envWithDefaultStringKey failed");
        }
    }

    function testEnvWithDefaultStringDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultStringTestDefault";
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            string memory output = cheats.envOr(key, expected[i]);
            assertEq(output, expected[i], "envWithDefaultStringDefault failed");
        }
    }

    function testEnvWithDefaultBytesKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytesTestKey";
        string[numEnvBytesTests] memory values = ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x00"];
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";
        for (uint256 i = 0; i < numEnvBytesTests; ++i) {
            cheats.setEnv(key, values[i]);
            bytes memory output = cheats.envOr(key, expected[i]);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaultBytesKey failed"
            );
        }
    }

    function testEnvWithDefaultBytesDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytesTestDefault";
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";
        for (uint256 i = 0; i < numEnvBytesTests; ++i) {
            bytes memory output = cheats.envOr(key, expected[i]);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaultBytesDefault failed"
            );
        }
    }

    function testEnvWithDefaultBoolArrKey() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolArrTestKey";
        string memory value = "true, false, True, False";
        bool[4] memory expected = [true, false, true, false];
        bool[] memory defaultValues = new bool[](4);
        defaultValues[0] = true;
        defaultValues[1] = false;
        defaultValues[2] = true;
        defaultValues[3] = false;

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultBoolArrKey failed"
        );
    }

    function testEnvWithDefaultBoolArrDefault() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolArrTestDefault";
        string memory value = "true, false, True, False";
        bool[4] memory expected = [true, false, true, false];
        bool[] memory defaultValues = new bool[](4);
        defaultValues[0] = true;
        defaultValues[1] = false;
        defaultValues[2] = true;
        defaultValues[3] = false;

        string memory delimiter = ",";
        bool[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultBoolArrDefault failed"
        );
    }

    function testEnvWithDefaultUintArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultUintArrTestKey";
        string memory value = "0," "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint256[4] memory expected = [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];
        uint256[] memory defaultValues = new uint256[](4);
        defaultValues[0] = type(uint256).min;
        defaultValues[1] = type(uint256).max;
        defaultValues[2] = type(uint256).min;
        defaultValues[3] = type(uint256).max;

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultUintArrKey failed"
        );
    }

    function testEnvWithDefaultUintArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultUintArrTestDefault";
        string memory value = "0," "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint256[4] memory expected = [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];
        uint256[] memory defaultValues = new uint256[](4);
        defaultValues[0] = type(uint256).min;
        defaultValues[1] = type(uint256).max;
        defaultValues[2] = type(uint256).min;
        defaultValues[3] = type(uint256).max;

        string memory delimiter = ",";
        uint256[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultUintArrDefault failed"
        );
    }

    function testEnvWithDefaultIntArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultIntArrTestKey";
        string memory value = "-57896044618658097711785492504343953926634992332820282019728792003956564819968,"
            "57896044618658097711785492504343953926634992332820282019728792003956564819967,"
            "-0x8000000000000000000000000000000000000000000000000000000000000000,"
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        int256[4] memory expected = [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        int256[] memory defaultValues = new int256[](4);
        defaultValues[0] = type(int256).min;
        defaultValues[1] = type(int256).max;
        defaultValues[2] = type(int256).min;
        defaultValues[3] = type(int256).max;

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultIntArrKey failed"
        );
    }

    function testEnvWithDefaultIntArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultIntArrTestDefault";
        string memory value = "-57896044618658097711785492504343953926634992332820282019728792003956564819968,"
            "57896044618658097711785492504343953926634992332820282019728792003956564819967,"
            "-0x8000000000000000000000000000000000000000000000000000000000000000,"
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        int256[4] memory expected = [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        int256[] memory defaultValues = new int256[](4);
        defaultValues[0] = type(int256).min;
        defaultValues[1] = type(int256).max;
        defaultValues[2] = type(int256).min;
        defaultValues[3] = type(int256).max;

        string memory delimiter = ",";
        int256[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultIntArrDefault failed"
        );
    }

    function testEnvWithDefaultAddressArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultAddressArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        address[] memory defaultValues = new address[](2);
        defaultValues[0] = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        defaultValues[1] = 0x0000000000000000000000000000000000000000;

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultAddressArrKey failed"
        );
    }

    function testEnvWithDefaultAddressArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultAddressArrTestDefault";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        address[] memory defaultValues = new address[](2);
        defaultValues[0] = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        defaultValues[1] = 0x0000000000000000000000000000000000000000;

        string memory delimiter = ",";
        address[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultAddressArrDefault failed"
        );
    }

    function testEnvWithDefaultBytes32ArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytes32ArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes32[2] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        bytes32[] memory defaultValues = new bytes32[](2);
        defaultValues[0] = bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000);
        defaultValues[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000000);

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultBytes32ArrKey failed"
        );
    }

    function testEnvWithDefaultBytes32ArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultBytes32ArrTestDefault";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes32[2] memory expected = [
            bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        bytes32[] memory defaultValues = new bytes32[](2);
        defaultValues[0] = bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000);
        defaultValues[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000000);

        string memory delimiter = ",";
        bytes32[] memory output = cheats.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envWithDefaultBytes32ArrDefault failed"
        );
    }

    function testEnvWithDefaultStringArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultStringArrTestKey";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[] memory defaultValues = new string[](2);
        defaultValues[0] = "hello, world!";
        defaultValues[1] = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";

        cheats.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = cheats.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaultStringArrKey failed"
            );
        }
    }

    function testEnvWithDefaultStringArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaultStringArrTestDefault";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[] memory defaultValues = new string[](2);
        defaultValues[0] = "hello, world!";
        defaultValues[1] = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";

        string memory delimiter = "|";
        string[] memory output = cheats.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaultStringArrDefault failed"
            );
        }
    }

    function testEnvWithDefaulBytesArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaulBytesArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes[] memory expected = new bytes[](2);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";

        cheats.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = cheats.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaulBytesArrKey failed"
            );
        }
    }

    function testEnvWithDefaulBytesArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaulBytesArrTestDefault";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes[] memory expected = new bytes[](2);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";

        string memory delimiter = ",";
        bytes[] memory output = cheats.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaulBytesArrDefault failed"
            );
        }
    }
}
