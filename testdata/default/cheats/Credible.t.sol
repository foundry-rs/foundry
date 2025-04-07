// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import {Assertion} from "credible-std/Assertion.sol";

contract MockAssertion is Assertion {
    MockContract mockContract;

    constructor(address mockContract_) {
        mockContract = MockContract(mockContract_);
    }

    function fnSelectors() external pure override returns (bytes4[] memory selectors) {
        selectors = new bytes4[](1);
        selectors[0] = this.assertIsOne.selector;
    }

    function assertIsOne() external view returns (bool) {
        return mockContract.value() == 1;
    }
}

contract MockContract {
    uint256 public value = 1;

    function increment() public {
        value++;
    }
}

contract CredibleTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    address assertionAdopter;

    address constant caller = address(0xdead);

    struct SimpleTransaction {
        address from;
        address to;
        uint256 value;
        bytes data;
    }

    function setUp() public {
        assertionAdopter = address(new MockContract());
        vm.deal(caller, 1 ether);
    }

    function testAssertionPass() public {
        SimpleTransaction memory transaction = SimpleTransaction({
            from: address(caller),
            to: address(assertionAdopter),
            value: 0,
            data: abi.encodeWithSelector(MockContract.increment.selector)
        });

        emit log_address(assertionAdopter);

        bytes memory assertion = abi.encodePacked(type(MockAssertion).creationCode, abi.encode(assertionAdopter));

        vm.assertionEx(abi.encode(transaction), assertionAdopter, assertion, "MockAssertion");
        assertTrue(MockContract(assertionAdopter).value() == 1);

        MockContract(assertionAdopter).increment();
        assertTrue(MockContract(assertionAdopter).value() == 2);

        vm.assertionEx(abi.encode(transaction), assertionAdopter, assertion, "MockAssertion");
    }
}
