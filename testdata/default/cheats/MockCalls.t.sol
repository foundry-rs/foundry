// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract MockCallsTest is Test {
    function testMockCallsLastShouldPersist() public {
        address mockUser = vm.addr(vm.randomUint());
        address mockErc20 = vm.addr(vm.randomUint());
        bytes memory data = abi.encodeWithSignature("balanceOf(address)", mockUser);
        bytes[] memory mocks = new bytes[](2);
        mocks[0] = abi.encode(2 ether);
        mocks[1] = abi.encode(7.219 ether);
        vm.mockCalls(mockErc20, data, mocks);
        (, bytes memory ret1) = mockErc20.call(data);
        assertEq(abi.decode(ret1, (uint256)), 2 ether);
        (, bytes memory ret2) = mockErc20.call(data);
        assertEq(abi.decode(ret2, (uint256)), 7.219 ether);
        (, bytes memory ret3) = mockErc20.call(data);
        assertEq(abi.decode(ret3, (uint256)), 7.219 ether);
    }

    function testMockCallsWithValue() public {
        address mockUser = vm.addr(vm.randomUint());
        address mockErc20 = vm.addr(vm.randomUint());
        bytes memory data = abi.encodeWithSignature("balanceOf(address)", mockUser);
        bytes[] memory mocks = new bytes[](3);
        mocks[0] = abi.encode(2 ether);
        mocks[1] = abi.encode(1 ether);
        mocks[2] = abi.encode(6.423 ether);
        vm.mockCalls(mockErc20, 1 ether, data, mocks);
        (, bytes memory ret1) = mockErc20.call{value: 1 ether}(data);
        assertEq(abi.decode(ret1, (uint256)), 2 ether);
        (, bytes memory ret2) = mockErc20.call{value: 1 ether}(data);
        assertEq(abi.decode(ret2, (uint256)), 1 ether);
        (, bytes memory ret3) = mockErc20.call{value: 1 ether}(data);
        assertEq(abi.decode(ret3, (uint256)), 6.423 ether);
    }

    function testMockCalls() public {
        address mockUser = vm.addr(vm.randomUint());
        address mockErc20 = vm.addr(vm.randomUint());
        bytes memory data = abi.encodeWithSignature("balanceOf(address)", mockUser);
        bytes[] memory mocks = new bytes[](3);
        mocks[0] = abi.encode(2 ether);
        mocks[1] = abi.encode(1 ether);
        mocks[2] = abi.encode(6.423 ether);
        vm.mockCalls(mockErc20, data, mocks);
        (, bytes memory ret1) = mockErc20.call(data);
        assertEq(abi.decode(ret1, (uint256)), 2 ether);
        (, bytes memory ret2) = mockErc20.call(data);
        assertEq(abi.decode(ret2, (uint256)), 1 ether);
        (, bytes memory ret3) = mockErc20.call(data);
        assertEq(abi.decode(ret3, (uint256)), 6.423 ether);
    }
}
