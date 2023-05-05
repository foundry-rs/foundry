// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";
import "../logs/console.sol";

contract Target {
    function consumePrank() external {}
}

contract ReadPrankTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testReadPrankWithNoActivePrank() public {
        // Act
        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, address(0));
        assertEq(newOrigin, address(0));
    }

    function testReadPrankWithActivePrankForMsgSender(address sender) public {
        // Arrange
        cheats.prank(sender);

        // Act
        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertTrue(isActive);
        assertEq(newSender, sender);
        assertEq(newOrigin, address(0));
    }

    function testReadPrankWithActivePrankForMsgSenderAndTxOrigin(
        address sender,
        address origin
    ) public {
        // Arrange
        cheats.prank(sender, origin);

        // Act
        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertTrue(isActive);
        assertEq(newSender, sender);
        assertEq(newOrigin, origin);
    }

    function testReadPrankAfterConsumingMsgSenderPrank(address sender) public {
        // Arrange
        Target target = new Target();
        cheats.prank(sender);

        // Act
        target.consumePrank();
        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, address(0));
        assertEq(newOrigin, address(0));
    }

    function testReadPrankAfterConsumingMsgSenderAndTxOriginPrank(
        address sender,
        address origin
    ) public {
        // Arrange
        Target target = new Target();
        cheats.prank(sender, origin);

        // Act
        target.consumePrank();
        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, address(0));
        assertEq(newOrigin, address(0));
    }

    function testReadPrankWithActiveRecurrentMsgSenderPrank(address sender)
        public
    {
        // Arrange
        Target target = new Target();
        cheats.startPrank(sender);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (bool isActive, address newSender, address newOrigin) = cheats
                .readPrank();

            // Assert
            assertTrue(isActive);
            assertEq(newSender, sender);
            assertEq(newOrigin, address(0));
        }
    }

    function testReadPrankWithActiveRecurrentMsgSenderAndTxOriginPrank(
        address sender,
        address origin
    ) public {
        // Arrange
        Target target = new Target();
        cheats.startPrank(sender, origin);

        for (uint256 i = 0; i < 5; i++) {
            // Act
            target.consumePrank();
            (bool isActive, address newSender, address newOrigin) = cheats
                .readPrank();

            // Assert
            assertTrue(isActive);
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadPrankAfterStoppingRecurrentMsgSenderPrank(address sender)
        public
    {
        // Arrange
        cheats.startPrank(sender);

        // Act
        cheats.stopPrank();

        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, address(0));
        assertEq(newOrigin, address(0));
    }

    function testReadPrankAfterStoppingRecurrentMsgSenderAndTxOriginPrank(
        address sender,
        address origin
    ) public {
        // Arrange
        cheats.startPrank(sender, origin);

        // Act
        cheats.stopPrank();

        (bool isActive, address newSender, address newOrigin) = cheats
            .readPrank();

        // Assert
        assertEq(isActive, false);
        assertEq(newSender, address(0));
        assertEq(newOrigin, address(0));
    }
}
