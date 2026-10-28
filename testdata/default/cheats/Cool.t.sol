// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract CoolTest is Test {
    uint256 public slot0 = 1;

    function testCool_SLOAD_normal() public {
        uint256 startGas;
        uint256 endGas;
        uint256 val;
        uint256 beforeCoolGas;
        uint256 noCoolGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        beforeCoolGas = startGas - endGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        noCoolGas = startGas - endGas;

        assertGt(beforeCoolGas, noCoolGas);
    }

    function testCool_SLOAD() public {
        uint256 startGas;
        uint256 endGas;
        uint256 val;
        uint256 beforeCoolGas;
        uint256 afterCoolGas;
        uint256 noCoolGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        beforeCoolGas = startGas - endGas;

        vm.cool(address(this));

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        afterCoolGas = startGas - endGas;

        assertEq(beforeCoolGas, afterCoolGas);

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        noCoolGas = startGas - endGas;

        assertGt(beforeCoolGas, noCoolGas);
    }
}

contract CoolTarget {
    uint256 public value;

    function set(uint256 newValue) external {
        value = newValue;
    }
}

/// forge-config: default.isolate = false
contract CoolPersistenceTest is Test {
    CoolTarget preexisting = new CoolTarget();
    CoolTarget created;

    function setUp() public {
        created = new CoolTarget();
        created.set(1);
        preexisting.set(2);
        vm.cool(address(created));
        vm.cool(address(preexisting));
    }

    function testCoolKeepsSetUpState() public {
        assertEq(preexisting.value(), 2);
        assertEq(created.value(), 1);
    }

    function testCool_EXTCODESIZE() public {
        address target = address(preexisting);
        uint256 size = target.code.length;
        assertGt(size, 0);

        vm.cool(target);

        uint256 gasUsed;
        uint256 cooledSize;
        assembly {
            let start := gas()
            cooledSize := extcodesize(target)
            gasUsed := sub(start, gas())
        }

        assertEq(cooledSize, size);
        assertGe(gasUsed, 2600);
    }

    function balanceGas(address target) internal view returns (uint256 gasUsed) {
        assembly {
            let start := gas()
            let targetBalance := balance(target)
            gasUsed := sub(start, gas())
            // Keeps the `BALANCE` from being optimized out.
            if eq(targetBalance, 0xdeadbeef) { revert(0, 0) }
        }
    }

    // The test contract is the transaction recipient, which is warm from the transaction start.
    function testCool_recipient() public {
        vm.cool(address(this));
        assertGe(balanceGas(address(this)), 2600);
        assertLt(balanceGas(address(this)), 2600);
    }

    // Like REVM's `WarmAddresses`, precompiles and the coinbase the transaction started with stay
    // warm (EIP-2929, EIP-3651), while `vm.coinbase` and `vm.accessList` only apply to later
    // transactions, including after an isolated call installed and restored its own.
    function testCool_keepsTransactionStartWarmth() public {
        address coinbase = block.coinbase;
        vm.coinbase(address(this));
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](1);
        access[0] = Vm.AccessListItem(address(this), new bytes32[](0));
        vm.accessList(access);
        CoolProbe probe = new CoolProbe();
        try probe.touchAndRevert(address(0)) {} catch {}

        vm.cool(address(this));
        vm.cool(coinbase);
        vm.cool(address(1));
        assertGe(balanceGas(address(this)), 2600);
        assertLt(balanceGas(coinbase), 2600);
        assertLt(balanceGas(address(1)), 2600);
    }

    // Fork switches keep the cooled persistent test contract cold, before and after the running
    // transaction's warmth was carried across a switch.
    function testCool_forkSwitchAfterCool() public {
        uint256 first = vm.createSelectFork("mainnet");
        uint256 second = vm.createFork("mainnet", block.number - 1);
        vm.cool(address(this));
        vm.selectFork(second);
        assertGe(balanceGas(address(this)), 2600);
        vm.selectFork(first);
        vm.cool(address(this));
        vm.selectFork(second);
        assertGe(balanceGas(address(this)), 2600);
    }

    // Like REVM's per-fork journals, cooling a non-persistent account only cools it on the active
    // fork.
    function testCool_forkNonPersistentPerContext() public {
        address target = address(0xbeef);
        uint256 first = vm.createSelectFork("mainnet");
        uint256 second = vm.createFork("mainnet", block.number - 1);
        balanceGas(target);
        vm.selectFork(second);
        balanceGas(target);
        vm.selectFork(first);
        vm.cool(target);
        assertGe(balanceGas(target), 2600);
        vm.cool(target);
        vm.selectFork(second);
        assertLt(balanceGas(target), 2600);
        vm.selectFork(first);
        assertGe(balanceGas(target), 2600);
    }

    // Like master's journal load, a broadcast call warms the broadcaster.
    function testCool_broadcastWarmsBroadcaster() public {
        address broadcaster = address(0xb0b);
        vm.startBroadcast(broadcaster);
        vm.cool(broadcaster);
        preexisting.set(3);
        vm.stopBroadcast();
        assertLt(balanceGas(broadcaster), 2600);
    }

    // A transaction run with `vm.executeTransaction` keeps the coinbase it started with warm.
    function testCool_keepsExecutedTransactionCoinbaseWarm() public {
        CoolCoinbase target = new CoolCoinbase();
        vm.coinbase(address(0xbeef));
        uint256 key = 0xA11CE;
        address signer = vm.addr(key);
        vm.executeTransaction(signLegacy(key, vm.getNonce(signer), address(target), abi.encodeCall(target.run, ())));
        assertLt(target.gasUsed(), 2600);
    }

    // A mined transaction replayed with `vm.transact` keeps its own block's coinbase warm.
    function testCool_forkTransactKeepsCoinbaseWarm() public {
        vm.createSelectFork("mainnet", 16260609);
        // The USDT transfer https://etherscan.io/tx/0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c
        address usdt = 0xdAC17F958D2ee523a2206206994597C13D831ec7;
        vm.etch(usdt, type(CoolCoinbase).runtimeCode);
        vm.makePersistent(usdt);
        vm.allowCheatcodes(usdt);
        vm.transact(0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);
        assertLt(CoolCoinbase(usdt).gasUsed(), 2600);
    }

    function signLegacy(uint256 key, uint256 nonce, address to, bytes memory data) internal returns (bytes memory) {
        bytes[] memory fields = new bytes[](9);
        fields[0] = rlpUint(nonce);
        fields[1] = rlpUint(0);
        fields[2] = rlpUint(1_000_000);
        fields[3] = abi.encodePacked(to);
        fields[4] = rlpUint(0);
        fields[5] = data;
        fields[6] = rlpUint(block.chainid);
        fields[7] = rlpUint(0);
        fields[8] = rlpUint(0);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, keccak256(vm.toRlp(fields)));
        fields[6] = rlpUint(block.chainid * 2 + 8 + v);
        fields[7] = rlpUint(uint256(r));
        fields[8] = rlpUint(uint256(s));
        return vm.toRlp(fields);
    }

    function rlpUint(uint256 value) internal pure returns (bytes memory out) {
        bytes memory word = abi.encodePacked(value);
        uint256 start;
        while (start < 32 && word[start] == 0) start++;
        out = new bytes(32 - start);
        for (uint256 i; i < out.length; i++) {
            out[i] = word[start + i];
        }
    }
}

contract CoolCoinbase is Test {
    uint256 public gasUsed;

    function run() public {
        address target = block.coinbase;
        vm.cool(target);
        uint256 used;
        assembly {
            let start := gas()
            let targetBalance := balance(target)
            used := sub(start, gas())
            if eq(targetBalance, 0xdeadbeef) { revert(0, 0) }
        }
        gasUsed = used;
    }

    fallback() external {
        run();
    }
}

contract CoolProbe is Test {
    uint256 public touched;

    function touch(address target) external {
        touched = target.balance;
    }

    function touchAndBroadcast(address target, bytes calldata signedTx) external {
        touched = target.balance;
        vm.broadcastRawTransaction(signedTx);
    }

    function touchAndRevert(address target) external view {
        require(target.balance == 0xdeadbeef);
    }

    function coolAndMeasure(address target, Vm.AccessListItem[] memory access) external returns (uint256 gasUsed) {
        // Changing the configured access list leaves the running transaction's in place.
        if (access.length == 0) {
            vm.noAccessList();
        } else {
            vm.accessList(access);
        }
        vm.cool(target);
        assembly {
            let start := gas()
            let targetBalance := balance(target)
            gasUsed := sub(start, gas())
            if eq(targetBalance, 0xdeadbeef) { revert(0, 0) }
        }
    }
}

/// forge-config: default.isolate = true
contract CoolPersistenceIsolatedTest is CoolPersistenceTest {
    // Access-list entries of an isolated call's transaction stay warm (EIP-2930).
    function testCool_keepsAccessListWarm() public {
        CoolProbe probe = new CoolProbe();
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](1);
        access[0] = Vm.AccessListItem(address(0xbeef), new bytes32[](0));
        Vm.AccessListItem[] memory replacement = new Vm.AccessListItem[](1);
        replacement[0] = Vm.AccessListItem(address(0xcafe), new bytes32[](0));

        vm.accessList(access);
        assertLt(probe.coolAndMeasure(address(0xbeef), new Vm.AccessListItem[](0)), 2600);
        vm.accessList(access);
        assertLt(probe.coolAndMeasure(address(0xbeef), replacement), 2600);
    }

    // As on master, an account the running transaction holds cold stays cold after a successful
    // isolated call touched it.
    function testCool_staysColdAfterIsolatedCall() public {
        CoolProbe probe = new CoolProbe();
        address target = address(preexisting);
        balanceGas(target);
        vm.cool(target);
        probe.touch(target);
        assertGe(balanceGas(target), 2600);
    }

    // As on master, an isolated call that broadcasts a transaction keeps the caller's access list
    // and the accounts it cooled cold.
    function testCool_staysColdAfterIsolatedBroadcast() public {
        CoolProbe probe = new CoolProbe();
        address target = address(preexisting);
        balanceGas(target);
        vm.cool(target);
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](1);
        access[0] = Vm.AccessListItem(address(0xcafe), new bytes32[](0));
        vm.accessList(access);
        uint256 key = 0xA11CE;
        probe.touchAndBroadcast(target, signLegacy(key, vm.getNonce(vm.addr(key)), address(0xbeef), ""));
        assertGe(balanceGas(address(0xcafe)), 2600);
        assertGe(balanceGas(target), 2600);
    }
}
