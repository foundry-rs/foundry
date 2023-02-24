// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

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
