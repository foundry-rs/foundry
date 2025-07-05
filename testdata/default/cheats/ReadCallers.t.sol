// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Target {
    function consumeNewCaller() external {}
}

contract ReadCallersTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testReadCallersWithNoActivePrankOrBroadcast() public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    // Prank Tests
    function testReadCallersWithActivePrankForMsgSender(address sender) public {
        VM.prank(sender);
        address expectedTxOrigin = tx.origin;

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActivePrankForMsgSenderAndTxOrigin(address sender, address origin) public {
        VM.prank(sender, origin);

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, origin);
    }

    function testReadCallersAfterConsumingMsgSenderPrank(address sender) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        VM.prank(sender);

        target.consumeNewCaller();
        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterConsumingMsgSenderAndTxOriginPrank(address sender, address origin) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        VM.prank(sender, origin);

        target.consumeNewCaller();
        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActiveRecurrentMsgSenderPrank(address sender) public {
        address expectedTxOrigin = tx.origin;
        Target target = new Target();
        VM.startPrank(sender);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

            assertEq(uint256(mode), uint256(Vm.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, expectedTxOrigin);
        }
    }

    function testReadCallersWithActiveRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        Target target = new Target();
        VM.startPrank(sender, origin);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

            assertEq(uint256(mode), uint256(Vm.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderPrank(address sender) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        VM.startPrank(sender);

        VM.stopPrank();

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        VM.startPrank(sender, origin);

        VM.stopPrank();

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    // Broadcast Tests
    function testReadCallersWithActiveBroadcast(address sender) public {
        VM.broadcast(sender);

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.Broadcast));
        assertEq(newSender, sender);
        assertEq(newOrigin, sender);
    }

    function testReadCallersAfterConsumingBroadcast(address sender) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        VM.broadcast(sender);

        target.consumeNewCaller();
        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActiveRecurrentBroadcast(address sender) public {
        Target target = new Target();
        VM.startBroadcast(sender);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

            assertEq(uint256(mode), uint256(Vm.CallerMode.RecurrentBroadcast));
            assertEq(newSender, sender);
            assertEq(newOrigin, sender);
        }
    }

    function testReadCallersAfterStoppingRecurrentBroadcast(address sender) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        VM.startBroadcast(sender);

        VM.stopBroadcast();

        (Vm.CallerMode mode, address newSender, address newOrigin) = VM.readCallers();

        assertEq(uint256(mode), uint256(Vm.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }
}
