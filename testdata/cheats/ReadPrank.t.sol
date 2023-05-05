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

    // TODO: add tests when calling vm.prank(adddres)

    function testReadPrankWithNoActivePrank() public {
        // Act
        (bool isActive, , ) = cheats.readPrank();

        // Assert
        assertEq(isActive, false);
    }

    function testReadPrankWithActivePrank(
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

    function testReadPrankAfterConsumingPrank(
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

    function testReadPrankWithActiveRecurrentPrank(
        address sender,
        address origin
    ) public {
        // Arrange
        cheats.startPrank(sender, origin);

        for (uint i = 0; i < 5; i++) {
            // Act
            (bool isActive, address newSender, address newOrigin) = cheats
                .readPrank();

            // Assert
            assertTrue(isActive);
            assertEq(newSender, sender);
            assertEq(newOrigin, origin);
        }
    }

    function testReadPrankAfterStoppingRecurrentPrank(
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
