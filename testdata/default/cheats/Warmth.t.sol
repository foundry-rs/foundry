// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// Returns whether the next access to `target` costs the warm EIP-2929 price (100) rather than the
/// cold one (2600).
function isWarm(address target) view returns (bool) {
    uint256 cost;
    assembly {
        cost := gas()
        let value := balance(target)
        cost := sub(cost, gas())
        // Keeps the optimizer from removing the access.
        if eq(value, 0xdeadbeef) { revert(0, 0) }
    }
    return cost < 2600;
}

/// Returns whether the next access to slot `key` of the running contract costs the warm price (100)
/// rather than the cold one (2100).
function isWarmSlot(uint256 key) view returns (bool) {
    uint256 cost;
    assembly {
        cost := gas()
        let value := sload(key)
        cost := sub(cost, gas())
        if eq(value, 0xdeadbeef) { revert(0, 0) }
    }
    return cost < 2100;
}

contract WarmthTarget {
    uint256 public x = 1;

    function inc() external {
        x++;
    }

    function hasWarmSlot(uint256 key) external view returns (bool) {
        return isWarmSlot(key);
    }
}

contract WarmthPoker {
    function poke(WarmthTarget target) external {
        target.inc();
    }
}

contract WarmthReverter {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function dealThenRevert(address target) external {
        vm.deal(target, 1);
        revert();
    }

    function executeThenRevert(bytes calldata transaction) external {
        vm.executeTransaction(transaction);
        revert();
    }
}

/// Cheatcodes that load an account or a storage slot leave it warm, so the first real access
/// after them pays the warm cost, and the warmth reverts with the enclosing call.
/// forge-config: default.isolate = false
contract CheatcodeWarmthTest is Test {
    // Signed legacy transfer of 17 wei from `TX_FROM` to `TX_TO` on chain 1.
    bytes constant TRANSFER_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";
    address constant TX_FROM = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TX_TO = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;

    // Stored at slot 1, after `IS_TEST` and `_failed`.
    string text = "short";

    function testCheatcodesWarmAccount() public {
        vm.deal(address(0x1001), 1);
        vm.assertTrue(isWarm(address(0x1001)), "deal");
        vm.etch(address(0x1002), hex"00");
        vm.assertTrue(isWarm(address(0x1002)), "etch");
        vm.store(address(0x1003), 0, bytes32(uint256(1)));
        vm.assertTrue(isWarm(address(0x1003)), "store");
        vm.load(address(0x1004), 0);
        vm.assertTrue(isWarm(address(0x1004)), "load");
        vm.setNonce(address(0x1005), 3);
        vm.assertTrue(isWarm(address(0x1005)), "setNonce");
        vm.setNonceUnsafe(address(0x1006), 3);
        vm.assertTrue(isWarm(address(0x1006)), "setNonceUnsafe");
        vm.resetNonce(address(0x1007));
        vm.assertTrue(isWarm(address(0x1007)), "resetNonce");
        vm.getNonce(address(0x1008));
        vm.assertTrue(isWarm(address(0x1008)), "getNonce");
        vm.mockCall(address(0x1009), bytes4(0x12345678), "");
        vm.assertTrue(isWarm(address(0x1009)), "mockCall");
        vm.prank(address(0x100a));
        vm.assertTrue(isWarm(address(0x100a)), "prank sender");
        vm.stopPrank();
        vm.signDelegation(address(0x100b), 0x1001);
        vm.assertTrue(isWarm(vm.addr(0x1001)), "signDelegation");
        vm.attachDelegation(vm.signDelegation(address(0x100b), 0x1002, 0));
        vm.assertTrue(isWarm(vm.addr(0x1002)), "attachDelegation");

        vm.etch(address(0x100c), hex"00");
        vm.etch(address(0x100d), hex"00");
        vm.cool(address(0x100c));
        vm.cool(address(0x100d));
        vm.cloneAccount(address(0x100c), address(0x100e));
        vm.assertTrue(isWarm(address(0x100c)), "cloneAccount source");
        vm.assertTrue(isWarm(address(0x100e)), "cloneAccount target");
        vm.copyStorage(address(0x100d), address(0x100f));
        vm.assertTrue(isWarm(address(0x100d)), "copyStorage from");
        vm.assertTrue(isWarm(address(0x100f)), "copyStorage to");

        vm.loadAllocs(string.concat(vm.projectRoot(), "/fixtures/Json/test_allocs.json"));
        vm.assertTrue(isWarm(address(0x420)), "loadAllocs");
        vm.setTip20LogoURI(address(0x1010), "ipfs://token");
        vm.assertTrue(isWarm(address(0x1010)), "setTip20LogoURI");
        WarmthTarget target = new WarmthTarget();
        vm.cool(address(target));
        vm.getStorageSlots(address(target), "x");
        vm.assertTrue(isWarm(address(target)), "getStorageSlots");

        vm.startBroadcast(address(0x1011));
        vm.stopBroadcast();
        vm.assertTrue(isWarm(address(0x1011)), "broadcast");
    }

    function testCheatcodesStayCold() public {
        vm.assertFalse(isWarm(address(0x1001)), "baseline");
        vm.assertFalse(isWarmSlot(100), "baseline slot");
        vm.prank(address(0x1002), address(0x1003));
        vm.assertFalse(isWarm(address(0x1003)), "prank origin");
        vm.stopPrank();
        // An explicit nonce does not read the authority's account.
        vm.signDelegation(address(0x1004), 0x1001, 0);
        vm.assertFalse(isWarm(vm.addr(0x1001)), "signDelegation with nonce");

        WarmthTarget target = new WarmthTarget();
        vm.startStateDiffRecording();
        target.x();
        vm.cool(address(target));
        vm.stopAndReturnStateDiff();
        vm.assertFalse(isWarm(address(target)), "stopAndReturnStateDiff");
    }

    function testCheatcodesWarmSlot() public {
        vm.store(address(this), bytes32(uint256(101)), bytes32(uint256(1)));
        vm.assertTrue(isWarmSlot(101), "store");
        vm.load(address(this), bytes32(uint256(102)));
        vm.assertTrue(isWarmSlot(102), "load");
        vm.setTip20LogoURI(address(this), "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        vm.assertTrue(isWarmSlot(5), "setTip20LogoURI base slot");
        vm.assertTrue(isWarmSlot(uint256(keccak256(abi.encode(5)))), "setTip20LogoURI data slot");
        vm.cool(address(this));
        vm.getStorageSlots(address(this), "text");
        vm.assertTrue(isWarmSlot(1), "getStorageSlots");

        WarmthTarget source = new WarmthTarget();
        vm.cloneAccount(address(source), address(0x1001));
        vm.assertTrue(WarmthTarget(address(0x1001)).hasWarmSlot(0), "cloneAccount");

        // Copied slots keep the source slots' warmth.
        vm.store(address(0x1002), bytes32(uint256(50)), bytes32(uint256(1)));
        vm.store(address(0x1002), bytes32(uint256(51)), bytes32(uint256(1)));
        vm.cool(address(0x1002));
        vm.load(address(0x1002), bytes32(uint256(50)));
        vm.copyStorage(address(0x1002), address(this));
        vm.assertTrue(isWarmSlot(50), "copyStorage warm slot");
        vm.assertFalse(isWarmSlot(51), "copyStorage cold slot");
    }

    function testWarmthRevertsWithCall() public {
        WarmthReverter reverter = new WarmthReverter();
        try reverter.dealThenRevert(address(0x1001)) {} catch {}
        vm.assertFalse(isWarm(address(0x1001)));
    }

    function testExecuteTransactionWarmth() public {
        vm.chainId(1);
        vm.deal(TX_FROM, 1 ether);
        vm.cool(TX_FROM);
        // Reads slot 0 and `BALANCE(0xbeef)`, then stops.
        vm.etch(TX_TO, hex"5f545061beef315000");
        // The transaction's warmth survives reverts, like its changes.
        WarmthReverter reverter = new WarmthReverter();
        try reverter.executeThenRevert(TRANSFER_TX) {} catch {}
        vm.assertTrue(isWarm(address(0xbeef)), "loaded account");
        vm.assertFalse(isWarm(TX_FROM), "cooled account");
        vm.etch(TX_TO, type(WarmthTarget).runtimeCode);
        vm.assertTrue(WarmthTarget(TX_TO).hasWarmSlot(0), "loaded slot");

        // Accesses the transaction reverted stay cold.
        vm.resetNonce(TX_FROM);
        // Reads `BALANCE(0xcafe)`, then reverts.
        vm.etch(TX_TO, hex"61cafe31505f5ffd");
        try vm.executeTransaction(TRANSFER_TX) {
            // Fails the test if the transaction did not revert.
            revert();
        } catch {}
        vm.assertFalse(isWarm(address(0xcafe)), "reverted access");
    }
}

/// Warmth that isolated calls carry over from their own transactions.
/// forge-config: default.isolate = true
contract CheatcodeWarmthIsolatedTest is Test {
    WarmthTarget counter;
    WarmthPoker poker;

    function setUp() public {
        counter = new WarmthTarget();
        poker = new WarmthPoker();
    }

    function testAppliedPrankWarmsSender() public {
        vm.startPrank(address(0x1001));
        vm.cool(address(0x1001));
        new WarmthTarget();
        vm.stopPrank();
        vm.assertTrue(isWarm(address(0x1001)));
    }

    function testStateDiffRecordingWarmsCalledAccount() public {
        WarmthTarget target = new WarmthTarget();
        vm.cool(address(target));
        vm.startStateDiffRecording();
        target.x();
        vm.assertTrue(isWarm(address(target)));
    }

    // An isolated STATICCALL cools the running transaction's accounts as for a new transaction,
    // the test contract included.
    function testStaticCallCoolsTestContract() public {
        vm.assertTrue(isWarm(address(this)));
        counter.x();
        vm.assertFalse(isWarm(address(this)));
    }

    // An account the STATICCALL cooled stays cold for the calling frame after an isolated CALL
    // accesses it.
    function testStaticCallCooledAccountStaysColdAfterIsolatedCall() public {
        counter.x();
        poker.poke(counter);
        vm.assertFalse(isWarm(address(counter)));
        vm.assertTrue(isWarm(address(counter)));
    }

    // An account the STATICCALL cooled is imported warm by an isolated CREATE once a snapshot
    // revert has dropped it from the running transaction.
    function testStaticCallCooledAccountWarmAfterRevertAndCreate() public {
        uint256 snapshot = vm.snapshotState();
        WarmthTarget created = new WarmthTarget();
        created.x();
        vm.revertToState(snapshot);
        assertEq(address(new WarmthTarget()), address(created));
        vm.assertTrue(isWarm(address(created)));
    }
}
