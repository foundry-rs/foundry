// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

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
    Vm constant vm = Vm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    address newSender;
    address newOrigin;
    address oldOrigin;

    constructor(address _newSender, address _newOrigin) {
        newSender = _newSender;
        newOrigin = _newOrigin;
        oldOrigin = tx.origin;
    }

    function incompletePrank() public {
        vm.startPrank(newSender, newOrigin);
    }

    function completePrank(NestedVictim victim) public {
        victim.assertCallerAndOrigin(
            newSender, "msg.sender was not set in nested prank", newOrigin, "tx.origin was not set in nested prank"
        );
        vm.stopPrank();

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this),
            "msg.sender was not cleaned up in nested prank",
            oldOrigin,
            "tx.origin was not cleaned up in nested prank"
        );
    }
}

contract ImplementationTest {
    uint256 public num;
    address public sender;

    function assertCorrectCaller(address expectedSender) public {
        require(msg.sender == expectedSender);
    }

    function assertCorrectOrigin(address expectedOrigin) public {
        require(tx.origin == expectedOrigin);
    }

    function setNum(uint256 _num) public {
        num = _num;
    }
}

contract ProxyTest {
    uint256 public num;
    address public sender;
}

contract PrankTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPrankDelegateCallPrank2() public {
        ProxyTest proxy = new ProxyTest();
        ImplementationTest impl = new ImplementationTest();
        vm.prank(address(proxy), true);

        // Assert correct `msg.sender`
        (bool success,) =
            address(impl).delegatecall(abi.encodeWithSignature("assertCorrectCaller(address)", address(proxy)));
        require(success, "prank2: delegate call failed assertCorrectCaller");

        // Assert storage updates
        uint256 num = 42;
        vm.prank(address(proxy), true);
        (bool successTwo,) = address(impl).delegatecall(abi.encodeWithSignature("setNum(uint256)", num));
        require(successTwo, "prank2: delegate call failed setNum");
        require(proxy.num() == num, "prank2: proxy's storage was not set correctly");
        vm.stopPrank();
    }

    function testPrankDelegateCallStartPrank2() public {
        ProxyTest proxy = new ProxyTest();
        ImplementationTest impl = new ImplementationTest();
        vm.startPrank(address(proxy), true);

        // Assert correct `msg.sender`
        (bool success,) =
            address(impl).delegatecall(abi.encodeWithSignature("assertCorrectCaller(address)", address(proxy)));
        require(success, "startPrank2: delegate call failed assertCorrectCaller");

        // Assert storage updates
        uint256 num = 42;
        (bool successTwo,) = address(impl).delegatecall(abi.encodeWithSignature("setNum(uint256)", num));
        require(successTwo, "startPrank2: delegate call failed setNum");
        require(proxy.num() == num, "startPrank2: proxy's storage was not set correctly");
        vm.stopPrank();
    }

    function testPrankDelegateCallPrank3(address origin) public {
        ProxyTest proxy = new ProxyTest();
        ImplementationTest impl = new ImplementationTest();
        vm.prank(address(proxy), origin, true);

        // Assert correct `msg.sender`
        (bool success,) =
            address(impl).delegatecall(abi.encodeWithSignature("assertCorrectCaller(address)", address(proxy)));
        require(success, "prank3: delegate call failed assertCorrectCaller");

        // Assert correct `tx.origin`
        vm.prank(address(proxy), origin, true);
        (bool successTwo,) = address(impl).delegatecall(abi.encodeWithSignature("assertCorrectOrigin(address)", origin));
        require(successTwo, "prank3: delegate call failed assertCorrectOrigin");

        // Assert storage updates
        uint256 num = 42;
        vm.prank(address(proxy), address(origin), true);
        (bool successThree,) = address(impl).delegatecall(abi.encodeWithSignature("setNum(uint256)", num));
        require(successThree, "prank3: delegate call failed setNum");
        require(proxy.num() == num, "prank3: proxy's storage was not set correctly");
        vm.stopPrank();
    }

    function testPrankDelegateCallStartPrank3(address origin) public {
        ProxyTest proxy = new ProxyTest();
        ImplementationTest impl = new ImplementationTest();
        vm.startPrank(address(proxy), origin, true);

        // Assert correct `msg.sender`
        (bool success,) =
            address(impl).delegatecall(abi.encodeWithSignature("assertCorrectCaller(address)", address(proxy)));
        require(success, "startPrank3: delegate call failed assertCorrectCaller");

        // Assert correct `tx.origin`
        (bool successTwo,) = address(impl).delegatecall(abi.encodeWithSignature("assertCorrectOrigin(address)", origin));
        require(successTwo, "startPrank3: delegate call failed assertCorrectOrigin");

        // Assert storage updates
        uint256 num = 42;
        (bool successThree,) = address(impl).delegatecall(abi.encodeWithSignature("setNum(uint256)", num));
        require(successThree, "startPrank3: delegate call failed setNum");
        require(proxy.num() == num, "startPrank3: proxy's storage was not set correctly");
        vm.stopPrank();
    }

    function testFailPrankDelegateCallToEOA() public {
        uint256 privateKey = uint256(keccak256(abi.encodePacked("alice")));
        address alice = vm.addr(privateKey);
        ImplementationTest impl = new ImplementationTest();
        vm.prank(alice, true);
        // Should fail when EOA pranked with delegatecall.
        address(impl).delegatecall(abi.encodeWithSignature("assertCorrectCaller(address)", alice));
    }

    function testPrankSender(address sender) public {
        // Perform the prank
        Victim victim = new Victim();
        vm.prank(sender);
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
        vm.prank(sender, origin);
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
        vm.prank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );

        // Overwrite the prank
        vm.prank(sender, origin);
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
        vm.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );

        // Overwrite the prank
        vm.prank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }

    function testStartPrank0AfterPrank1(address sender, address origin) public {
        // Perform the prank
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        vm.startPrank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );

        // Overwrite the prank
        vm.startPrank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin invariant failed"
        );

        vm.stopPrank();
        // Ensure we cleaned up correctly after stopping the prank
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }

    function testStartPrank1AfterStartPrank0(address sender, address origin) public {
        // Perform the prank
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        vm.startPrank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin was set during prank incorrectly"
        );

        // Ensure prank is still up as startPrank covers multiple calls
        victim.assertCallerAndOrigin(
            sender, "msg.sender was cleaned up incorrectly", oldOrigin, "tx.origin invariant failed"
        );

        // Overwrite the prank
        vm.startPrank(sender, origin);
        victim.assertCallerAndOrigin(sender, "msg.sender was not set during prank", origin, "tx.origin was not set");

        // Ensure prank is still up as startPrank covers multiple calls
        victim.assertCallerAndOrigin(
            sender, "msg.sender was cleaned up incorrectly", origin, "tx.origin invariant failed"
        );

        vm.stopPrank();
        // Ensure everything is back to normal after stopPrank
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }

    function testFailOverwriteUnusedPrank(address sender, address origin) public {
        // Set the prank, but not use it
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        vm.startPrank(sender, origin);
        // try to overwrite the prank. This should fail.
        vm.startPrank(address(this), origin);
    }

    function testFailOverwriteUnusedPrankAfterSuccessfulPrank(address sender, address origin) public {
        // Set the prank, but not use it
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        vm.startPrank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was set during prank incorrectly"
        );
        vm.startPrank(address(this), origin);
        // try to overwrite the prank. This should fail.
        vm.startPrank(sender, origin);
    }

    function testStartPrank0AfterStartPrank1(address sender, address origin) public {
        // Perform the prank
        address oldOrigin = tx.origin;
        Victim victim = new Victim();
        vm.startPrank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );

        // Ensure prank is still ongoing as we haven't called stopPrank
        victim.assertCallerAndOrigin(
            sender, "msg.sender was cleaned up incorrectly", origin, "tx.origin was cleaned up incorrectly"
        );

        // Overwrite the prank
        vm.startPrank(sender);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", oldOrigin, "tx.origin was not reset correctly"
        );

        vm.stopPrank();
        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin invariant failed"
        );
    }

    function testPrankConstructorSender(address sender) public {
        vm.prank(sender);
        ConstructorVictim victim = new ConstructorVictim(
            sender, "msg.sender was not set during prank", tx.origin, "tx.origin invariant failed"
        );

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", tx.origin, "tx.origin invariant failed"
        );
    }

    function testPrankConstructorOrigin(address sender, address origin) public {
        // Perform the prank
        vm.prank(sender, origin);
        ConstructorVictim victim = new ConstructorVictim(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
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
        vm.startPrank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );
        victim.assertCallerAndOrigin(
            sender,
            "msg.sender was not set during prank (call 2)",
            origin,
            "tx.origin was not set during prank (call 2)"
        );
        vm.stopPrank();

        // Ensure we cleaned up correctly
        victim.assertCallerAndOrigin(
            address(this), "msg.sender was not cleaned up", oldOrigin, "tx.origin was not cleaned up"
        );
    }

    function testPrankStartStopConstructor(address sender, address origin) public {
        // Perform the prank
        vm.startPrank(sender, origin);
        ConstructorVictim victim = new ConstructorVictim(
            sender, "msg.sender was not set during prank", origin, "tx.origin was not set during prank"
        );
        new ConstructorVictim(
            sender,
            "msg.sender was not set during prank (call 2)",
            origin,
            "tx.origin was not set during prank (call 2)"
        );
        vm.stopPrank();

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
    /// │Test│          │Pranker│     │Vm│ │Victim│               │Inner Victim│
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
    /// │Test│          │Pranker│     │Vm│ │Victim│               │Inner Victim│
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

        vm.prank(sender, origin);
        victim.assertCallerAndOrigin(
            sender, "msg.sender was not set correctly", origin, "tx.origin was not set correctly"
        );
    }
}
