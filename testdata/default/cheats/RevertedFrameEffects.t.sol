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
        } else if (effect == FrameEffect.Etch) {
            vm.etch(target, hex"00");
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
        revert();
    }
}

contract RevertedFrameOuter {
    function observeSender(RevertedFrameHelper helper, RevertedFrameProbe probe) external returns (address) {
        return helper.observeSender(probe);
    }

    function run(RevertedFrameHelper helper, FrameEffect effect, address target, RevertedFrameProbe probe) external {
        helper.run(effect, target, probe);
    }
}

contract RevertedFrameEffectsTest is Test {
    address constant target = address(0x1234);
    RevertedFrameHelper helper;
    RevertedFrameOuter outer;
    RevertedFrameProbe probe;

    function setUp() public {
        helper = new RevertedFrameHelper();
        outer = new RevertedFrameOuter();
        probe = new RevertedFrameProbe();
    }

    function test_deal_persists_depth1() public {
        try helper.run(FrameEffect.Deal, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(target.balance, 123);
    }

    function test_deal_persists_depth2() public {
        try outer.run(helper, FrameEffect.Deal, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(target.balance, 123);
    }

    function test_store_reverts_depth1() public {
        try helper.run(FrameEffect.Store, target, probe) {} catch {}
        assertEq(vm.load(target, bytes32(0)), bytes32(0));
    }

    function test_store_reverts_depth2() public {
        try outer.run(helper, FrameEffect.Store, target, probe) {} catch {}
        assertEq(vm.load(target, bytes32(0)), bytes32(0));
    }

    function test_etch_reverts_depth1() public {
        try helper.run(FrameEffect.Etch, target, probe) {} catch {}
        assertEq(target.code.length, 0);
    }

    function test_etch_reverts_depth2() public {
        try outer.run(helper, FrameEffect.Etch, target, probe) {} catch {}
        assertEq(target.code.length, 0);
    }

    function test_setNonce_persists_depth1() public {
        try helper.run(FrameEffect.SetNonce, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonce_persists_depth2() public {
        try outer.run(helper, FrameEffect.SetNonce, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonceUnsafe_persists_depth1() public {
        try helper.run(FrameEffect.SetNonceUnsafe, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_setNonceUnsafe_persists_depth2() public {
        try outer.run(helper, FrameEffect.SetNonceUnsafe, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 123);
    }

    function test_resetNonce_persists_depth1() public {
        vm.setNonce(target, 123);
        try helper.run(FrameEffect.ResetNonce, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 0);
    }

    function test_resetNonce_persists_depth2() public {
        vm.setNonce(target, 123);
        try outer.run(helper, FrameEffect.ResetNonce, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(vm.getNonce(target), 0);
    }

    function test_warp_persists_depth1() public {
        try helper.run(FrameEffect.Warp, target, probe) {} catch {}
        assertEq(block.timestamp, 123);
    }

    function test_warp_persists_depth2() public {
        try outer.run(helper, FrameEffect.Warp, target, probe) {} catch {}
        assertEq(block.timestamp, 123);
    }

    function test_roll_persists_depth1() public {
        try helper.run(FrameEffect.Roll, target, probe) {} catch {}
        assertEq(block.number, 123);
    }

    function test_roll_persists_depth2() public {
        try outer.run(helper, FrameEffect.Roll, target, probe) {} catch {}
        assertEq(block.number, 123);
    }

    function test_fee_persists_depth1() public {
        try helper.run(FrameEffect.Fee, target, probe) {} catch {}
        assertEq(block.basefee, 123);
    }

    function test_fee_persists_depth2() public {
        try outer.run(helper, FrameEffect.Fee, target, probe) {} catch {}
        assertEq(block.basefee, 123);
    }

    function test_chainId_persists_depth1() public {
        try helper.run(FrameEffect.ChainId, target, probe) {} catch {}
        assertEq(block.chainid, 123);
    }

    function test_chainId_persists_depth2() public {
        try outer.run(helper, FrameEffect.ChainId, target, probe) {} catch {}
        assertEq(block.chainid, 123);
    }

    function test_coinbase_persists_depth1() public {
        try helper.run(FrameEffect.Coinbase, target, probe) {} catch {}
        assertEq(block.coinbase, target);
    }

    function test_coinbase_persists_depth2() public {
        try outer.run(helper, FrameEffect.Coinbase, target, probe) {} catch {}
        assertEq(block.coinbase, target);
    }

    function test_prevrandao_persists_depth1() public {
        try helper.run(FrameEffect.Prevrandao, target, probe) {} catch {}
        assertEq(block.prevrandao, 123);
    }

    function test_prevrandao_persists_depth2() public {
        try outer.run(helper, FrameEffect.Prevrandao, target, probe) {} catch {}
        assertEq(block.prevrandao, 123);
    }

    function test_blobBaseFee_persists_depth1() public {
        try helper.run(FrameEffect.BlobBaseFee, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(block.blobbasefee, 7);
    }

    function test_blobBaseFee_persists_depth2() public {
        try outer.run(helper, FrameEffect.BlobBaseFee, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(block.blobbasefee, 7);
    }

    function test_txGasPrice_persists_depth1() public {
        try helper.run(FrameEffect.TxGasPrice, target, probe) {} catch {}
        assertEq(tx.gasprice, 123);
    }

    function test_txGasPrice_persists_depth2() public {
        try outer.run(helper, FrameEffect.TxGasPrice, target, probe) {} catch {}
        assertEq(tx.gasprice, 123);
    }

    function test_blobhashes_persists_depth1() public {
        try helper.run(FrameEffect.Blobhashes, target, probe) {} catch {}
        assertEq(blobhash(0), bytes32(uint256(123)));
    }

    function test_blobhashes_persists_depth2() public {
        try outer.run(helper, FrameEffect.Blobhashes, target, probe) {} catch {}
        assertEq(blobhash(0), bytes32(uint256(123)));
    }

    function test_label_persists_depth1() public {
        try helper.run(FrameEffect.Label, target, probe) {} catch {}
        assertEq(vm.getLabel(target), "reverted label");
    }

    function test_label_persists_depth2() public {
        try outer.run(helper, FrameEffect.Label, target, probe) {} catch {}
        assertEq(vm.getLabel(target), "reverted label");
    }

    function test_startPrank_persists_depth1() public {
        try helper.run(FrameEffect.StartPrank, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(helper.observeSender(probe), target);
    }

    function test_startPrank_persists_depth2() public {
        try outer.run(helper, FrameEffect.StartPrank, target, probe) {} catch {}
        // Master behavior: persists.
        assertEq(outer.observeSender(helper, probe), target);
    }

    function test_mockCall_persists_depth1() public {
        try helper.run(FrameEffect.MockCall, target, probe) {} catch {}
        assertEq(probe.value(), 123);
    }

    function test_mockCall_persists_depth2() public {
        try outer.run(helper, FrameEffect.MockCall, target, probe) {} catch {}
        assertEq(probe.value(), 123);
    }

    function test_record_persists_depth1() public {
        try helper.run(FrameEffect.Record, target, probe) {} catch {}
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(probe));
        assertEq(reads.length, 1);
        assertEq(reads[0], bytes32(0));
        assertEq(writes.length, 0);
    }

    function test_record_persists_depth2() public {
        try outer.run(helper, FrameEffect.Record, target, probe) {} catch {}
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(probe));
        assertEq(reads.length, 1);
        assertEq(reads[0], bytes32(0));
        assertEq(writes.length, 0);
    }

    function test_recordLogs_persists_depth1() public {
        try helper.run(FrameEffect.RecordLogs, target, probe) {} catch {}
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 1);
        assertEq(logs[0].emitter, address(helper));
        assertEq(logs[0].topics.length, 1);
        assertEq(logs[0].topics[0], keccak256("Recorded(uint256)"));
        assertEq(logs[0].data, abi.encode(uint256(123)));
    }

    function test_recordLogs_persists_depth2() public {
        try outer.run(helper, FrameEffect.RecordLogs, target, probe) {} catch {}
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 1);
        assertEq(logs[0].emitter, address(helper));
        assertEq(logs[0].topics.length, 1);
        assertEq(logs[0].topics[0], keccak256("Recorded(uint256)"));
        assertEq(logs[0].data, abi.encode(uint256(123)));
    }
}
