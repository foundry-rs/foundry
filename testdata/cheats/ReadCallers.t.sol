// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Target {
    function consumePrank() external {}
}

contract ReadCallersTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testReadCallersWithNoActivePrankOrBroadcast() public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        // Act
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    // Prank Tests
    function testReadCallersWithActivePrankForMsgSender(address sender) public {
        // Arrange
        cheats.prank(sender);
        address expectedTxOrigin = tx.origin;

        // Act
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActivePrankForMsgSenderAndTxOrigin(address sender, address origin) public {
        // Arrange
        cheats.prank(sender, origin);

        // Act
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.Prank));
        assertEq(newSender, sender);
        assertEq(newOrigin, origin);
    }

    function testReadCallersAfterConsumingMsgSenderPrank(address sender) public {
        // Arrange
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender);

        // Act
        target.consumePrank();
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterConsumingMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender, origin);

        // Act
        target.consumePrank();
        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersWithActiveRecurrentMsgSenderPrank(address sender) public {
        // Arrange
        address expectedTxOrigin = tx.origin;
        Target target = new Target();
        cheats.startPrank(sender);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

            // Assert
            assertEq(uint256(mode), uint256(Cheats.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, expectedTxOrigin);
        }
    }

    function testReadCallersWithActiveRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        Target target = new Target();
        cheats.startPrank(sender, origin);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

            // Assert
            assertEq(uint256(mode), uint256(Cheats.CallerMode.RecurrentPrank));
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderPrank(address sender) public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender);

        // Act
        cheats.stopPrank();

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadCallersAfterStoppingRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender, origin);

        // Act
        cheats.stopPrank();

        (Cheats.CallerMode mode, address newSender, address newOrigin) = cheats.readCallers();

        // Assert
        assertEq(uint256(mode), uint256(Cheats.CallerMode.None));
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }
}
