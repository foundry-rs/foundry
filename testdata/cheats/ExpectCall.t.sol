// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

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
}

contract ExpectCallTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testExpectCallWithData() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
    }

    function testExpectMultipleCallsWithData() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
    }

    function testExpectMultipleCallsWithDataAdditive() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
    }

    function testExpectMultipleCallsWithDataAdditiveLowerBound() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
        target.add(1, 2);
    }

    function testFailExpectMultipleCallsWithDataAdditive() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        // Not enough calls to satisfy the additive expectCall, which expects 3 calls.
        target.add(1, 2);
        target.add(1, 2);
    }

    function testFailExpectCallWithData() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(3, 3);
    }

    function testExpectInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector));
        target.sum();
    }

    function testFailExpectInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector));

        // this function does not call inner
        target.hello();
    }

    function testExpectSelectorCall() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector));
        target.add(5, 5);
    }

    function testFailExpectSelectorCall() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector));
    }

    function testFailExpectCallWithMoreParameters() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 3, 3, 3));
        target.add(3, 3);
    }

    function testExpectCallWithValue() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2));
        target.pay{value: 1}(2);
    }

    function testFailExpectCallValue() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2));
    }

    function testExpectCallWithValueWithoutParameters() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 3, abi.encodeWithSelector(target.pay.selector));
        target.pay{value: 3}(100);
    }

    function testExpectCallWithValueAndGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 1, 50_000, abi.encodeWithSelector(inner.pay.selector, 1));
        target.forwardPay{value: 1}();
    }

    function testExpectCallWithNoValueAndGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 0, 50_000, abi.encodeWithSelector(inner.add.selector, 1, 1));
        target.addHardGasLimit();
    }

    function testFailExpectCallWithNoValueAndWrongGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1));
        target.addHardGasLimit();
    }

    function testExpectCallWithValueAndMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 1, 50_000, abi.encodeWithSelector(inner.pay.selector, 1));
        target.forwardPay{value: 1}();
    }

    function testExpectCallWithNoValueAndMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1));
        target.addHardGasLimit();
    }

    function testFailExpectCallWithNoValueAndWrongMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 0, 50_001, abi.encodeWithSelector(inner.add.selector, 1, 1));
        target.addHardGasLimit();
    }
}

contract ExpectCallCountTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testExpectCallCountWithData() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 3);
        target.add(1, 2);
        target.add(1, 2);
        target.add(1, 2);
    }

    function testExpectZeroCallCountAssert() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 0);
        target.add(3, 3);
    }

    function testFailExpectCallCountWithWrongCount() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        target.add(1, 2);
    }

    function testExpectCountInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector), 1);
        target.sum();
    }

    function testFailExpectCountInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector), 1);

        // this function does not call inner
        target.hello();
    }

    function testExpectCountInnerAndOuterCalls() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), abi.encodeWithSelector(inner.numberB.selector), 2);
        inner.numberB();
        target.sum();
    }

    function testExpectCallCountWithValue() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2), 1);
        target.pay{value: 1}(2);
    }

    function testExpectZeroCallCountValue() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2), 0);
        target.pay{value: 2}(2);
    }

    function testFailExpectCallCountValue() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 1, abi.encodeWithSelector(target.pay.selector, 2), 1);
        target.pay{value: 2}(2);
    }

    function testExpectCallCountWithValueWithoutParameters() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), 3, abi.encodeWithSelector(target.pay.selector), 3);
        target.pay{value: 3}(100);
        target.pay{value: 3}(100);
        target.pay{value: 3}(100);
    }

    function testExpectCallCountWithValueAndGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 1, 50_000, abi.encodeWithSelector(inner.pay.selector, 1), 2);
        target.forwardPay{value: 1}();
        target.forwardPay{value: 1}();
    }

    function testExpectCallCountWithNoValueAndGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 0, 50_000, abi.encodeWithSelector(inner.add.selector, 1, 1), 1);
        target.addHardGasLimit();
    }

    function testExpectZeroCallCountWithNoValueAndWrongGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1), 0);
        target.addHardGasLimit();
    }

    function testFailExpectCallCountWithNoValueAndWrongGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1), 2);
        target.addHardGasLimit();
        target.addHardGasLimit();
    }

    function testExpectCallCountWithValueAndMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 1, 50_000, abi.encodeWithSelector(inner.pay.selector, 1), 1);
        target.forwardPay{value: 1}();
    }

    function testExpectCallCountWithNoValueAndMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 0, 25_000, abi.encodeWithSelector(inner.add.selector, 1, 1), 2);
        target.addHardGasLimit();
        target.addHardGasLimit();
    }

    function testExpectCallZeroCountWithNoValueAndWrongMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 0, 50_001, abi.encodeWithSelector(inner.add.selector, 1, 1), 0);
        target.addHardGasLimit();
    }

    function testFailExpectCallCountWithNoValueAndWrongMinGas() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCallMinGas(address(inner), 0, 50_001, abi.encodeWithSelector(inner.add.selector, 1, 1), 1);
        target.addHardGasLimit();
    }
}

contract ExpectCallMixedTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testFailOverrideNoCountWithCount() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        // You should not be able to overwrite a expectCall that had no count with some count.
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        target.add(1, 2);
        target.add(1, 2);
    }

    function testFailOverrideCountWithCount() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        // You should not be able to overwrite a expectCall that had a count with some count.
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 1);
        target.add(1, 2);
        target.add(1, 2);
    }

    function testFailOverrideCountWithNoCount() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        // You should not be able to overwrite a expectCall that had a count with no count.
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
    }

    function testExpectMatchPartialAndFull() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector), 2);
        // Even if a partial match is speciifed, you should still be able to look for full matches
        // as one does not override the other.
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2));
        target.add(1, 2);
        target.add(1, 2);
    }

    function testExpectMatchPartialAndFullFlipped() public {
        Contract target = new Contract();
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector));
        // Even if a partial match is speciifed, you should still be able to look for full matches
        // as one does not override the other.
        cheats.expectCall(address(target), abi.encodeWithSelector(target.add.selector, 1, 2), 2);
        target.add(1, 2);
        target.add(1, 2);
    }
}
