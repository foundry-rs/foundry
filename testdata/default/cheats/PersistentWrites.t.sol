// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Child {}

contract Bumper {
    function create() external {
        new Child();
    }
}

contract Reverter {
    uint256 x;

    function reverts() external {
        x = 1;
        revert();
    }
}

contract Dealer {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    bool fails;
    uint256 stored;

    fallback() external payable {
        vm.deal(address(0xA11CE), 100);
        stored = 1;
        require(!fails);
    }
}

contract SendThenDealZero {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    fallback() external payable {
        payable(address(0xA11CE)).transfer(msg.value);
        vm.deal(address(0xA11CE), 0);
        revert();
    }
}

/// `deal` and nonce cheatcodes survive reverts of the enclosing calls, and compose with the
/// transfers and nonce bumps those reverts undo.
contract PersistentWritesTest is Test {
    address payable constant A = payable(address(0xA11CE));
    address constant EOA = address(0xE0A);
    // Transfers 17 wei from `SENDER` to `TARGET` at nonce 0 on chain 1.
    bytes constant RAW_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
    address constant SENDER = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TARGET = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
    // Unfunded on the forked chains.
    address constant FRESH = address(uint160(uint256(keccak256("persistent writes"))));
    address constant FRESH_RESTORED = address(uint160(uint256(keccak256("persistent writes after a restore"))));
    Bumper bumper;
    Reverter reverter = new Reverter();
    uint256 otherFork;
    uint256 setUpSnapshot;

    function setUp() public {
        vm.deal(address(this), 1 ether);
        bumper = new Bumper();
        setUpSnapshot = vm.snapshotState();
    }

    function dealTwice() external {
        vm.deal(A, 1);
        vm.deal(A, 2);
        revert();
    }

    function transferThenDeal() external {
        A.transfer(10);
        vm.deal(A, 100);
        revert();
    }

    function dealThenTransfer() external {
        vm.deal(A, 100);
        A.transfer(10);
        revert();
    }

    function transferThenDealZero() external {
        A.transfer(10);
        vm.deal(A, 0);
        revert();
    }

    function transferThenDealZeroThenDeal() external {
        A.transfer(10);
        vm.deal(A, 0);
        vm.deal(A, 100);
        revert();
    }

    function dealAfterSaturatedChild() external {
        try this.transferThenDealZero() {} catch {}
        vm.deal(A, 5);
        revert();
    }

    function transferThenExecute() external {
        A.transfer(200);
        vm.executeTransaction(RAW_TX);
        revert();
    }

    function failedExecuteThenDeal() external {
        try vm.executeTransaction(RAW_TX) {} catch {}
        vm.deal(A, 5);
        revert();
    }

    function dealInRevertedChild() external {
        try this.dealTwice() {} catch {}
        A.transfer(10);
        revert();
    }

    function dealThenRestore() external {
        vm.deal(A, 100);
        vm.revertToState(vm.snapshotState());
        revert();
    }

    function restoreThenDeal() external {
        uint256 id = vm.snapshotState();
        vm.deal(A, 77);
        vm.revertToState(id);
        vm.deal(A, 100);
        revert();
    }

    function restoreEarlierThenDeal(uint256 id) external {
        vm.revertToState(id);
        vm.deal(A, 100);
        revert();
    }

    function dealThenRestoreSetUp() external {
        vm.deal(A, 100);
        vm.revertToState(setUpSnapshot);
        revert();
    }

    function dealThenRestoreInChild() external {
        vm.deal(A, 100);
        this.restoreSnapshot();
        revert();
    }

    function restoreSnapshot() external {
        vm.revertToState(vm.snapshotState());
    }

    function resetInRevertedChild() external {
        try this.resetNonceAfterCreate(0) {} catch {}
        vm.setNonceUnsafe(EOA, 5);
        revert();
    }

    function createThenSetNonce() external {
        bumper.create();
        vm.setNonce(address(bumper), 10);
        revert();
    }

    function setNonceTwice() external {
        vm.setNonce(A, 5);
        vm.setNonce(A, 7);
        revert();
    }

    function setNonceOnCreated() external {
        vm.setNonce(address(new Child{salt: 0}()), 5);
        revert();
    }

    function resetNonceAfterCreate(uint64 nonce) external {
        vm.prank(EOA);
        new Child();
        vm.resetNonce(EOA);
        if (nonce != 0) vm.setNonceUnsafe(EOA, nonce);
        revert();
    }

    function broadcastReverting() external {
        vm.startBroadcast(EOA);
        try reverter.reverts() {} catch {}
        vm.stopBroadcast();
    }

    function broadcastThenRevert() external {
        vm.broadcastRawTransaction(RAW_TX);
        revert();
    }

    function executeThenRevert() external {
        vm.store(TARGET, bytes32(uint256(1)), bytes32(uint256(42)));
        vm.executeTransaction(RAW_TX);
        revert();
    }

    function selectThenDeal() external {
        vm.selectFork(otherFork);
        vm.deal(FRESH, 100);
        revert();
    }

    function restoreThenSelectThenDeal() external {
        vm.revertToState(vm.snapshotState());
        vm.selectFork(otherFork);
        vm.deal(FRESH, 300);
        revert();
    }

    function restoreOtherForkThenDeal(uint256 id) external {
        vm.revertToState(id);
        vm.deal(FRESH_RESTORED, 300);
        revert();
    }

    function dealThenSelectFork() external {
        vm.deal(FRESH, 100);
        vm.selectFork(otherFork);
        revert();
    }

    function callReverting(bytes memory data) internal {
        (bool ok,) = address(this).call(data);
        require(!ok, "expected revert");
    }

    function balanceAfter(bytes memory data) internal returns (uint256) {
        vm.deal(A, 50);
        callReverting(data);
        return A.balance;
    }

    function testWritesSurviveReverts() public {
        assertEq(balanceAfter(abi.encodeCall(this.dealTwice, ())), 2, "dealTwice");
        assertEq(balanceAfter(abi.encodeCall(this.transferThenDeal, ())), 90, "transferThenDeal");
        assertEq(balanceAfter(abi.encodeCall(this.dealThenTransfer, ())), 100, "dealThenTransfer");
        // A change below the balance the revert restores leaves no balance.
        assertEq(balanceAfter(abi.encodeCall(this.transferThenDealZero, ())), 0, "transferThenDealZero");
        // The changes add up before saturating, and an enclosing revert keeps the saturated balance.
        assertEq(
            balanceAfter(abi.encodeCall(this.transferThenDealZeroThenDeal, ())), 90, "transferThenDealZeroThenDeal"
        );
        assertEq(balanceAfter(abi.encodeCall(this.dealAfterSaturatedChild, ())), 5, "dealAfterSaturatedChild");
        assertEq(balanceAfter(abi.encodeCall(this.dealInRevertedChild, ())), 2, "dealInRevertedChild");
        // A snapshot restore keeps the writes made before the snapshot.
        assertEq(balanceAfter(abi.encodeCall(this.dealThenRestoreInChild, ())), 100, "dealThenRestoreInChild");
        assertEq(balanceAfter(abi.encodeCall(this.dealThenRestore, ())), 100, "dealThenRestore");
        // The revert undoes the restore, and a write made after it replays as the value it set.
        assertEq(balanceAfter(abi.encodeCall(this.restoreThenDeal, ())), 100, "restoreThenDeal");
        uint256 id = vm.snapshotState();
        vm.deal(A, 10);
        callReverting(abi.encodeCall(this.restoreEarlierThenDeal, (id)));
        assertEq(A.balance, 100, "restoreEarlierThenDeal");

        callReverting(abi.encodeCall(this.createThenSetNonce, ()));
        assertEq(vm.getNonce(address(bumper)), 9, "createThenSetNonce");
        callReverting(abi.encodeCall(this.setNonceTwice, ()));
        assertEq(vm.getNonce(A), 7, "setNonceTwice");

        // A failed executed transaction rolls back its own `deal`, which survives as well.
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.deal(A, 50);
        vm.etch(TARGET, type(Dealer).runtimeCode);
        vm.store(TARGET, 0, bytes32(uint256(1)));
        try vm.executeTransaction(RAW_TX) {} catch {}
        assertEq(A.balance, 100, "failed executeTransaction deal");
        // The executed transaction's `deal` merges as a change, which saturates the same way.
        vm.setNonceUnsafe(SENDER, 0);
        vm.store(TARGET, 0, 0);
        assertEq(balanceAfter(abi.encodeCall(this.transferThenExecute, ())), 0, "transferThenExecute");
        // So does a failed executed transaction's `deal`, under an enclosing revert.
        vm.setNonceUnsafe(SENDER, 0);
        vm.etch(TARGET, type(SendThenDealZero).runtimeCode);
        assertEq(balanceAfter(abi.encodeCall(this.failedExecuteThenDeal, ())), 5, "failedExecuteThenDeal");

        // A snapshot of an earlier transaction holds none of the writes; last, as it rolls back the
        // rest of the state.
        assertEq(balanceAfter(abi.encodeCall(this.dealThenRestoreSetUp, ())), 100, "dealThenRestoreSetUp");
    }

    function testWritesUndoneWithTheirFrame() public {
        // A revert that undoes an account's creation also undoes the nonce set on it.
        callReverting(abi.encodeCall(this.setNonceOnCreated, ()));
        address created = vm.computeCreate2Address(0, keccak256(type(Child).creationCode));
        assertEq(vm.getNonce(created), 0, "setNonceOnCreated");
        callReverting(abi.encodeCall(this.resetNonceAfterCreate, (0)));
        assertEq(vm.getNonce(EOA), 0, "resetNonceAfterCreate");

        // The broadcast nonce bump of a failed call belongs to its caller's frame.
        this.broadcastReverting();
        assertEq(vm.getNonce(EOA), 1, "broadcastReverting");

        // The undone nonce changes add up before saturating.
        callReverting(abi.encodeCall(this.resetNonceAfterCreate, (10)));
        assertEq(vm.getNonce(EOA), 9, "setNonceUnsafeAfterCreate");
        // An enclosing revert keeps the saturated nonce.
        callReverting(abi.encodeCall(this.resetInRevertedChild, ()));
        assertEq(vm.getNonce(EOA), 5, "resetInRevertedChild");

        // The executed transaction's own `deal` is part of its merged changes.
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.deal(A, 50);
        vm.etch(TARGET, type(Dealer).runtimeCode);
        callReverting(abi.encodeCall(this.executeThenRevert, ()));
        assertEq(A.balance, 100, "executeTransaction deal");
        // Intentionally differs from master, whose revert restored the enclosing store over it.
        assertEq(vm.load(TARGET, bytes32(uint256(1))), bytes32(uint256(1)), "executeTransaction store");

        // Unlike executeTransaction, a raw transaction that fails at its root drops its writes.
        vm.setNonceUnsafe(SENDER, 0);
        vm.deal(A, 50);
        vm.store(TARGET, 0, bytes32(uint256(1)));
        vm.broadcastRawTransaction(RAW_TX);
        assertEq(vm.getNonce(SENDER), 1, "raw transaction executed");
        assertEq(A.balance, 50, "failed raw transaction deal");
    }

    // A raw transaction accepts its changes to accounts the running transaction has not loaded, so
    // a revert neither undoes nor replays them. Only a failing call that undoes a snapshot restore
    // replays them, see `StateSnapshots.t.sol`.
    function testRawTransactionInRevertedCall() public {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        callReverting(abi.encodeCall(this.broadcastThenRevert, ()));
        assertEq(TARGET.balance, 17);
        assertEq(vm.getNonce(SENDER), 1);
    }

    // A failed executed transaction drops the changes a raw transaction it ran accepted, with the
    // rest of its state.
    function testRawTransactionInFailedExecutedTransaction() public {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.deal(vm.addr(1), 1 ether);
        bytes[] memory unsigned = new bytes[](9);
        unsigned[1] = hex"01";
        unsigned[2] = hex"030d40";
        unsigned[3] = abi.encodePacked(address(this));
        unsigned[5] = abi.encodeCall(this.broadcastThenRevert, ());
        unsigned[6] = hex"01";
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, keccak256(vm.toRlp(unsigned)));
        unsigned[6] = abi.encodePacked(v + 10);
        unsigned[7] = trimLeadingZeros(r);
        unsigned[8] = trimLeadingZeros(s);
        try vm.executeTransaction(vm.toRlp(unsigned)) {} catch {}
        assertEq(TARGET.balance, 0);
        assertEq(vm.getNonce(SENDER), 0);
    }

    function trimLeadingZeros(bytes32 word) internal pure returns (bytes memory out) {
        uint256 offset;
        while (offset < 32 && word[offset] == bytes1(0)) {
            offset++;
        }
        out = new bytes(32 - offset);
        for (uint256 i; i < out.length; i++) {
            out[i] = word[offset + i];
        }
    }

    // A fork switch accepts the writes on the fork it leaves, so a later revert does not replay
    // them on the fork it selected.
    function testForkWritesStayOnTheirFork() public {
        uint256 fork = vm.createSelectFork("mainnet");
        otherFork = vm.createFork("mainnet");
        callReverting(abi.encodeCall(this.dealThenSelectFork, ()));
        assertEq(FRESH.balance, 0, "selected fork");
        vm.selectFork(fork);
        assertEq(FRESH.balance, 100, "left fork");

        // A write after the switch is undone with the frame only if its rollback reached it.
        callReverting(abi.encodeCall(this.selectThenDeal, ()));
        assertEq(FRESH.balance, 100, "dealt after the switch");

        // A revert that undoes a restore does not keep the writes made after a later switch, which
        // accepted the state the restore installed.
        vm.selectFork(fork);
        callReverting(abi.encodeCall(this.restoreThenSelectThenDeal, ()));
        assertEq(FRESH.balance, 100, "dealt after a restore and a switch");

        // Nor the writes made after a restore that switches back to the fork of an earlier
        // snapshot, taken before the selected fork was created.
        vm.selectFork(fork);
        uint256 id = vm.snapshotState();
        vm.createSelectFork("mainnet");
        callReverting(abi.encodeCall(this.restoreOtherForkThenDeal, (id)));
        assertEq(vm.activeFork(), fork, "restored fork");
        assertEq(FRESH_RESTORED.balance, 0, "dealt after a restore to another fork");
    }
}
