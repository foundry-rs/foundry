// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract WarmthTarget {
    uint256 public x = 1;

    function set(uint256 value) external {
        x = value;
    }
}

contract WarmthReverter {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function dealThenRevert(address target) external {
        vm.deal(target, 1);
        revert("reverted");
    }
}

/// Cheatcodes that load an account or a storage slot leave it warm (EIP-2929), so the first real
/// access after them pays the warm cost, and the warmth reverts with the enclosing call.
abstract contract CheatcodeWarmthBase is Test {
    // Signed legacy transfer of 17 wei from `TX_FROM` to `TX_TO` on chain 1.
    bytes constant TRANSFER_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
    address constant TX_FROM = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TX_TO = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
    uint256 constant TIP20_LOGO_URI_SLOT = 5;

    // Stored at slot 1, after `IS_TEST` and `_failed`.
    string text = "short";

    function isWarm(address target) internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let value := balance(target)
            cost := sub(cost, gas())
            // Keeps the optimizer from removing the access.
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
        // A cold account access costs 2600 gas, a warm one 100.
        return cost < 2600;
    }

    function isWarmSlot(uint256 key) internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let value := sload(key)
            cost := sub(cost, gas())
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
        // A cold slot access costs 2100 gas, a warm one 100.
        return cost < 2100;
    }

    function testAccessWithoutCheatcodeIsCold() public {
        vm.assertFalse(isWarm(address(0x1000)));
        vm.assertFalse(isWarmSlot(100));
    }

    function testDealWarms() public {
        vm.deal(address(0x1001), 1);
        vm.assertTrue(isWarm(address(0x1001)));
    }

    function testEtchWarms() public {
        vm.etch(address(0x1002), hex"00");
        vm.assertTrue(isWarm(address(0x1002)));
    }

    function testStoreWarmsAccountAndSlot() public {
        vm.store(address(0x1003), 0, bytes32(uint256(1)));
        vm.assertTrue(isWarm(address(0x1003)));

        vm.store(address(this), bytes32(uint256(101)), bytes32(uint256(1)));
        vm.assertTrue(isWarmSlot(101));
    }

    function testLoadWarmsAccountAndSlot() public {
        vm.load(address(0x1004), 0);
        vm.assertTrue(isWarm(address(0x1004)));

        vm.load(address(this), bytes32(uint256(102)));
        vm.assertTrue(isWarmSlot(102));
    }

    function testNonceCheatcodesWarm() public {
        vm.setNonce(address(0x1005), 3);
        vm.assertTrue(isWarm(address(0x1005)));

        vm.setNonceUnsafe(address(0x1006), 3);
        vm.assertTrue(isWarm(address(0x1006)));

        vm.resetNonce(address(0x1007));
        vm.assertTrue(isWarm(address(0x1007)));

        vm.getNonce(address(0x1008));
        vm.assertTrue(isWarm(address(0x1008)));

        Vm.Wallet memory wallet = vm.createWallet(7);
        vm.getNonce(wallet);
        vm.assertTrue(isWarm(wallet.addr));
    }

    function testMockCallWarmsCallee() public {
        vm.mockCall(address(0x1009), bytes4(0x12345678), "");
        vm.assertTrue(isWarm(address(0x1009)));

        vm.mockCallRevert(address(0x100a), bytes4(0x12345678), "");
        vm.assertTrue(isWarm(address(0x100a)));

        vm.mockCalls(address(0x100b), abi.encodePacked(bytes4(0x12345678)), new bytes[](1));
        vm.assertTrue(isWarm(address(0x100b)));
    }

    function testMockFunctionDoesNotWarm() public {
        vm.mockFunction(address(0x100c), address(0x100d), abi.encodePacked(bytes4(0x12345678)));
        vm.assertFalse(isWarm(address(0x100c)));
        vm.assertFalse(isWarm(address(0x100d)));
    }

    function testPrankWarmsSenderOnly() public {
        vm.prank(address(0x100e), address(0x100f));
        vm.assertTrue(isWarm(address(0x100e)));
        vm.assertFalse(isWarm(address(0x100f)));
        vm.stopPrank();

        vm.startPrank(address(0x1010));
        vm.assertTrue(isWarm(address(0x1010)));
        vm.stopPrank();
    }

    function testAppliedPrankWarmsSender() public {
        vm.startPrank(address(0x1011));
        vm.cool(address(0x1011));
        new WarmthTarget();
        vm.stopPrank();
        vm.assertTrue(isWarm(address(0x1011)));
    }

    function testBroadcastWarmsSender() public {
        vm.broadcast(address(0x1012));
        vm.assertTrue(isWarm(address(0x1012)));
    }

    function testLabelDoesNotWarm() public {
        vm.label(address(0x1013), "label");
        vm.assertFalse(isWarm(address(0x1013)));
    }

    function testDelegationCheatcodesWarmAuthority() public {
        address implementation = address(0x1014);

        vm.signDelegation(implementation, 0x1001);
        vm.assertTrue(isWarm(vm.addr(0x1001)));

        // An explicit nonce does not read the authority's account.
        vm.signDelegation(implementation, 0x1002, 0);
        vm.assertFalse(isWarm(vm.addr(0x1002)));

        vm.signAndAttachDelegation(implementation, 0x1003, 0);
        vm.assertTrue(isWarm(vm.addr(0x1003)));

        Vm.SignedDelegation memory delegation = vm.signDelegation(implementation, 0x1004, 0);
        vm.attachDelegation(delegation);
        vm.assertTrue(isWarm(vm.addr(0x1004)));

        // The authority is loaded before its nonce is checked.
        delegation = vm.signDelegation(implementation, 0x1005, 5);
        (bool success,) = address(vm).call(abi.encodeWithSignature(
            "attachDelegation((uint8,bytes32,bytes32,uint64,address))", delegation
        ));
        vm.assertFalse(success);
        vm.assertTrue(isWarm(vm.addr(0x1005)));
    }

    function testGetStorageSlotsWarmsTargetAndBytesSlot() public {
        vm.getStorageSlots(address(this), "text");
        vm.cool(address(this));
        vm.getStorageSlots(address(this), "text");
        vm.assertTrue(isWarmSlot(1));

        WarmthTarget target = new WarmthTarget();
        vm.cool(address(target));
        vm.getStorageSlots(address(target), "x");
        vm.assertTrue(isWarm(address(target)));
    }

    function testStateDiffWarmsChangedAccounts() public {
        WarmthTarget target = new WarmthTarget();
        vm.startStateDiffRecording();
        target.set(5);

        vm.cool(address(target));
        vm.getStateDiff();
        vm.assertTrue(isWarm(address(target)));

        vm.cool(address(target));
        vm.getStateDiffJson();
        vm.assertTrue(isWarm(address(target)));

        vm.cool(address(target));
        vm.stopAndReturnStateDiff();
        vm.assertFalse(isWarm(address(target)));
    }

    function testStateDiffRecordingWarmsCalledAccount() public {
        WarmthTarget target = new WarmthTarget();
        vm.cool(address(target));
        vm.startStateDiffRecording();
        target.x();
        vm.assertTrue(isWarm(address(target)));
    }

    function testAccessesDoesNotWarm() public {
        WarmthTarget target = new WarmthTarget();
        vm.record();
        target.x();
        vm.cool(address(target));
        vm.accesses(address(target));
        vm.assertFalse(isWarm(address(target)));
    }

    function testCloneAccountWarmsSourceAndTarget() public {
        vm.etch(address(0x1015), hex"00");
        vm.cool(address(0x1015));
        vm.cloneAccount(address(0x1015), address(0x1016));
        vm.assertTrue(isWarm(address(0x1015)));
        vm.assertTrue(isWarm(address(0x1016)));
    }

    function testCopyStorageWarmsBothAccounts() public {
        vm.etch(address(0x1017), hex"00");
        vm.cool(address(0x1017));
        vm.copyStorage(address(0x1017), address(0x1018));
        vm.assertTrue(isWarm(address(0x1017)));
        vm.assertTrue(isWarm(address(0x1018)));
    }

    function testCopyStorageKeepsSlotWarmth() public {
        address source = address(0x101b);
        vm.store(source, bytes32(uint256(50)), bytes32(uint256(1)));
        vm.store(source, bytes32(uint256(51)), bytes32(uint256(1)));
        vm.cool(source);
        vm.load(source, bytes32(uint256(50)));
        vm.copyStorage(source, address(this));
        vm.assertTrue(isWarmSlot(50));
        vm.assertFalse(isWarmSlot(51));
    }

    function testLoadAllocsWarmsAccounts() public {
        vm.loadAllocs(string.concat(vm.projectRoot(), "/fixtures/Json/test_allocs.json"));
        vm.assertTrue(isWarm(address(0x420)));
    }

    function testSetTip20LogoURIWarmsTokenAndSlots() public {
        vm.setTip20LogoURI(address(0x1019), "ipfs://token");
        vm.assertTrue(isWarm(address(0x1019)));

        vm.setTip20LogoURI(address(this), "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        vm.assertTrue(isWarmSlot(TIP20_LOGO_URI_SLOT));
        vm.assertTrue(isWarmSlot(uint256(keccak256(abi.encode(TIP20_LOGO_URI_SLOT)))));
    }

    function testExecuteTransactionWarmsNewlyAccessedAccounts() public {
        vm.fee(1);
        vm.chainId(1);
        vm.deal(TX_FROM, 1 ether);
        vm.cool(TX_FROM);
        vm.executeTransaction(TRANSFER_TX);
        vm.assertTrue(isWarm(TX_TO));
        // An account the running transaction already loaded keeps its warmth.
        vm.assertFalse(isWarm(TX_FROM));
    }

    function testWarmthRevertsWithCall() public {
        WarmthReverter reverter = new WarmthReverter();
        try reverter.dealThenRevert(address(0x101a)) {} catch {}
        vm.assertFalse(isWarm(address(0x101a)));
    }
}

/// forge-config: default.isolate = false
contract CheatcodeWarmthTest is CheatcodeWarmthBase {}

/// forge-config: default.isolate = true
contract CheatcodeWarmthIsolatedTest is CheatcodeWarmthBase {}
