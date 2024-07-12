// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract AssertionsTest is DSTest {
    string constant errorMessage = "User provided message";
    uint256 constant maxDecimals = 77;

    Vm constant vm = Vm(HEVM_ADDRESS);

    function _abs(int256 a) internal pure returns (uint256) {
        // Required or it will fail when `a = type(int256).min`
        if (a == type(int256).min) {
            return uint256(type(int256).max) + 1;
        }

        return uint256(a > 0 ? a : -a);
    }

    function _getDelta(uint256 a, uint256 b) internal pure returns (uint256) {
        return a > b ? a - b : b - a;
    }

    function _getDelta(int256 a, int256 b) internal pure returns (uint256) {
        // a and b are of the same sign
        // this works thanks to two's complement, the left-most bit is the sign bit
        if ((a ^ b) > -1) {
            return _getDelta(_abs(a), _abs(b));
        }

        // a and b are of opposite signs
        return _abs(a) + _abs(b);
    }

    function _prefixDecWithZeroes(string memory intPart, string memory decimalPart, uint256 decimals)
        internal
        returns (string memory)
    {
        while (bytes(decimalPart).length < decimals) {
            decimalPart = string.concat("0", decimalPart);
        }

        return string.concat(intPart, ".", decimalPart);
    }

    function _formatWithDecimals(uint256 value, uint256 decimals) internal returns (string memory) {
        string memory intPart = vm.toString(value / (10 ** decimals));
        string memory decimalPart = vm.toString(value % (10 ** decimals));

        return _prefixDecWithZeroes(intPart, decimalPart, decimals);
    }

    function _formatWithDecimals(int256 value, uint256 decimals) internal returns (string memory) {
        string memory formatted = _formatWithDecimals(_abs(value), decimals);
        if (value < 0) {
            formatted = string.concat("-", formatted);
        }

        return formatted;
    }

    function testFuzzAssertEqNotEq(uint256 left, uint256 right, uint256 decimals) public {
        vm.assume(left != right);
        vm.assume(decimals <= maxDecimals);

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " != ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertEqDecimal(left, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " == ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertNotEqDecimal(left, left, decimals);
    }

    function testFuzzAssertEqNotEq(int256 left, int256 right, uint256 decimals) public {
        vm.assume(left != right);
        vm.assume(decimals <= maxDecimals);

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    errorMessage,
                    ": ",
                    _formatWithDecimals(left, decimals),
                    " != ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertEqDecimal(left, right, decimals, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    errorMessage, ": ", _formatWithDecimals(left, decimals), " == ", _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertNotEqDecimal(left, left, decimals, errorMessage);
    }

    function testFuzzAssertEqNotEq(bool left, bool right) public {
        vm.assume(left != right);

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);
    }

    function testFuzzAssertEqNotEq(address left, address right) public {
        vm.assume(left != right);

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);
    }

    function testFuzzAssertEqNotEq(bytes32 left, bytes32 right) public {
        vm.assume(left != right);

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);
    }

    function testFuzzAssertEqNotEq(string memory left, string memory right) public {
        vm.assume(keccak256(abi.encodePacked(left)) != keccak256(abi.encodePacked(right)));

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": ", left, " != ", right)));
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": ", left, " == ", left)));
        vm.assertNotEq(left, left, errorMessage);
    }

    function testFuzzAssertEqNotEq(bytes memory left, bytes memory right) public {
        vm.assume(keccak256(left) != keccak256(right));

        vm.assertEq(left, left);
        vm.assertEq(right, right);
        vm.assertNotEq(left, right);
        vm.assertNotEq(right, left);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " != ", vm.toString(right)))
        );
        vm.assertEq(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " == ", vm.toString(left)))
        );
        vm.assertNotEq(left, left, errorMessage);
    }

    function testFuzzAssertGtLt(uint256 left, uint256 right, uint256 decimals) public {
        vm.assume(left < right);
        vm.assume(decimals <= maxDecimals);

        vm.assertGt(right, left);
        vm.assertLt(left, right);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " <= ", vm.toString(right)))
        );
        vm.assertGt(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " <= ", vm.toString(right)))
        );
        vm.assertGt(right, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " >= ", vm.toString(left)))
        );
        vm.assertLt(left, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " >= ", vm.toString(left)))
        );
        vm.assertLt(right, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " <= ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGtDecimal(left, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " <= ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGtDecimal(right, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " >= ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLtDecimal(left, left, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " >= ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLtDecimal(right, left, decimals);
    }

    function testFuzzAssertGtLt(int256 left, int256 right, uint256 decimals) public {
        vm.assume(left < right);
        vm.assume(decimals <= maxDecimals);

        vm.assertGt(right, left);
        vm.assertLt(left, right);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " <= ", vm.toString(right)))
        );
        vm.assertGt(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " <= ", vm.toString(right)))
        );
        vm.assertGt(right, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " >= ", vm.toString(left)))
        );
        vm.assertLt(left, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " >= ", vm.toString(left)))
        );
        vm.assertLt(right, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " <= ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGtDecimal(left, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " <= ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGtDecimal(right, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " >= ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLtDecimal(left, left, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " >= ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLtDecimal(right, left, decimals);
    }

    function testFuzzAssertGeLe(uint256 left, uint256 right, uint256 decimals) public {
        vm.assume(left < right);
        vm.assume(decimals <= maxDecimals);

        vm.assertGe(left, left);
        vm.assertLe(left, left);
        vm.assertGe(right, right);
        vm.assertLe(right, right);
        vm.assertGe(right, left);
        vm.assertLe(left, right);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " < ", vm.toString(right)))
        );
        vm.assertGe(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " > ", vm.toString(left)))
        );
        vm.assertLe(right, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " < ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGeDecimal(left, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " > ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLeDecimal(right, left, decimals);
    }

    function testFuzzAssertGeLe(int256 left, int256 right, uint256 decimals) public {
        vm.assume(left < right);
        vm.assume(decimals <= maxDecimals);

        vm.assertGe(left, left);
        vm.assertLe(left, left);
        vm.assertGe(right, right);
        vm.assertLe(right, right);
        vm.assertGe(right, left);
        vm.assertLe(left, right);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(left), " < ", vm.toString(right)))
        );
        vm.assertGe(left, right, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": ", vm.toString(right), " > ", vm.toString(left)))
        );
        vm.assertLe(right, left, errorMessage);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(left, decimals),
                    " < ",
                    _formatWithDecimals(right, decimals)
                )
            )
        );
        vm.assertGeDecimal(left, right, decimals);

        vm._expectCheatcodeRevert(
            bytes(
                string.concat(
                    "assertion failed: ",
                    _formatWithDecimals(right, decimals),
                    " > ",
                    _formatWithDecimals(left, decimals)
                )
            )
        );
        vm.assertLeDecimal(right, left, decimals);
    }

    function testFuzzAssertApproxEqAbs(uint256 left, uint256 right, uint256 decimals) public {
        uint256 delta = _getDelta(right, left);
        vm.assume(decimals <= maxDecimals);

        vm.assertApproxEqAbs(left, right, delta);

        if (delta > 0) {
            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        errorMessage,
                        ": ",
                        vm.toString(left),
                        " !~= ",
                        vm.toString(right),
                        " (max delta: ",
                        vm.toString(delta - 1),
                        ", real delta: ",
                        vm.toString(delta),
                        ")"
                    )
                )
            );
            vm.assertApproxEqAbs(left, right, delta - 1, errorMessage);

            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        "assertion failed: ",
                        _formatWithDecimals(left, decimals),
                        " !~= ",
                        _formatWithDecimals(right, decimals),
                        " (max delta: ",
                        _formatWithDecimals(delta - 1, decimals),
                        ", real delta: ",
                        _formatWithDecimals(delta, decimals),
                        ")"
                    )
                )
            );
            vm.assertApproxEqAbsDecimal(left, right, delta - 1, decimals);
        }
    }

    function testFuzzAssertApproxEqAbs(int256 left, int256 right, uint256 decimals) public {
        uint256 delta = _getDelta(right, left);
        vm.assume(decimals <= maxDecimals);

        vm.assertApproxEqAbs(left, right, delta);

        if (delta > 0) {
            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        errorMessage,
                        ": ",
                        vm.toString(left),
                        " !~= ",
                        vm.toString(right),
                        " (max delta: ",
                        vm.toString(delta - 1),
                        ", real delta: ",
                        vm.toString(delta),
                        ")"
                    )
                )
            );
            vm.assertApproxEqAbs(left, right, delta - 1, errorMessage);

            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        "assertion failed: ",
                        _formatWithDecimals(left, decimals),
                        " !~= ",
                        _formatWithDecimals(right, decimals),
                        " (max delta: ",
                        _formatWithDecimals(delta - 1, decimals),
                        ", real delta: ",
                        _formatWithDecimals(delta, decimals),
                        ")"
                    )
                )
            );
            vm.assertApproxEqAbsDecimal(left, right, delta - 1, decimals);
        }
    }

    function testFuzzAssertApproxEqRel(uint256 left, uint256 right, uint256 decimals) public {
        vm.assume(right != 0);
        uint256 delta = _getDelta(right, left);
        vm.assume(delta < type(uint256).max / (10 ** 18));
        vm.assume(decimals <= maxDecimals);

        uint256 percentDelta = delta * (10 ** 18) / right;

        vm.assertApproxEqRel(left, right, percentDelta);

        if (percentDelta > 0) {
            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        errorMessage,
                        ": ",
                        vm.toString(left),
                        " !~= ",
                        vm.toString(right),
                        " (max delta: ",
                        _formatWithDecimals(percentDelta - 1, 16),
                        "%, real delta: ",
                        _formatWithDecimals(percentDelta, 16),
                        "%)"
                    )
                )
            );
            vm.assertApproxEqRel(left, right, percentDelta - 1, errorMessage);

            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        "assertion failed: ",
                        _formatWithDecimals(left, decimals),
                        " !~= ",
                        _formatWithDecimals(right, decimals),
                        " (max delta: ",
                        _formatWithDecimals(percentDelta - 1, 16),
                        "%, real delta: ",
                        _formatWithDecimals(percentDelta, 16),
                        "%)"
                    )
                )
            );
            vm.assertApproxEqRelDecimal(left, right, percentDelta - 1, decimals);
        }
    }

    function testFuzzAssertApproxEqRel(int256 left, int256 right, uint256 decimals) public {
        vm.assume(left < right);
        vm.assume(right != 0);
        uint256 delta = _getDelta(right, left);
        vm.assume(delta < type(uint256).max / (10 ** 18));
        vm.assume(decimals <= maxDecimals);

        uint256 percentDelta = delta * (10 ** 18) / _abs(right);

        vm.assertApproxEqRel(left, right, percentDelta);

        if (percentDelta > 0) {
            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        errorMessage,
                        ": ",
                        vm.toString(left),
                        " !~= ",
                        vm.toString(right),
                        " (max delta: ",
                        _formatWithDecimals(percentDelta - 1, 16),
                        "%, real delta: ",
                        _formatWithDecimals(percentDelta, 16),
                        "%)"
                    )
                )
            );
            vm.assertApproxEqRel(left, right, percentDelta - 1, errorMessage);

            vm._expectCheatcodeRevert(
                bytes(
                    string.concat(
                        "assertion failed: ",
                        _formatWithDecimals(left, decimals),
                        " !~= ",
                        _formatWithDecimals(right, decimals),
                        " (max delta: ",
                        _formatWithDecimals(percentDelta - 1, 16),
                        "%, real delta: ",
                        _formatWithDecimals(percentDelta, 16),
                        "%)"
                    )
                )
            );
            vm.assertApproxEqRelDecimal(left, right, percentDelta - 1, decimals);
        }
    }

    function testAssertEqNotEqArrays() public {
        {
            uint256[] memory arr1 = new uint256[](1);
            arr1[0] = 1;
            uint256[] memory arr2 = new uint256[](2);
            arr2[0] = 1;
            arr2[1] = 2;

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes("assertion failed: [1] != [1, 2]"));
            vm.assertEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat("assertion failed: [1, 2] == [1, 2]")));
            vm.assertNotEq(arr2, arr2);
        }
        {
            int256[] memory arr1 = new int256[](2);
            int256[] memory arr2 = new int256[](1);
            arr1[0] = 5;
            arr2[0] = type(int256).max;

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": [5, 0] != [", vm.toString(arr2[0]), "]")));
            vm.assertEq(arr1, arr2, errorMessage);

            vm._expectCheatcodeRevert(bytes(string.concat("assertion failed: [5, 0] == [5, 0]")));
            vm.assertNotEq(arr1, arr1);
        }
        {
            bool[] memory arr1 = new bool[](2);
            bool[] memory arr2 = new bool[](2);
            arr1[0] = true;
            arr2[1] = true;

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": [true, false] != [false, true]")));
            vm.assertEq(arr1, arr2, errorMessage);

            vm._expectCheatcodeRevert(bytes(string("assertion failed: [true, false] == [true, false]")));
            vm.assertNotEq(arr1, arr1);
        }
        {
            address[] memory arr1 = new address[](1);
            address[] memory arr2 = new address[](0);

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": [", vm.toString(arr1[0]), "] != []")));
            vm.assertEq(arr1, arr2, errorMessage);

            vm._expectCheatcodeRevert(bytes(string("assertion failed: [] == []")));
            vm.assertNotEq(arr2, arr2);
        }
        {
            bytes32[] memory arr1 = new bytes32[](1);
            bytes32[] memory arr2 = new bytes32[](1);
            arr1[0] = bytes32(uint256(1));

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(
                bytes(string.concat(errorMessage, ": [", vm.toString(arr1[0]), "] != [", vm.toString(arr2[0]), "]"))
            );
            vm.assertEq(arr1, arr2, errorMessage);

            vm._expectCheatcodeRevert(
                bytes(string.concat("assertion failed: [", vm.toString(arr2[0]), "] == [", vm.toString(arr2[0]), "]"))
            );
            vm.assertNotEq(arr2, arr2);
        }
        {
            string[] memory arr1 = new string[](1);
            string[] memory arr2 = new string[](3);

            arr1[0] = "foo";
            arr2[2] = "bar";

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes("assertion failed: [foo] != [, , bar]"));
            vm.assertEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": [foo] == [foo]")));
            vm.assertNotEq(arr1, arr1, errorMessage);
        }
        {
            bytes[] memory arr1 = new bytes[](1);
            bytes[] memory arr2 = new bytes[](2);

            arr1[0] = hex"1111";
            arr2[1] = hex"1234";

            vm.assertEq(arr1, arr1);
            vm.assertEq(arr2, arr2);
            vm.assertNotEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes("assertion failed: [0x1111] != [0x, 0x1234]"));
            vm.assertEq(arr1, arr2);

            vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": [0x1111] == [0x1111]")));
            vm.assertNotEq(arr1, arr1, errorMessage);
        }
    }

    function testAssertBool() public {
        vm.assertTrue(true);
        vm.assertFalse(false);

        vm._expectCheatcodeRevert(bytes("assertion failed"));
        vm.assertTrue(false);

        vm._expectCheatcodeRevert(bytes(errorMessage));
        vm.assertTrue(false, errorMessage);

        vm._expectCheatcodeRevert(bytes("assertion failed"));
        vm.assertFalse(true);

        vm._expectCheatcodeRevert(bytes(errorMessage));
        vm.assertFalse(true, errorMessage);
    }

    function testAssertApproxEqRel() public {
        vm._expectCheatcodeRevert(bytes("assertion failed: overflow in delta calculation"));
        vm.assertApproxEqRel(type(int256).min, type(int256).max, 0);

        vm._expectCheatcodeRevert(
            bytes(string.concat(errorMessage, ": 1 !~= 0 (max delta: 0.0000000000000000%, real delta: undefined)"))
        );
        vm.assertApproxEqRel(int256(1), int256(0), 0, errorMessage);

        vm._expectCheatcodeRevert(bytes(string.concat(errorMessage, ": overflow in delta calculation")));
        vm.assertApproxEqRel(uint256(0), type(uint256).max, 0, errorMessage);

        vm._expectCheatcodeRevert(
            bytes("assertion failed: 1 !~= 0 (max delta: 0.0000000000000000%, real delta: undefined)")
        );
        vm.assertApproxEqRel(uint256(1), uint256(0), uint256(0));

        vm.assertApproxEqRel(uint256(0), uint256(0), uint256(0));
    }
}
