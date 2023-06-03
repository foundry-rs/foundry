// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Mock {
    uint256 state = 0;

    function numberA() public pure returns (uint256) {
        return 1;
    }

    function numberB() public pure returns (uint256) {
        return 2;
    }

    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }

    function pay(uint256 a) public payable returns (uint256) {
        return a;
    }

    function noReturnValue() public {
        // Does nothing of value, but also ensures that Solidity will 100%
        // generate an `extcodesize` check.
        state += 1;
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

        cheats.mockCall(address(target), abi.encodeWithSelector(target.numberB.selector), abi.encode(10));

        // post-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);
    }

    function testMockNested() public {
        Mock inner = new Mock();
        NestedMock target = new NestedMock(inner);

        // pre-mock
        assertEq(target.sum(), 3);

        cheats.mockCall(address(inner), abi.encodeWithSelector(inner.numberB.selector), abi.encode(9));

        // post-mock
        assertEq(target.sum(), 10);
    }

    function testMockSelector() public {
        Mock target = new Mock();
        assertEq(target.add(5, 5), 10);

        cheats.mockCall(address(target), abi.encodeWithSelector(target.add.selector), abi.encode(11));

        assertEq(target.add(5, 5), 11);
    }

    function testMockCalldata() public {
        Mock target = new Mock();
        assertEq(target.add(5, 5), 10);
        assertEq(target.add(6, 4), 10);

        cheats.mockCall(address(target), abi.encodeWithSelector(target.add.selector, 5, 5), abi.encode(11));

        assertEq(target.add(5, 5), 11);
        assertEq(target.add(6, 4), 10);
    }

    function testClearMockedCalls() public {
        Mock target = new Mock();

        cheats.mockCall(address(target), abi.encodeWithSelector(target.numberB.selector), abi.encode(10));

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);

        cheats.clearMockedCalls();

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);
    }

    function testMockCallMultiplePartialMatch() public {
        Mock mock = new Mock();

        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector), abi.encode(10));
        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector, 2), abi.encode(20));
        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector, 2, 3), abi.encode(30));

        assertEq(mock.add(1, 2), 10);
        assertEq(mock.add(2, 2), 20);
        assertEq(mock.add(2, 3), 30);
    }

    function testMockCallWithValue() public {
        Mock mock = new Mock();

        cheats.mockCall(address(mock), 10, abi.encodeWithSelector(mock.pay.selector), abi.encode(10));

        assertEq(mock.pay{value: 10}(1), 10);
        assertEq(mock.pay(1), 1);

        for (uint256 i = 0; i < 100; i++) {
            cheats.mockCall(address(mock), i, abi.encodeWithSelector(mock.pay.selector), abi.encode(i * 2));
        }

        assertEq(mock.pay(1), 0);
        assertEq(mock.pay{value: 10}(1), 20);
        assertEq(mock.pay{value: 50}(1), 100);
    }

    function testMockCallWithValueCalldataPrecedence() public {
        Mock mock = new Mock();

        cheats.mockCall(address(mock), 10, abi.encodeWithSelector(mock.pay.selector), abi.encode(10));
        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.pay.selector, 2), abi.encode(2));

        assertEq(mock.pay{value: 10}(1), 10);
        assertEq(mock.pay{value: 10}(2), 2);
        assertEq(mock.pay(2), 2);
    }

    function testMockCallEmptyAccount() public {
        Mock mock = Mock(address(100));

        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector), abi.encode(10));
        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.noReturnValue.selector), abi.encode());

        assertEq(mock.add(1, 2), 10);
        mock.noReturnValue();
    }
}

contract MockCallRevertTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    error TestError(bytes msg);

    bytes constant ERROR_MESSAGE = "ERROR_MESSAGE";

    function testMockGettersRevert() public {
        Mock target = new Mock();

        // pre-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);

        cheats.mockCallRevert(address(target), abi.encodeWithSelector(target.numberB.selector), ERROR_MESSAGE);

        // post-mock
        assertEq(target.numberA(), 1);
        cheats.expectRevert();
        target.numberB();
    }

    function testMockRevertWithCustomError() public {
        Mock target = new Mock();

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);

        bytes memory customError = abi.encodeWithSelector(TestError.selector, ERROR_MESSAGE);

        cheats.mockCallRevert(address(target), abi.encodeWithSelector(target.numberB.selector), customError);

        assertEq(target.numberA(), 1);
        cheats.expectRevert(customError);
        target.numberB();
    }

    function testMockNestedRevert() public {
        Mock inner = new Mock();
        NestedMock target = new NestedMock(inner);

        assertEq(target.sum(), 3);

        cheats.mockCallRevert(address(inner), abi.encodeWithSelector(inner.numberB.selector), ERROR_MESSAGE);

        cheats.expectRevert(ERROR_MESSAGE);
        target.sum();
    }

    function testMockCalldataRevert() public {
        Mock target = new Mock();
        assertEq(target.add(5, 5), 10);
        assertEq(target.add(6, 4), 10);

        cheats.mockCallRevert(address(target), abi.encodeWithSelector(target.add.selector, 5, 5), ERROR_MESSAGE);

        assertEq(target.add(6, 4), 10);

        cheats.expectRevert(ERROR_MESSAGE);
        target.add(5, 5);
    }

    function testClearMockRevertedCalls() public {
        Mock target = new Mock();

        cheats.mockCallRevert(address(target), abi.encodeWithSelector(target.numberB.selector), ERROR_MESSAGE);

        cheats.clearMockedCalls();

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);
    }

    function testMockCallRevertPartialMatch() public {
        Mock mock = new Mock();

        cheats.mockCallRevert(address(mock), abi.encodeWithSelector(mock.add.selector, 2), ERROR_MESSAGE);

        assertEq(mock.add(1, 2), 3);

        cheats.expectRevert(ERROR_MESSAGE);
        mock.add(2, 3);
    }

    function testMockCallRevertWithValue() public {
        Mock mock = new Mock();

        cheats.mockCallRevert(address(mock), 10, abi.encodeWithSelector(mock.pay.selector), ERROR_MESSAGE);

        assertEq(mock.pay(1), 1);
        assertEq(mock.pay(2), 2);

        cheats.expectRevert(ERROR_MESSAGE);
        mock.pay{value: 10}(1);
    }

    function testMockCallResetsMockCallRevert() public {
        Mock mock = new Mock();

        cheats.mockCallRevert(address(mock), abi.encodeWithSelector(mock.add.selector), ERROR_MESSAGE);

        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector), abi.encode(5));
        assertEq(mock.add(2, 3), 5);
    }

    function testMockCallRevertResetsMockCall() public {
        Mock mock = new Mock();

        cheats.mockCall(address(mock), abi.encodeWithSelector(mock.add.selector), abi.encode(5));
        assertEq(mock.add(2, 3), 5);

        cheats.mockCallRevert(address(mock), abi.encodeWithSelector(mock.add.selector), ERROR_MESSAGE);

        cheats.expectRevert(ERROR_MESSAGE);
        mock.add(2, 3);
    }

    function testMockCallRevertWithCall() public {
        Mock mock = new Mock();

        bytes memory customError = abi.encodeWithSelector(TestError.selector, ERROR_MESSAGE);

        cheats.mockCallRevert(address(mock), abi.encodeWithSelector(mock.add.selector), customError);

        (bool success, bytes memory data) = address(mock).call(abi.encodeWithSelector(Mock.add.selector, 2, 3));
        assertEq(success, false);
        assertEq(data, customError);
    }

    function testMockCallEmptyAccount() public {
        Mock mock = Mock(address(100));

        cheats.mockCallRevert(address(mock), abi.encodeWithSelector(mock.add.selector), ERROR_MESSAGE);

        cheats.expectRevert(ERROR_MESSAGE);
        mock.add(1, 2);
    }
}
