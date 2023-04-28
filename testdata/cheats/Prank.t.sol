// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Victim {
    function assertCallerAndOrigin(
        address expectedSender,
        string memory senderMessage,
        address expectedOrigin,
        string memory originMessage
    ) public view {
        require(msg.sender == expectedSender, senderMessage);
        require(tx.origin == expectedOrigin, originMessage);
    }
}

contract ConstructorVictim is Victim {
    constructor(
        address expectedSender,
        string memory senderMessage,
        address expectedOrigin,
        string memory originMessage
    ) {
        require(msg.sender == expectedSender, senderMessage);
        require(tx.origin == expectedOrigin, originMessage);
    }
}

contract NestedVictim {
    Victim innerVictim;

    constructor(Victim victim) {
        innerVictim = victim;
    }

    function assertCallerAndOrigin(
        address expectedSender,
        string memory senderMessage,
        address expectedOrigin,
        string memory originMessage
    ) public view {
        require(msg.sender == expectedSender, senderMessage);
        require(tx.origin == expectedOrigin, originMessage);
        innerVictim.assertCallerAndOrigin(
            address(this),
            "msg.sender was incorrectly set for nested victim",
            expectedOrigin,
            "tx.origin was incorrectly set for nested victim"
        );
    }
}

contract NestedPranker {
    Cheats constant cheats = Cheats(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    address newSender;
    address newOrigin;
    address oldOrigin;

    constructor(address _newSender, address _newOrigin) {
        newSender = _newSender;
        newOrigin = _newOrigin;
        oldOrigin = tx.origin;
    }

    function incompletePrank() public {
        cheats.startPrank(newSender, newOrigin);
    }

    function completePrank(NestedVictim victim) public {
        victim.assertCallerAndOrigin(
            newSender, "msg.sender was not set in nested prank", newOrigin, "tx.origin was not set in nested prank"
        );
        cheats.stopPrank();

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this),
            "msg.sender was not cleaned up in nested prank",
            oldOrigin,
            "tx.origin was not cleaned up in nested prank"
        );
    }
}

contract PrankTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testPrankSender(address sender) public {
        // Perform the prank
        Victim victim = new Victim();
        cheats.prank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", tx.origin, "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", tx.origin, "tx.origin invariant failed"
        );
    }

    function testPrankOrigin(address sender, address origin) public {
        address oldOrigin = tx.origin;

        // Perform the prank
        Victim victim = new Victim();
        cheats.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin was not cleaned up"
        );
    }

    function testPrank1AfterPrank0(address sender, address origin) public {
        // Perform the prank
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        cheats.prank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );

        // Overwrite the prank
        cheats.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }
    
    function testPrank0AfterPrank1(address sender, address origin) public {
        // Perform the prank
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        cheats.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );

        // Overwrite the prank
        cheats.prank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }

    function testPrankConstructorSender(address sender) public {
        cheats.prank(sender);
        ConstructorVictim victim = new ConstructorVictim(
            sender,
            "msg.sender was not set during prank",
            tx.origin,
            "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", tx.origin, "tx.origin invariant failed"
        );
    }

    function testPrankConstructorOrigin(address sender, address origin) public {
        // Perform the prank
        cheats.prank(sender, origin);
        ConstructorVictim victim = new ConstructorVictim(
            sender,
            "msg.sender was not set during prank",
            origin,
            "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", tx.origin, "tx.origin was not cleaned up"
        );
    }

    function testPrankStartStop(address sender, address origin) public {
        address oldOrigin = tx.origin;

        // Perform the prank
        Victim victim = new Victim();
        cheats.startPrank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );
        victim.assertCallerAndOrigin(
            sender,
            "msg.sender was not set during prank (call 2)",
            origin,
            "tx.origin was not set during prank (call 2)"
        );
        cheats.stopPrank();

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin was not cleaned up"
        );
    }

    function testPrankStartStopConstructor(address sender, address origin) public {
        // Perform the prank
        cheats.startPrank(sender, origin);
        ConstructorVictim victim = new ConstructorVictim(
            sender,
            "msg.sender was not set during prank",
            origin,
            "tx.origin was not set during prank"
        );
        new ConstructorVictim(
            sender,
            "msg.sender was not set during prank (call 2)",
            origin,
            "tx.origin was not set during prank (call 2)"
        );
        cheats.stopPrank();

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", tx.origin, "tx.origin was not cleaned up"
        );
    }

    /// This test checks that depth is working correctly with respect
    /// to the `startPrank` and `stopPrank` cheatcodes.
    ///
    /// The nested pranker calls `startPrank` but does not call
    /// `stopPrank` at first.
    ///
    /// Then, we call our victim from the main test: this call
    /// should NOT have altered `msg.sender` or `tx.origin`.
    ///
    /// Then, the nested pranker will complete their prank: this call
    /// SHOULD have altered `msg.sender` and `tx.origin`.
    ///
    /// Each call to the victim calls yet another victim. The expected
    /// behavior for this call is that `tx.origin` is altered when
    /// the nested pranker calls, otherwise not. In both cases,
    /// `msg.sender` should be the address of the first victim.
    ///
    /// Success case:
    ///
    /// ┌────┐          ┌───────┐     ┌──────┐ ┌──────┐               ┌────────────┐
    /// │Test│          │Pranker│     │Cheats│ │Victim│               │Inner Victim│
    /// └─┬──┘          └───┬───┘     └──┬───┘ └──┬───┘               └─────┬──────┘
    ///   │                 │            │        │                         │
    ///   │incompletePrank()│            │        │                         │
    ///   │────────────────>│            │        │                         │
    ///   │                 │            │        │                         │
    ///   │                 │startPrank()│        │                         │
    ///   │                 │───────────>│        │                         │
    ///   │                 │            │        │                         │
    ///   │         should not be pranked│        │                         │
    ///   │──────────────────────────────────────>│                         │
    ///   │                 │            │        │                         │
    ///   │                 │            │        │  should not be pranked  │
    ///   │                 │            │        │────────────────────────>│
    ///   │                 │            │        │                         │
    ///   │ completePrank() │            │        │                         │
    ///   │────────────────>│            │        │                         │
    ///   │                 │            │        │                         │
    ///   │                 │  should be pranked  │                         │
    ///   │                 │────────────────────>│                         │
    ///   │                 │            │        │                         │
    ///   │                 │            │        │only tx.origin is pranked│
    ///   │                 │            │        │────────────────────────>│
    ///   │                 │            │        │                         │
    ///   │                 │stopPrank() │        │                         │
    ///   │                 │───────────>│        │                         │
    ///   │                 │            │        │                         │
    ///   │                 │should not be pranked│                         │
    ///   │                 │────────────────────>│                         │
    ///   │                 │            │        │                         │
    ///   │                 │            │        │  should not be pranked  │
    ///   │                 │            │        │────────────────────────>│
    /// ┌─┴──┐          ┌───┴───┐     ┌──┴───┐ ┌──┴───┐               ┌─────┴──────┐
    /// │Test│          │Pranker│     │Cheats│ │Victim│               │Inner Victim│
    /// └────┘          └───────┘     └──────┘ └──────┘               └────────────┘
    /// If this behavior is incorrectly implemented then the victim
    /// will be pranked the first time it is called.
    function testPrankComplex(address sender, address origin) public {
        address oldOrigin = tx.origin;

        NestedPranker pranker = new NestedPranker(sender, origin);
        Victim innerVictim = new Victim();
        NestedVictim victim = new NestedVictim(innerVictim);

        pranker.incompletePrank();
        victim.assertCallerAndOrigin(
            address(this),
            "msg.sender was altered at an incorrect depth",
            oldOrigin,
            "tx.origin was altered at an incorrect depth"
        );

        pranker.completePrank(victim);
    }

    /// Checks that `tx.origin` is set for all subcalls of a `prank`.
    ///
    /// Ref: issue #1210
    function testTxOriginInNestedPrank(address sender, address origin) public {
        address oldSender = msg.sender;
        address oldOrigin = tx.origin;

        Victim innerVictim = new Victim();
        NestedVictim victim = new NestedVictim(innerVictim);

        cheats.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set correctly", origin, "tx.origin was not set correctly"
        );
    }
}
