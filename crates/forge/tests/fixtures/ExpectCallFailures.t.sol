// Note Used in forge-cli tests to assert failures.
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";

contract Contract {
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
}

contract NestedContract {
    Contract private inner;

    constructor(Contract _inner) {
        inner = _inner;
    }

    function sum() public view returns (uint256) {
        return inner.numberA() + inner.numberB();
    }

    function forwardPay() public payable returns (uint256) {
        return inner.pay{gas: 50_000, value: 1}(1);
    }

    function addHardGasLimit() public view returns (uint256) {
        return inner.add{gas: 50_000}(1, 1);
    }

    function hello() public pure returns (string memory) {
        return "hi";
    }

    function sumInPlace(uint256 a, uint256 b) public view returns (uint256) {
        return a + b + 42;
    }
}

contract ExpectCallFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function exposed_callTargetNTimes(Contract target, uint256 a, uint256 b, uint256 times) public pure {
        for (uint256 i = 0; i < times; i++) {
            target.add(a, b);
        }
    }

    function exposed_failExpectInnerCall(NestedContract target) public {
        // this function does not call inner
        target.hello();
    }

    function testShouldFailExpectMultipleCallsWithDataAdditive() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        // Not enough calls to satisfy the additive expectCall, which expects 3 calls.
        this.exposed_callTargetNTimes(target, 1, 2, 2);
    }

    function testShouldFailExpectCallWithData() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 1);
        this.exposed_callTargetNTimes(target, 3, 3, 1);
    }

    function testShouldFailExpectInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        vm.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector));

        this.exposed_failExpectInnerCall(target);
    }

    function testShouldFailExpectSelectorCall() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector));
    }

    function testShouldFailExpectCallWithMoreParameters() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 3, 3, 3));
        target.add(3, 3);
        this.exposed_callTargetNTimes(target, 3, 3, 1);
    }

    function testShouldFailExpectCallValue() public {
        Contract target = new Contract();
        vm.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2));
    }

    function exposed_addHardGasLimit(NestedContract target) public {
        target.addHardGasLimit();
    }

    function testShouldFailExpectCallWithNoValueAndWrongGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);
        vm.expectCall(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1));
        this.exposed_addHardGasLimit(target);
    }

    function testShouldFailExpectCallWithNoValueAndWrongMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);
        vm.expectCallMinGas(address(inner), 0, 50_001, abi.encodeWithSelector(inner.add.selector, 1, 1));
        this.exposed_addHardGasLimit(target);
    }

    /// Ensure that you cannot use expectCall with an expectRevert.
    function testShouldFailExpectCallWithRevertDisallowed() public {
        Contract target = new Contract();
        vm.expectRevert();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector));
        this.exposed_callTargetNTimes(target, 5, 5, 1);
    }
}

contract ExpectCallCountFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testShouldFailExpectCallCountWithWrongCount() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        target.add(1, 2);
    }

    function testShouldFailExpectCountInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        vm.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector), 1);

        // this function does not call inner
        target.hello();
    }

    function exposed_pay(Contract target, uint256 value, uint256 amount) public payable {
        target.pay{value: value}(amount);
    }

    function testShouldFailExpectCallCountValue() public {
        Contract target = new Contract();
        vm.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2), 1);
        this.exposed_pay{value: 2}(target, 2, 2);
    }

    function exposed_addHardGasLimit(NestedContract target, uint256 times) public {
        for (uint256 i = 0; i < times; i++) {
            target.addHardGasLimit();
        }
    }

    function testShouldFailExpectCallCountWithNoValueAndWrongGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);
        vm.expectCall(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1), 2);
        this.exposed_addHardGasLimit(target, 2);
    }

    function testShouldFailExpectCallCountWithNoValueAndWrongMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);
        vm.expectCallMinGas(address(inner), 0, 50_001, abi.encodeWithSelector(inner.add.selector, 1, 1), 1);
        this.exposed_addHardGasLimit(target, 1);
    }
}

contract ExpectCallMixedFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function exposed_callTargetNTimes(Contract target, uint256 a, uint256 b, uint256 times) public {
        for (uint256 i = 0; i < times; i++) {
            target.add(1, 2);
        }
    }

    function testShouldFailOverrideNoCountWithCount() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        // You should not be able to overwrite a expectCall that had no count with some count.
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        this.exposed_callTargetNTimes(target, 1, 2, 2);
    }

    function testShouldFailOverrideCountWithCount() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        // You should not be able to overwrite a expectCall that had a count with some count.
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 1);
        target.add(1, 2);
        target.add(1, 2);
    }

    function testShouldFailOverrideCountWithNoCount() public {
        Contract target = new Contract();
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        // You should not be able to overwrite a expectCall that had a count with no count.
        vm.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
    }
}
