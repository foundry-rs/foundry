// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

//FIXME(Odysseas): Add Credible-std as a git submodule

interface PhEvm {
    //Forks to the state prior to the assertion triggering transaction.
    function forkPreState() external;

    //Forks to the state after the assertion triggering transaction.
    function forkPostState() external;
}

/// @notice The Credible contract
contract Credible {
    //Precompile address -
    PhEvm ph = PhEvm(address(uint160(uint256(keccak256("Kim Jong Un Sucks")))));
}

/// @notice Assertion interface for the PhEvm precompile
abstract contract Assertion is Credible {
    /// @notice The type of state change that triggers the assertion
    enum TriggerType {
        /// @notice The assertion is triggered by a storage change
        STORAGE,
        /// @notice The assertion is triggered by a transfer of ether
        ETHER,
        /// @notice The assertion is triggered by both a storage change and a transfer of ether
        BOTH
    }

    /// @notice A struct that contains the type of state change and the function selector of the assertion function
    struct Trigger {
        /// @notice The type of state change that triggers the assertion
        TriggerType triggerType;
        /// @notice The assertion function selector
        bytes4 fnSelector;
    }

    /// @notice Returns all the triggers for the assertion
    /// @return An array of Trigger structs
    function fnSelectors() external pure virtual returns (Trigger[] memory);
}

contract MockAssertion is Assertion {
    function fnSelectors() external pure override returns (Trigger[] memory) {
        Trigger[] memory triggers = new Trigger[](1);
        triggers[0] = Trigger({
            triggerType: TriggerType.STORAGE,
            fnSelector: this.assertionTrue.selector
        });
        return triggers;
    }

    function assertionTrue() external returns (bool) {
        return true;
    }
}

contract MockContract {
}

contract CredibleTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    address assertionAdopter;
    bytes[] assertions;

    struct SimpleTransaction {
        address from;
        address to;
        uint256 value;
        bytes data;
    }

    function setUp() public {
        assertionAdopter = address(new MockContract());
    }

    function testAssertionPass() public {
        SimpleTransaction memory transaction = SimpleTransaction({
            from: address(0xbeef),
            to: address(0),
            value: 0,
            data: bytes("")
        });
        bytes memory assertionBytecode = abi.encodePacked(type(MockAssertion).creationCode);
        assertions.push(assertionBytecode);
        vm.assertionEx(abi.encode(transaction), assertionAdopter, assertions);
    }
}
