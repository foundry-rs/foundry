// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Target {
    function consumeNewCaller() external {}
}

contract ReadCallersTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testReadCallersWithNoActivePrankOrBroadcast() public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    // Prank Tests
    function testReadCallersWithActivePrankForMsgSender(address sender) public {
        cheats.prank(sender);
        address expectedTxOrigin = tx.origin;

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActivePrankForMsgSenderAndTxOrigin(address sender, address origin) public {
        cheats.prank(sender, origin);

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, origin);
    }

    function testReadCallersAfterConsumingMsgSenderPrank(address sender) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender);

        target.consumeNewCaller();
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterConsumingMsgSenderAndTxOriginPrank(address sender, address origin) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender, origin);

        target.consumeNewCaller();
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActiveRecurrentMsgSenderPrank(address sender) public {
        address expectedTxOrigin = tx.origin;
        Target target = new Target();
        cheats.startPrank(sender);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

            assertEq(uint256(mode), uint256(Cheats.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, expectedTxOrigin);
        }
    }

    function testReadCallersWithActiveRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        Target target = new Target();
        cheats.startPrank(sender, origin);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

            assertEq(uint256(mode), uint256(Cheats.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderPrank(address sender) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender);

        cheats.stopPrank();

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender, origin);

        cheats.stopPrank();

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    // Broadcast Tests
    function testReadCallersWithActiveBroadcast(address sender) public {
        cheats.broadcast(sender);

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.Broadcast));
        assertEq(newSender, sender);
        assertEq(newOrigin, sender);
    }

    function testReadCallersAfterConsumingBroadcast(address sender) public {
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.broadcast(sender);

        target.consumeNewCaller();
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActiveRecurrentBroadcast(address sender) public {
        Target target = new Target();
        cheats.startBroadcast(sender);

        for (uint256 i = 0; i < 5; i++) {
            target.consumeNewCaller();
            (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

            assertEq(uint256(mode), uint256(Cheats.CallerMode.RecurrentBroadcast));
            assertEq(newSender, sender);
            assertEq(newOrigin, sender);
        }
    }

    function testReadCallersAfterStoppingRecurrentBroadcast(address sender) public {
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startBroadcast(sender);

        cheats.stopBroadcast();

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }
}
