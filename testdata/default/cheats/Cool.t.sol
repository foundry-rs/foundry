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

    // Only the coinbase the transaction started with stays warm.
    function testCool_recipientMadeCoinbase() public {
        vm.coinbase(address(this));
        vm.cool(address(this));
        assertGe(balanceGas(address(this)), 2600);
    }

    // `vm.accessList` applies to later isolated calls, not to the running transaction.
    function testCool_recipientInConfiguredAccessList() public {
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](1);
        access[0] = Vm.AccessListItem(address(this), new bytes32[](0));
        vm.accessList(access);
        vm.cool(address(this));
        assertGe(balanceGas(address(this)), 2600);
    }

    // Warmth from a frame that reverts is reverted with it.
    function testCool_rewarmedInRevertedFrame() public {
        CoolProbe probe = new CoolProbe();
        vm.cool(address(this));
        try probe.touchAndRevert(address(this)) {} catch {}
        assertGe(balanceGas(address(this)), 2600);
    }

    // A fork switch after cooling keeps the persistent test contract cold.
    function testCool_forkSwitchAfterCool() public {
        vm.createSelectFork("mainnet");
        uint256 other = vm.createFork("mainnet", block.number - 1);
        vm.cool(address(this));
        vm.selectFork(other);
        assertGe(balanceGas(address(this)), 2600);
    }

    // Precompiles and the coinbase the transaction started with stay warm for the whole
    // transaction (EIP-2929, EIP-3651).
    function testCool_keepsPrecompileAndCoinbaseWarm() public {
        address coinbase = block.coinbase;
        vm.coinbase(address(0xbeef));
        vm.cool(address(1));
        vm.cool(coinbase);
        assertLt(balanceGas(address(1)), 2600);
        assertLt(balanceGas(coinbase), 2600);
    }
}

contract CoolProbe is Test {
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
}
