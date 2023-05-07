// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Target {
    function consumePrank() external {}
}

contract ReadPrankTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testReadPrankWithNoActivePrank() public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        // Act
        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadPrankWithActivePrankForMsgSender(address sender) public {
        // Arrange
        cheats.prank(sender);
        address expectedTxOrigin = tx.origin;

        // Act
        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertTrue(isActive);
        assertEq(newSender, sender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadPrankWithActivePrankForMsgSenderAndTxOrigin(address sender, address origin) public {
        // Arrange
        cheats.prank(sender, origin);

        // Act
        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertTrue(isActive);
        assertEq(newSender, sender);
        assertEq(newOrigin, origin);
    }

    function testReadPrankAfterConsumingMsgSenderPrank(address sender) public {
        // Arrange
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender);

        // Act
        target.consumePrank();
        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadPrankAfterConsumingMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        Target target = new Target();
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;

        cheats.prank(sender, origin);

        // Act
        target.consumePrank();
        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadPrankWithActiveRecurrentMsgSenderPrank(address sender) public {
        // Arrange
        address expectedTxOrigin = tx.origin;
        Target target = new Target();
        cheats.startPrank(sender);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

            // Assert
            assertTrue(isActive);
            assertEq(newSender, sender);
            assertEq(newOrigin, expectedTxOrigin);
        }
    }

    function testReadPrankWithActiveRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        Target target = new Target();
        cheats.startPrank(sender, origin);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

            // Assert
            assertTrue(isActive);
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadPrankAfterStoppingRecurrentMsgSenderPrank(address sender) public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender);

        // Act
        cheats.stopPrank();

        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }

    function testReadPrankAfterStoppingRecurrentMsgSenderAndTxOriginPrank(address sender, address origin) public {
        // Arrange
        address expectedSender = msg.sender;
        address expectedTxOrigin = tx.origin;
        cheats.startPrank(sender, origin);

        // Act
        cheats.stopPrank();

        (bool isActive, address newSender, address newOrigin) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, expectedSender);
        assertEq(newOrigin, expectedTxOrigin);
    }
}
