// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

contract Counter {
    uint256 public counter;

    function increment() public {
        counter++;
    }
}

contract AttachBlobTest is Test {
    uint256 bobPk = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address bob = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function testAttachBlob() public {
        bytes memory blob = abi.encode("Blob the Builder");
        vm.attachBlob(blob);

        vm.broadcast(bobPk);
        counter.increment();
    }

    function testAttachBlobWithCreateTx() public {
        bytes memory blob = abi.encode("Blob the Builder");
        vm.attachBlob(blob);

        vm.broadcast(bobPk);
        new Counter();

        // blob is attached with this tx instead
        vm.broadcast(bobPk);
        counter.increment();
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testRevertAttachBlobWithDelegation() public {
        bytes memory blob = abi.encode("Blob the Builder");
        vm.attachBlob(blob);
        vm.signAndAttachDelegation(address(0), bobPk);

        vm.broadcast(bobPk);
        vm.expectRevert("both delegation and blob are active; `attachBlob` and `attachDelegation` are not compatible");
        counter.increment();
    }
}
