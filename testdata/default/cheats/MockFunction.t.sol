// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IMockFunctionContract {
    function a() external view returns (uint256);
    function mocked_function() external;
    function mocked_args_function(uint256 x) external;
}

contract MockFunctionContract is IMockFunctionContract {
    uint256 public a;

    function mocked_function() public {
        a = 321;
    }

    function mocked_args_function(uint256 x) public {
        a = 321 + x;
    }
}

contract ModelMockFunctionContract is IMockFunctionContract {
    uint256 public a;

    function mocked_function() public {
        a = 123;
    }

    function mocked_args_function(uint256 x) public {
        a = 123 + x;
    }
}

contract Proxy {
    address immutable impl;

    constructor(address impl_) {
        impl = impl_;
    }

    fallback() external {
        _delegate(impl);
    }

    // code from https://github.com/OpenZeppelin/openzeppelin-contracts/blob/239795bea728c8dca4deb6c66856dd58a6991112/contracts/proxy/Proxy.sol#L22-L45
    function _delegate(address implementation) internal virtual {
        assembly {
            // Copy msg.data. We take full control of memory in this inline assembly
            // block because it will not return to Solidity code. We overwrite the
            // Solidity scratch pad at memory position 0.
            calldatacopy(0x00, 0x00, calldatasize())

            // Call the implementation.
            // out and outsize are 0 because we don't know the size yet.
            let result := delegatecall(gas(), implementation, 0x00, calldatasize(), 0x00, 0x00)

            // Copy the returned data.
            returndatacopy(0x00, 0x00, returndatasize())

            switch result
            // delegatecall returns 0 on error.
            case 0 {
                revert(0x00, returndatasize())
            }
            default {
                return(0x00, returndatasize())
            }
        }
    }
}

contract MockFunctionTest is Test {
    MockFunctionContract my_contract;
    ModelMockFunctionContract model_contract;
    IMockFunctionContract my_proxy;

    function setUp() public {
        my_contract = new MockFunctionContract();
        model_contract = new ModelMockFunctionContract();
        my_proxy = IMockFunctionContract(address(new Proxy(address(my_contract))));
    }

    function test_mock_function() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_function.selector)
        );
        my_contract.mocked_function();
        assertEq(my_contract.a(), 123);
    }

    function test_mock_function_concrete_args() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector, 456)
        );
        my_contract.mocked_args_function(456);
        assertEq(my_contract.a(), 123 + 456);
        my_contract.mocked_args_function(567);
        assertEq(my_contract.a(), 321 + 567);
    }

    function test_mock_function_all_args() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector)
        );
        my_contract.mocked_args_function(678);
        assertEq(my_contract.a(), 123 + 678);
        my_contract.mocked_args_function(789);
        assertEq(my_contract.a(), 123 + 789);
    }

    function test_mock_function_via_proxy() public {
        vm.mockFunction(
            address(my_proxy),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_function.selector)
        );
        my_proxy.mocked_function();
        assertEq(my_proxy.a(), 123, "mocked function should be called via proxy");

        // reset mock
        vm.mockFunction(
            address(my_proxy), address(my_proxy), abi.encodeWithSelector(MockFunctionContract.mocked_function.selector)
        );
        my_proxy.mocked_function();
        assertEq(my_proxy.a(), 321, "after reset, original function should be called");
    }

    function test_mock_function_via_proxy_concrete_args() public {
        vm.mockFunction(
            address(my_proxy),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector, 100)
        );
        my_proxy.mocked_args_function(100);
        assertEq(my_proxy.a(), 123 + 100, "mocked args function should be called via proxy");
        my_proxy.mocked_args_function(200);
        assertEq(my_proxy.a(), 321 + 200, "original args function should be called for different args");

        // reset mock
        vm.mockFunction(
            address(my_proxy),
            address(my_proxy),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector, 100)
        );
        my_proxy.mocked_args_function(100);
        assertEq(my_proxy.a(), 321 + 100, "after reset, original args function should be called");
        my_proxy.mocked_args_function(200);
        assertEq(my_proxy.a(), 321 + 200, "original args function should be called for different args");
    }

    function test_mock_function_via_proxy_all_args() public {
        vm.mockFunction(
            address(my_proxy),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector)
        );
        my_proxy.mocked_args_function(300);
        assertEq(my_proxy.a(), 123 + 300, "mocked args function should be called via proxy");
        my_proxy.mocked_args_function(400);
        assertEq(my_proxy.a(), 123 + 400, "mocked args function should be called via proxy");

        // reset mock
        vm.mockFunction(
            address(my_proxy),
            address(my_proxy),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector)
        );
        my_proxy.mocked_args_function(300);
        assertEq(my_proxy.a(), 321 + 300, "after reset, original args function should be called");
        my_proxy.mocked_args_function(400);
        assertEq(my_proxy.a(), 321 + 400, "after reset, original args function should be called");
    }

    function test_mock_function_via_impl() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_function.selector)
        );
        my_proxy.mocked_function();
        assertEq(my_proxy.a(), 123, "mocked function should be called via impl address");

        // reset mock
        vm.mockFunction(
            address(my_contract),
            address(my_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_function.selector)
        );
        my_proxy.mocked_function();
        assertEq(my_proxy.a(), 321, "after reset, original function should be called");
    }

    function test_mock_function_via_impl_concrete_args() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector, 200)
        );
        my_proxy.mocked_args_function(200);
        assertEq(my_proxy.a(), 123 + 200, "mocked args function should be called via impl address");
        my_proxy.mocked_args_function(300);
        assertEq(my_proxy.a(), 321 + 300, "original args function should be called for different args");

        // reset mock
        vm.mockFunction(
            address(my_contract),
            address(my_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector, 200)
        );
        my_proxy.mocked_args_function(200);
        assertEq(my_proxy.a(), 321 + 200, "after reset, original args function should be called");
        my_proxy.mocked_args_function(300);
        assertEq(my_proxy.a(), 321 + 300, "original args function should be called for different args");
    }

    function test_mock_function_via_impl_all_args() public {
        vm.mockFunction(
            address(my_contract),
            address(model_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector)
        );
        my_proxy.mocked_args_function(400);
        assertEq(my_proxy.a(), 123 + 400, "mocked args function should be called via impl address");
        my_proxy.mocked_args_function(500);
        assertEq(my_proxy.a(), 123 + 500, "mocked args function should be called via impl address");

        // reset mock
        vm.mockFunction(
            address(my_contract),
            address(my_contract),
            abi.encodeWithSelector(MockFunctionContract.mocked_args_function.selector)
        );
        my_proxy.mocked_args_function(400);
        assertEq(my_proxy.a(), 321 + 400, "after reset, original args function should be called");
        my_proxy.mocked_args_function(500);
        assertEq(my_proxy.a(), 321 + 500, "after reset, original args function should be called");
    }
}
