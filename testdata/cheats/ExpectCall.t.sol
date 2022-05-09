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
}

contract NestedContract {
    Contract private inner;

    constructor(Contract _inner) {
        inner = _inner;
    }

    function sum() public view returns (uint256) {
        return inner.numberA() + inner.numberB();
    }

    function hello() public pure returns (string memory) {
        return "hi";
    }
}

contract ExpectCallTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testExpectCallWithData() public {
        Contract target = new Contract();
        cheats.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 1, 2)
        );
        target.add(1, 2);
    }

    function testFailExpectCallWithData() public {
        Contract target = new Contract();
        cheats.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 1, 2)
        );
        target.add(3, 3);
    }

    function testExpectInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector)
        );
        target.sum();
    }

    function testFailExpectInnerCall() public {
        Contract inner = new Contract();
        NestedContract target = new NestedContract(inner);

        cheats.expectCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector)
        );

        // this function does not call inner
        target.hello();
    }

    function testExpectSelectorCall() public {
        Contract target = new Contract();
        cheats.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector)
        );
        target.add(5, 5);
    }

    function testFailExpectSelectorCall() public {
        Contract target = new Contract();
        cheats.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector)
        );
    }

    function testFailExpectCallWithMoreParameters() public {
        Contract target = new Contract();
        cheats.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 3, 3, 3)
        );
        target.add(3, 3);
    }
}
