// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract AttachDelegationTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 pk = 77814517325470205911140941194401928579557062014761831930645393041380819009408;
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

    function testCreateDelegation(address implementation, uint64 nonce) public {
        bytes32 delegation = vm.createDelegation(implementation, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.signDelegation(delegation, pk);

        vm.attachDelegation(implementation, nonce, v, r, s);
        vm.broadcast();
        // @todo - assert a transaction's msg.sender is implementation address
    }
}
