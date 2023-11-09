// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract EnvTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSetEnv() public {
        string memory key = "_foundryCheatcodeSetEnvTestKey";
        string memory val = "_foundryCheatcodeSetEnvTestVal";
        vm.setEnv(key, val);
    }

    uint256 constant numEnvBoolTests = 2;

    function testEnvBool() public {
        string memory key = "_foundryCheatcodeEnvBoolTestKey";
        string[numEnvBoolTests] memory values = ["true", "false"];
        bool[numEnvBoolTests] memory expected = [true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            vm.setEnv(key, values[i]);
            bool output = vm.envBool(key);
            require(output == expected[i], "envBool failed");
        }
    }

    uint256 constant numEnvUintTests = 6;

    function testEnvUint() public {
        string memory key = "_foundryCheatcodeEnvUintTestKey";
        string[numEnvUintTests] memory values = [
            "0",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0x0a",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            10,
            77814517325470205911140941194401928579557062014761831930645393041380819009408,
            type(uint256).min,
            type(uint256).max
        ];
        for (uint256 i = 0; i < numEnvUintTests; ++i) {
            vm.setEnv(key, values[i]);
            uint256 output = vm.envUint(key);
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
            vm.setEnv(key, values[i]);
            int256 output = vm.envInt(key);
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
            vm.setEnv(key, values[i]);
            address output = vm.envAddress(key);
            require(output == expected[i], "envAddress failed");
        }
    }

    uint256 constant numEnvBytes32Tests = 2;

    function testEnvBytes32() public {
        string memory key = "_foundryCheatcodeEnvBytes32TestKey";
        string[numEnvBytes32Tests] memory values = [
            "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811",
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        ];
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            vm.setEnv(key, values[i]);
            bytes32 output = vm.envBytes32(key);
            require(output == expected[i], "envBytes32 failed");
        }
    }

    uint256 constant numEnvStringTests = 2;

    function testEnvString() public {
        string memory key = "_foundryCheatcodeEnvStringTestKey";
        string[numEnvStringTests] memory values = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            vm.setEnv(key, values[i]);
            string memory output = vm.envString(key);
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
            vm.setEnv(key, values[i]);
            bytes memory output = vm.envBytes(key);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))), "envBytes failed"
            );
        }
    }

    function testEnvBoolArr() public {
        string memory key = "_foundryCheatcodeEnvBoolArrTestKey";
        string memory value = "true, false";
        bool[numEnvBoolTests] memory expected = [true, false];

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = vm.envBool(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envBoolArr failed");
    }

    function testEnvUintArr() public {
        string memory key = "_foundryCheatcodeEnvUintArrTestKey";
        string memory value = "0," "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint256[4] memory expected = [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];

        vm.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = vm.envUint(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envUintArr failed");
    }

    function testEnvIntArr() public {
        string memory key = "_foundryCheatcodeEnvIntArrTestKey";
        string memory value = "-57896044618658097711785492504343953926634992332820282019728792003956564819968,"
            "57896044618658097711785492504343953926634992332820282019728792003956564819967,"
            "-0x8000000000000000000000000000000000000000000000000000000000000000,"
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        int256[4] memory expected = [type(int256).min, type(int256).max, type(int256).min, type(int256).max];

        vm.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = vm.envInt(key, delimiter);
        require(keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envIntArr failed");
    }

    function testEnvAddressArr() public {
        string memory key = "_foundryCheatcodeEnvAddressArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];

        vm.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = vm.envAddress(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envAddressArr failed"
        );
    }

    function testEnvBytes32Arr() public {
        string memory key = "_foundryCheatcodeEnvBytes32ArrTestKey";
        string memory value = "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811,"
            "0x0000000000000000000000000000000000000000000000000000000000000000";
        bytes32[2] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = vm.envBytes32(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envBytes32Arr failed"
        );
    }

    function testEnvStringArr() public {
        string memory key = "_foundryCheatcodeEnvStringArrTestKey";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];

        vm.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = vm.envString(key, delimiter);
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

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = vm.envBytes(key, delimiter);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envBytesArr failed"
            );
        }
    }

    function testEnvBoolEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvBoolEmptyArrTestKey";
        string memory value = "";
        bool[] memory expected = new bool[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = vm.envBool(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envBoolEmptyArr failed"
        );
    }

    function testEnvUintEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvUintEmptyArrTestKey";
        string memory value = "";
        uint256[] memory expected = new uint256[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = vm.envUint(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envUintEmptyArr failed"
        );
    }

    function testEnvIntEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvIntEmptyArrTestKey";
        string memory value = "";
        int256[] memory expected = new int256[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = vm.envInt(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envIntEmptyArr failed"
        );
    }

    function testEnvAddressEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvAddressEmptyArrTestKey";
        string memory value = "";
        address[] memory expected = new address[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = vm.envAddress(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envAddressEmptyArr failed"
        );
    }

    function testEnvBytes32EmptyArr() public {
        string memory key = "_foundryCheatcodeEnvBytes32EmptyArrTestKey";
        string memory value = "";
        bytes32[] memory expected = new bytes32[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = vm.envBytes32(key, delimiter);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envBytes32EmptyArr failed"
        );
    }

    function testEnvStringEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvStringEmptyArrTestKey";
        string memory value = "";
        string[] memory expected = new string[](0);

        vm.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = vm.envString(key, delimiter);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envStringEmptyArr failed"
            );
        }
    }

    function testEnvBytesEmptyArr() public {
        string memory key = "_foundryCheatcodeEnvBytesEmptyArrTestKey";
        string memory value = "";
        bytes[] memory expected = new bytes[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = vm.envBytes(key, delimiter);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envBytesEmptyArr failed"
            );
        }
    }

    function testEnvOrBoolKey() public {
        string memory key = "_foundryCheatcodeEnvOrBoolTestKey";
        string[numEnvBoolTests] memory values = ["true", "false"];
        bool[numEnvBoolTests] memory expected = [true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            vm.setEnv(key, values[i]);
            bool output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrBoolKey failed");
        }
    }

    function testEnvOrBoolDefault() public {
        string memory key = "_foundryCheatcodeEnvOrBoolTestDefault";
        bool[numEnvBoolTests] memory expected = [true, false];
        for (uint256 i = 0; i < numEnvBoolTests; ++i) {
            bool output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrBoolDefault failed");
        }
    }

    function testEnvOrUintKey() public {
        string memory key = "_foundryCheatcodeEnvOrUintTestKey";
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
            vm.setEnv(key, values[i]);
            uint256 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrUintKey failed");
        }
    }

    function testEnvOrUintDefault() public {
        string memory key = "_foundryCheatcodeEnvOrUintTestDefault";
        uint256[numEnvUintTests] memory expected = [
            type(uint256).min,
            type(uint256).max,
            1,
            77814517325470205911140941194401928579557062014761831930645393041380819009408,
            type(uint256).min,
            type(uint256).max
        ];
        for (uint256 i = 0; i < numEnvUintTests; ++i) {
            uint256 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrUintDefault failed");
        }
    }

    function testEnvOrIntKey() public {
        string memory key = "_foundryCheatcodeEnvOrIntTestKey";
        string[numEnvIntTests] memory values = [
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
            "57896044618658097711785492504343953926634992332820282019728792003956564819967",
            "-0x8000000000000000000000000000000000000000000000000000000000000000",
            "0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ];
        int256[numEnvIntTests] memory expected =
            [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        for (uint256 i = 0; i < numEnvIntTests; ++i) {
            vm.setEnv(key, values[i]);
            int256 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrIntKey failed");
        }
    }

    function testEnvOrIntDefault() public {
        string memory key = "_foundryCheatcodeEnvOrIntTestDefault";
        int256[numEnvIntTests] memory expected =
            [type(int256).min, type(int256).max, type(int256).min, type(int256).max];
        for (uint256 i = 0; i < numEnvIntTests; ++i) {
            int256 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrIntDefault failed");
        }
    }

    function testEnvOrAddressKey() public {
        string memory key = "_foundryCheatcodeEnvOrAddressTestKey";
        string[numEnvAddressTests] memory values =
            ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x0000000000000000000000000000000000000000"];
        address[numEnvAddressTests] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        for (uint256 i = 0; i < numEnvAddressTests; ++i) {
            vm.setEnv(key, values[i]);
            address output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrAddressKey failed");
        }
    }

    function testEnvOrAddressDefault() public {
        string memory key = "_foundryCheatcodeEnvOrAddressTestDefault";
        address[numEnvAddressTests] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        for (uint256 i = 0; i < numEnvAddressTests; ++i) {
            address output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrAddressDefault failed");
        }
    }

    function testEnvOrBytes32Key() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32TestKey";
        string[numEnvBytes32Tests] memory values = [
            "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811",
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        ];
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            vm.setEnv(key, values[i]);
            bytes32 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrBytes32Key failed");
        }
    }

    function testEnvOrBytes32Default() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32TestDefault";
        bytes32[numEnvBytes32Tests] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        for (uint256 i = 0; i < numEnvBytes32Tests; ++i) {
            bytes32 output = vm.envOr(key, expected[i]);
            require(output == expected[i], "envOrBytes32Default failed");
        }
    }

    function testEnvOrStringKey() public {
        string memory key = "_foundryCheatcodeEnvOrStringTestKey";
        string[numEnvStringTests] memory values = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            vm.setEnv(key, values[i]);
            string memory output = vm.envOr(key, expected[i]);
            assertEq(output, expected[i], "envOrStringKey failed");
        }
    }

    function testEnvOrStringDefault() public {
        string memory key = "_foundryCheatcodeEnvOrStringTestDefault";
        string[numEnvStringTests] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        for (uint256 i = 0; i < numEnvStringTests; ++i) {
            string memory output = vm.envOr(key, expected[i]);
            assertEq(output, expected[i], "envOrStringDefault failed");
        }
    }

    function testEnvOrBytesKey() public {
        string memory key = "_foundryCheatcodeEnvOrBytesTestKey";
        string[numEnvBytesTests] memory values = ["0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", "0x00"];
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";
        for (uint256 i = 0; i < numEnvBytesTests; ++i) {
            vm.setEnv(key, values[i]);
            bytes memory output = vm.envOr(key, expected[i]);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrBytesKey failed"
            );
        }
    }

    function testEnvOrBytesDefault() public {
        string memory key = "_foundryCheatcodeEnvOrBytesTestDefault";
        bytes[] memory expected = new bytes[](numEnvBytesTests);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";
        for (uint256 i = 0; i < numEnvBytesTests; ++i) {
            bytes memory output = vm.envOr(key, expected[i]);
            require(
                keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrBytesDefault failed"
            );
        }
    }

    function testEnvOrBoolArrKey() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolArrTestKey";
        string memory value = "true, false";
        bool[numEnvBoolTests] memory expected = [true, false];
        bool[] memory defaultValues = new bool[](numEnvBoolTests);
        defaultValues[0] = true;
        defaultValues[1] = false;

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envOrBoolArrKey failed"
        );
    }

    function testEnvOrBoolArrDefault() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolArrTestDefault";
        string memory value = "true, false";
        bool[numEnvBoolTests] memory expected = [true, false];
        bool[] memory defaultValues = new bool[](numEnvBoolTests);
        defaultValues[0] = true;
        defaultValues[1] = false;

        string memory delimiter = ",";
        bool[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBoolArrDefault failed"
        );
    }

    function testEnvOrUintArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrUintArrTestKey";
        string memory value = "0," "115792089237316195423570985008687907853269984665640564039457584007913129639935,"
            "0x0000000000000000000000000000000000000000000000000000000000000000,"
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        uint256[4] memory expected = [type(uint256).min, type(uint256).max, type(uint256).min, type(uint256).max];
        uint256[] memory defaultValues = new uint256[](4);
        defaultValues[0] = type(uint256).min;
        defaultValues[1] = type(uint256).max;
        defaultValues[2] = type(uint256).min;
        defaultValues[3] = type(uint256).max;

        vm.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envOrUintArrKey failed"
        );
    }

    function testEnvOrUintArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrUintArrTestDefault";
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
        uint256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrUintArrDefault failed"
        );
    }

    function testEnvOrIntArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrIntArrTestKey";
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

        vm.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))), "envOrIntArrKey failed"
        );
    }

    function testEnvOrIntArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrIntArrTestDefault";
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
        int256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrIntArrDefault failed"
        );
    }

    function testEnvOrAddressArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrAddressArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        address[] memory defaultValues = new address[](2);
        defaultValues[0] = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        defaultValues[1] = 0x0000000000000000000000000000000000000000;

        vm.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrAddressArrKey failed"
        );
    }

    function testEnvOrAddressArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrAddressArrTestDefault";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x0000000000000000000000000000000000000000";
        address[2] memory expected =
            [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, 0x0000000000000000000000000000000000000000];
        address[] memory defaultValues = new address[](2);
        defaultValues[0] = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        defaultValues[1] = 0x0000000000000000000000000000000000000000;

        string memory delimiter = ",";
        address[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrAddressArrDefault failed"
        );
    }

    function testEnvOrBytes32ArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32ArrTestKey";
        string memory value = "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811,"
            "0x0000000000000000000000000000000000000000000000000000000000000000";
        bytes32[2] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        bytes32[] memory defaultValues = new bytes32[](2);
        defaultValues[0] = bytes32(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D000000000000000000000000);
        defaultValues[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000000);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBytes32ArrKey failed"
        );
    }

    function testEnvOrBytes32ArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32ArrTestDefault";
        string memory value = "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811,"
            "0x0000000000000000000000000000000000000000000000000000000000000000";
        bytes32[2] memory expected = [
            bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811),
            bytes32(0x0000000000000000000000000000000000000000000000000000000000000000)
        ];
        bytes32[] memory defaultValues = new bytes32[](2);
        defaultValues[0] = bytes32(0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811);
        defaultValues[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000000);

        string memory delimiter = ",";
        bytes32[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBytes32ArrDefault failed"
        );
    }

    function testEnvOrStringArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrStringArrTestKey";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[] memory defaultValues = new string[](2);
        defaultValues[0] = "hello, world!";
        defaultValues[1] = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";

        vm.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = vm.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrStringArrKey failed"
            );
        }
    }

    function testEnvOrStringArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrStringArrTestDefault";
        string memory value = "hello, world!|" "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string[2] memory expected = ["hello, world!", "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"];
        string[] memory defaultValues = new string[](2);
        defaultValues[0] = "hello, world!";
        defaultValues[1] = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";

        string memory delimiter = "|";
        string[] memory output = vm.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrStringArrDefault failed"
            );
        }
    }

    function testEnvOrBytesArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrBytesArrTestKey";
        string memory value = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D," "0x00";
        bytes[] memory expected = new bytes[](2);
        expected[0] = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        expected[1] = hex"00";

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = vm.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrBytesArrKey failed"
            );
        }
    }

    function testEnvOrBytesArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrBytesArrTestDefault";
        string memory value = "0x463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811," "0x00";
        bytes[] memory expected = new bytes[](2);
        expected[0] = hex"463df98a03418e6196421718c1b96779a6d4f0bcff1702a9e8f2323bb49f6811";
        expected[1] = hex"00";

        string memory delimiter = ",";
        bytes[] memory output = vm.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrBytesArrDefault failed"
            );
        }
    }

    function testEnvOrBoolEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolEmptyArrTestKey";
        string memory value = "";
        bool[] memory expected = new bool[](0);
        bool[] memory defaultValues = new bool[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bool[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBoolEmptyArrKey failed"
        );
    }

    function testEnvOrBoolEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvBoolWithDefaultBoolEmptyArrTestDefault";
        string memory value = "";
        bool[] memory expected = new bool[](0);
        bool[] memory defaultValues = new bool[](0);

        string memory delimiter = ",";
        bool[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBoolEmptyArrDefault failed"
        );
    }

    function testEnvOrUintEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrUintEmptyArrTestKey";
        string memory value = "";
        uint256[] memory expected = new uint256[](0);
        uint256[] memory defaultValues = new uint256[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        uint256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrUintEmptyArrKey failed"
        );
    }

    function testEnvOrUintEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrUintEmptyArrTestDefault";
        string memory value = "";
        uint256[] memory expected = new uint256[](0);
        uint256[] memory defaultValues = new uint256[](0);

        string memory delimiter = ",";
        uint256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrUintEmptyArrDefault failed"
        );
    }

    function testEnvOrIntEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrIntEmptyArrTestKey";
        string memory value = "";
        int256[] memory expected = new int256[](0);
        int256[] memory defaultValues = new int256[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        int256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrIntEmptyArrKey failed"
        );
    }

    function testEnvOrIntEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrIntEmptyArrTestDefault";
        string memory value = "";
        int256[] memory expected = new int256[](0);
        int256[] memory defaultValues = new int256[](0);

        string memory delimiter = ",";
        int256[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrIntEmptyArrDefault failed"
        );
    }

    function testEnvOrAddressEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrAddressEmptyArrTestKey";
        string memory value = "";
        address[] memory expected = new address[](0);
        address[] memory defaultValues = new address[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        address[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrAddressEmptyArrKey failed"
        );
    }

    function testEnvOrAddressEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrAddressEmptyArrTestDefault";
        string memory value = "";
        address[] memory expected = new address[](0);
        address[] memory defaultValues = new address[](0);

        string memory delimiter = ",";
        address[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrAddressEmptyArrDefault failed"
        );
    }

    function testEnvOrBytes32EmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32EmptyArrTestKey";
        string memory value = "";
        bytes32[] memory expected = new bytes32[](0);
        bytes32[] memory defaultValues = new bytes32[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes32[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBytes32EmptyArrKey failed"
        );
    }

    function testEnvOrBytes32EmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrBytes32EmptyArrTestDefault";
        string memory value = "";
        bytes32[] memory expected = new bytes32[](0);
        bytes32[] memory defaultValues = new bytes32[](0);

        string memory delimiter = ",";
        bytes32[] memory output = vm.envOr(key, delimiter, defaultValues);
        require(
            keccak256(abi.encodePacked((output))) == keccak256(abi.encodePacked((expected))),
            "envOrBytes32EmptyArrDefault failed"
        );
    }

    function testEnvOrStringEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvOrStringEmptyArrTestKey";
        string memory value = "";
        string[] memory expected = new string[](0);
        string[] memory defaultValues = new string[](0);

        vm.setEnv(key, value);
        string memory delimiter = "|";
        string[] memory output = vm.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrStringEmptyArrKey failed"
            );
        }
    }

    function testEnvOrStringEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvOrStringEmptyArrTestDefault";
        string memory value = "";
        string[] memory expected = new string[](0);
        string[] memory defaultValues = new string[](0);

        string memory delimiter = "|";
        string[] memory output = vm.envOr(key, delimiter, defaultValues);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envOrStringEmptyArrDefault failed"
            );
        }
    }

    function testEnvWithDefaulBytesEmptyArrKey() public {
        string memory key = "_foundryCheatcodeEnvWithDefaulBytesEmptyArrTestKey";
        string memory value = "";
        bytes[] memory expected = new bytes[](0);
        bytes[] memory defaultValues = new bytes[](0);

        vm.setEnv(key, value);
        string memory delimiter = ",";
        bytes[] memory output = vm.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaulBytesEmptyArrKey failed"
            );
        }
    }

    function testEnvWithDefaulBytesEmptyArrDefault() public {
        string memory key = "_foundryCheatcodeEnvWithDefaulBytesEmptyArrTestDefault";
        string memory value = "";
        bytes[] memory expected = new bytes[](0);
        bytes[] memory defaultValues = new bytes[](0);

        string memory delimiter = ",";
        bytes[] memory output = vm.envOr(key, delimiter, expected);
        for (uint256 i = 0; i < expected.length; ++i) {
            require(
                keccak256(abi.encodePacked((output[i]))) == keccak256(abi.encodePacked((expected[i]))),
                "envWithDefaulBytesEmptyArrDefault failed"
            );
        }
    }
}
