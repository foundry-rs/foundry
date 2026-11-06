// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

enum FrameEffect {
    Deal,
    Store,
    Etch,
    SetNonce,
    SetNonceUnsafe,
    ResetNonce,
    Warp,
    Roll,
    Fee,
    ChainId,
    Coinbase,
    Prevrandao,
    BlobBaseFee,
    TxGasPrice,
    Blobhashes,
    Label,
    StartPrank,
    MockCall,
    Record,
    RecordLogs
}

contract RevertedFrameProbe {
    uint256 public value = 7;

    function sender() external view returns (address) {
        return msg.sender;
    }
}

error FrameEffectApplied();

contract RevertedFrameHelper is Test {
    event Recorded(uint256 value);

    function observeSender(RevertedFrameProbe probe) external returns (address sender) {
        sender = probe.sender();
        vm.stopPrank();
    }

    function run(FrameEffect effect, address target, RevertedFrameProbe probe) external {
        if (effect == FrameEffect.Deal) {
            vm.deal(target, 123);
        } else if (effect == FrameEffect.Store) {
            vm.store(target, bytes32(0), bytes32(uint256(123)));
            require(vm.load(target, bytes32(0)) == bytes32(uint256(123)), "store was not applied");
        } else if (effect == FrameEffect.Etch) {
            vm.etch(target, hex"00");
            require(keccak256(target.code) == keccak256(hex"00"), "etch was not applied");
        } else if (effect == FrameEffect.SetNonce) {
            vm.setNonce(target, 123);
        } else if (effect == FrameEffect.SetNonceUnsafe) {
            vm.setNonceUnsafe(target, 123);
        } else if (effect == FrameEffect.ResetNonce) {
            vm.resetNonce(target);
        } else if (effect == FrameEffect.Warp) {
            vm.warp(123);
        } else if (effect == FrameEffect.Roll) {
            vm.roll(123);
        } else if (effect == FrameEffect.Fee) {
            vm.fee(123);
        } else if (effect == FrameEffect.ChainId) {
            vm.chainId(123);
        } else if (effect == FrameEffect.Coinbase) {
            vm.coinbase(target);
        } else if (effect == FrameEffect.Prevrandao) {
            vm.prevrandao(bytes32(uint256(123)));
        } else if (effect == FrameEffect.BlobBaseFee) {
            vm.blobBaseFee(10_000_000);
        } else if (effect == FrameEffect.TxGasPrice) {
            vm.txGasPrice(123);
        } else if (effect == FrameEffect.Blobhashes) {
            bytes32[] memory hashes = new bytes32[](1);
            hashes[0] = bytes32(uint256(123));
            vm.blobhashes(hashes);
        } else if (effect == FrameEffect.Label) {
            vm.label(target, "reverted label");
        } else if (effect == FrameEffect.StartPrank) {
            vm.startPrank(target);
        } else if (effect == FrameEffect.MockCall) {
            vm.mockCall(address(probe), abi.encodeWithSelector(probe.value.selector), abi.encode(uint256(123)));
        } else if (effect == FrameEffect.Record) {
            vm.record();
            probe.value();
        } else if (effect == FrameEffect.RecordLogs) {
            vm.recordLogs();
            emit Recorded(123);
        }
        revert FrameEffectApplied();
    }

    function dealAfterReceive(uint256 amount) external payable {
        vm.deal(address(this), amount);
        revert FrameEffectApplied();
    }

    function dealThenRevertToOwnState(uint256 amount) external payable {
        vm.deal(address(this), amount);
        vm.revertToState(vm.snapshotState());
        revert FrameEffectApplied();
    }

    function revertToStateBetweenWrites(uint256 amount, uint256 snapshot) external payable {
        vm.deal(address(this), amount);
        vm.revertToState(snapshot);
        vm.setNonce(address(this), 50);
        revert FrameEffectApplied();
    }

    function dealThenSelectForks(uint256 amount, uint256 otherFork, uint256 thisFork) external payable {
        vm.deal(address(this), amount);
        vm.selectFork(otherFork);
        vm.selectFork(thisFork);
        revert FrameEffectApplied();
    }

    function dealThenExecuteTransaction(uint256 amount, bytes calldata rawTx) external payable {
        vm.deal(address(this), amount);
        vm.executeTransaction(rawTx);
        revert FrameEffectApplied();
    }

    function setNonceAfterCreate() external {
        new RevertedFrameProbe();
        vm.setNonce(address(this), 50);
        revert FrameEffectApplied();
    }
}

contract RevertedFrameOuter {
    function observeSender(RevertedFrameHelper helper, RevertedFrameProbe probe) external returns (address) {
        return helper.observeSender(probe);
    }

    function run(RevertedFrameHelper helper, FrameEffect effect, address target, RevertedFrameProbe probe) external {
        helper.run(effect, target, probe);
    }

    function dealAfterReceive(RevertedFrameHelper helper, uint256 amount) external payable {
        helper.dealAfterReceive{value: msg.value}(amount);
    }

    function revertToStateBetweenWrites(RevertedFrameHelper helper, uint256 amount, uint256 snapshot)
        external
        payable
    {
        try helper.revertToStateBetweenWrites{value: msg.value}(amount, snapshot) {} catch {}
    }
}

contract RevertedFrameEffectsTest is Test {
    address constant target = address(0x1234);
    RevertedFrameHelper helper;
    RevertedFrameOuter outer;
    RevertedFrameProbe probe;

    event Recorded(uint256 value);

    function setUp() public {
        helper = new RevertedFrameHelper();
        outer = new RevertedFrameOuter();
        probe = new RevertedFrameProbe();
    }

    function test_deal_persists_depth1() public {
        runAndCatch(FrameEffect.Deal, false);
        // Master behavior: persists.
        assertEq(target.balance, 123);
    }

    function test_deal_persists_depth2() public {
        runAndCatch(FrameEffect.Deal, true);
        // Master behavior: persists.
        assertEq(target.balance, 123);
    }

    function test_deal_after_receive_persists_depth1() public {
        vm.deal(address(this), 1 ether);
        (bool success, bytes memory result) =
            address(helper).call{value: 1 ether}(abi.encodeCall(helper.dealAfterReceive, (0)));
        assertHelperReverted(success, result);
        assertEq(address(helper).balance, 0);
        assertEq(address(this).balance, 1 ether);
    }

    function test_deal_after_receive_persists_depth2() public {
        vm.deal(address(this), 1 ether);
        (bool success, bytes memory result) =
            address(outer).call{value: 1 ether}(abi.encodeCall(outer.dealAfterReceive, (helper, 5 ether)));
        assertHelperReverted(success, result);
        assertEq(address(helper).balance, 5 ether);
        assertEq(address(outer).balance, 0);
        assertEq(address(this).balance, 1 ether);
    }

    function test_deal_after_receive_persists_across_state_revert() public {
        vm.deal(address(this), 1 ether);
        (bool success, bytes memory result) =
            address(helper).call{value: 1 ether}(abi.encodeCall(helper.dealThenRevertToOwnState, (0)));
        assertHelperReverted(success, result);
        assertEq(address(helper).balance, 0);
        assertEq(address(this).balance, 1 ether);
    }

    function test_writes_around_earlier_state_revert_persist_depth2() public {
        vm.deal(address(this), 1 ether);
        uint256 snapshot = vm.snapshotState();
        outer.revertToStateBetweenWrites{value: 1 ether}(helper, 0, snapshot);
        assertEq(address(helper).balance, 0);
        assertEq(vm.getNonce(address(helper)), 50);
        assertEq(address(outer).balance, 1 ether);
    }

    function test_deal_after_receive_persists_across_fork_round_trip() public {
        uint256 forkA = vm.createSelectFork("mainnet", 14_608_400);
        uint256 forkB = vm.createFork("mainnet", 14_608_401);
        RevertedFrameHelper forkHelper = new RevertedFrameHelper();
        vm.deal(address(this), 1 ether);
        (bool success, bytes memory result) = address(forkHelper).call{value: 1 ether}(
            abi.encodeCall(forkHelper.dealThenSelectForks, (0, forkB, forkA))
        );
        assertHelperReverted(success, result);
        assertEq(address(forkHelper).balance, 0);
        assertEq(address(this).balance, 1 ether);
    }

    function test_deal_after_receive_persists_across_unrelated_transaction() public {
        vm.chainId(1);
        vm.deal(0x5316812db67073C4d4af8BB3000C5B86c2877e94, 1 ether);
        vm.deal(address(this), 1 ether);
        // Legacy transfer of 17 wei from 0x5316812db67073C4d4af8BB3000C5B86c2877e94 at nonce 0.
        bytes memory rawTx =
            hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
        (bool success, bytes memory result) =
            address(helper).call{value: 1 ether}(abi.encodeCall(helper.dealThenExecuteTransaction, (0, rawTx)));
        assertHelperReverted(success, result);
        assertEq(address(helper).balance, 0);
        assertEq(address(this).balance, 1 ether);
    }

    function test_store_reverts_depth1() public {
        runAndCatch(FrameEffect.Store, false);
        assertEq(vm.load(target, bytes32(0)), bytes32(0));
    }

    function test_store_reverts_depth2() public {
        runAndCatch(FrameEffect.Store, true);
        assertEq(vm.load(target, bytes32(0)), bytes32(0));
    }

    function test_etch_reverts_depth1() public {
        runAndCatch(FrameEffect.Etch, false);
        assertEq(target.code.length, 0);
    }

    function test_etch_reverts_depth2() public {
        runAndCatch(FrameEffect.Etch, true);
        assertEq(target.code.length, 0);
    }

    function test_setNonce_persists_depth1() public {
        runAndCatch(FrameEffect.SetNonce, false);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonce_persists_depth2() public {
        runAndCatch(FrameEffect.SetNonce, true);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonce_after_create_persists_depth1() public {
        (bool success, bytes memory result) = address(helper).call(abi.encodeCall(helper.setNonceAfterCreate, ()));
        assertHelperReverted(success, result);
        assertEq(vm.getNonce(address(helper)), 50);
    }

    function test_setNonceUnsafe_persists_depth1() public {
        runAndCatch(FrameEffect.SetNonceUnsafe, false);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonceUnsafe_persists_depth2() public {
        runAndCatch(FrameEffect.SetNonceUnsafe, true);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_resetNonce_persists_depth1() public {
        vm.setNonce(target, 123);
        runAndCatch(FrameEffect.ResetNonce, false);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 0);
    }

    function test_resetNonce_persists_depth2() public {
        vm.setNonce(target, 123);
        runAndCatch(FrameEffect.ResetNonce, true);
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 0);
    }

    function test_warp_persists_depth1() public {
        runAndCatch(FrameEffect.Warp, false);
        assertEq(block.timestamp, 123);
    }

    function test_warp_persists_depth2() public {
        runAndCatch(FrameEffect.Warp, true);
        assertEq(block.timestamp, 123);
    }

    function test_roll_persists_depth1() public {
        runAndCatch(FrameEffect.Roll, false);
        assertEq(block.number, 123);
    }

    function test_roll_persists_depth2() public {
        runAndCatch(FrameEffect.Roll, true);
        assertEq(block.number, 123);
    }

    function test_fee_persists_depth1() public {
        runAndCatch(FrameEffect.Fee, false);
        assertEq(block.basefee, 123);
    }

    function test_fee_persists_depth2() public {
        runAndCatch(FrameEffect.Fee, true);
        assertEq(block.basefee, 123);
    }

    function test_chainId_persists_depth1() public {
        runAndCatch(FrameEffect.ChainId, false);
        assertEq(block.chainid, 123);
    }

    function test_chainId_persists_depth2() public {
        runAndCatch(FrameEffect.ChainId, true);
        assertEq(block.chainid, 123);
    }

    function test_coinbase_persists_depth1() public {
        runAndCatch(FrameEffect.Coinbase, false);
        assertEq(block.coinbase, target);
    }

    function test_coinbase_persists_depth2() public {
        runAndCatch(FrameEffect.Coinbase, true);
        assertEq(block.coinbase, target);
    }

    function test_prevrandao_persists_depth1() public {
        runAndCatch(FrameEffect.Prevrandao, false);
        assertEq(block.prevrandao, 123);
    }

    function test_prevrandao_persists_depth2() public {
        runAndCatch(FrameEffect.Prevrandao, true);
        assertEq(block.prevrandao, 123);
    }

    function test_blobBaseFee_persists_depth1() public {
        runAndCatch(FrameEffect.BlobBaseFee, false);
        // Master behavior: persists.
        assertEq(block.blobbasefee, 10_000_000);
    }

    function test_blobBaseFee_persists_depth2() public {
        runAndCatch(FrameEffect.BlobBaseFee, true);
        // Master behavior: persists.
        assertEq(block.blobbasefee, 10_000_000);
    }

    function test_txGasPrice_persists_depth1() public {
        runAndCatch(FrameEffect.TxGasPrice, false);
        assertEq(tx.gasprice, 123);
    }

    function test_txGasPrice_persists_depth2() public {
        runAndCatch(FrameEffect.TxGasPrice, true);
        assertEq(tx.gasprice, 123);
    }

    function test_blobhashes_persists_depth1() public {
        runAndCatch(FrameEffect.Blobhashes, false);
        assertEq(blobhash(0), bytes32(uint256(123)));
    }

    function test_blobhashes_persists_depth2() public {
        runAndCatch(FrameEffect.Blobhashes, true);
        assertEq(blobhash(0), bytes32(uint256(123)));
    }

    function test_label_persists_depth1() public {
        runAndCatch(FrameEffect.Label, false);
        assertEq(vm.getLabel(target), "reverted label");
    }

    function test_label_persists_depth2() public {
        runAndCatch(FrameEffect.Label, true);
        assertEq(vm.getLabel(target), "reverted label");
    }

    function test_startPrank_persists_depth1() public {
        runAndCatch(FrameEffect.StartPrank, false);
        // Master behavior: persists.
        assertEq(helper.observeSender(probe), target);
    }

    function test_startPrank_persists_depth2() public {
        runAndCatch(FrameEffect.StartPrank, true);
        // Master behavior: persists.
        assertEq(outer.observeSender(helper, probe), target);
    }

    function test_mockCall_persists_depth1() public {
        runAndCatch(FrameEffect.MockCall, false);
        assertEq(probe.value(), 123);
    }

    function test_mockCall_persists_depth2() public {
        runAndCatch(FrameEffect.MockCall, true);
        assertEq(probe.value(), 123);
    }

    function test_record_persists_depth1() public {
        runAndCatch(FrameEffect.Record, false);
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(probe));
        assertEq(reads.length, 1);
        assertEq(reads[0], bytes32(0));
        assertEq(writes.length, 0);
    }

    function test_record_persists_depth2() public {
        runAndCatch(FrameEffect.Record, true);
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(probe));
        assertEq(reads.length, 1);
        assertEq(reads[0], bytes32(0));
        assertEq(writes.length, 0);
    }

    function test_recordLogs_persists_without_reverted_logs_depth1() public {
        runAndCatch(FrameEffect.RecordLogs, false);
        assertEq(vm.getRecordedLogs().length, 0);
        emit Recorded(456);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 1);
        assertEq(logs[0].emitter, address(this));
        assertEq(logs[0].topics.length, 1);
        assertEq(logs[0].topics[0], keccak256("Recorded(uint256)"));
        assertEq(logs[0].data, abi.encode(uint256(456)));
    }

    function test_recordLogs_persists_without_reverted_logs_depth2() public {
        runAndCatch(FrameEffect.RecordLogs, true);
        assertEq(vm.getRecordedLogs().length, 0);
        emit Recorded(456);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 1);
        assertEq(logs[0].emitter, address(this));
        assertEq(logs[0].topics.length, 1);
        assertEq(logs[0].topics[0], keccak256("Recorded(uint256)"));
        assertEq(logs[0].data, abi.encode(uint256(456)));
    }

    function runAndCatch(FrameEffect effect, bool nested) internal {
        bool success;
        bytes memory result;
        if (nested) {
            (success, result) = address(outer).call(abi.encodeCall(outer.run, (helper, effect, target, probe)));
        } else {
            (success, result) = address(helper).call(abi.encodeCall(helper.run, (effect, target, probe)));
        }
        assertHelperReverted(success, result);
    }

    function assertHelperReverted(bool success, bytes memory result) internal {
        assertTrue(!success, "helper did not revert");
        assertEq(result, abi.encodeWithSelector(FrameEffectApplied.selector), "unexpected helper revert");
    }
}
