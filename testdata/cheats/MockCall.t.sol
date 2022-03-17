// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Mock {
    function numberA() public pure returns (uint256) {
        return 1;
    }

    function numberB() public pure returns (uint256) {
        return 2;
    }

    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}

contract NestedMock {
    Mock private inner;

    constructor(Mock _inner) {
        inner = _inner;
    }

    function sum() public view returns (uint256) {
        return inner.numberA() + inner.numberB();
    }
}

contract MockCallTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testMockGetters() public {
        Mock target = new Mock();

        // pre-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);

        cheats.mockCall(
            address(target),
            abi.encodeWithSelector(target.numberB.selector),
            abi.encode(10)
        );

        // post-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);
    }

    function testMockNested() public {
        Mock inner = new Mock();
        NestedMock target = new NestedMock(inner);

        // pre-mock
        assertEq(target.sum(), 3);

        cheats.mockCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector),
            abi.encode(9)
        );

        // post-mock
        assertEq(target.sum(), 10);
    }

    function testMockSelector() public {
        Mock target = new Mock();
        assertEq(target.add(5, 5), 10);

        cheats.mockCall(
            address(target),
            abi.encodeWithSelector(target.add.selector),
            abi.encode(11)
        );

        assertEq(target.add(5, 5), 11);
    }

    function testMockCalldata() public {
        Mock target = new Mock();
        assertEq(target.add(5, 5), 10);
        assertEq(target.add(6, 4), 10);

        cheats.mockCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 5, 5),
            abi.encode(11)
        );

        assertEq(target.add(5, 5), 11);
        assertEq(target.add(6, 4), 10);
    }

    function testClearMockedCalls() public {
        Mock target = new Mock();

        cheats.mockCall(
            address(target),
            abi.encodeWithSelector(target.numberB.selector),
            abi.encode(10)
        );

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);

        cheats.clearMockedCalls();

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);
    }
}
