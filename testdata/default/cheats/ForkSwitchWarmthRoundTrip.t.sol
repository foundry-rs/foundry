// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = false
contract ForkSwitchWarmthRoundTripTest is Test {
    // DAI, whose `totalSupply()` reads a storage slot.
    address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;

    uint256 mainnetFork;
    uint256 optimismFork;

    function setUp() public {
        mainnetFork = vm.createFork("mainnet");
        optimismFork = vm.createFork("optimism");
    }

    // A fork keeps the warmth of the running transaction across later switches, including visits
    // that do not touch the warm account or slot again.
    function testForkWarmthSurvivesUntouchedVisit() public {
        vm.selectFork(mainnetFork);
        address account = address(0xBEEF);
        isCold(account);
        isColdSlot();

        vm.selectFork(optimismFork);
        vm.selectFork(mainnetFork);
        vm.selectFork(optimismFork);
        vm.selectFork(mainnetFork);

        assertTrue(!isCold(account), "account warmth was dropped");
        assertTrue(!isColdSlot(), "slot warmth was dropped");
    }

    function isCold(address account) internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let accountBalance := balance(account)
            cost := sub(cost, gas())
            if eq(accountBalance, 0xdeadbeef) { revert(0, 0) }
        }
        return cost > 2000;
    }

    function isColdSlot() internal view returns (bool) {
        uint256 cost;
        address dai = DAI;
        assembly {
            mstore(0, 0x18160ddd) // totalSupply()
            cost := gas()
            let success := staticcall(gas(), dai, 28, 4, 0, 32)
            cost := sub(cost, gas())
            if iszero(success) { revert(0, 0) }
        }
        // Reading the slot of a warm account through a warm call costs well under a cold access.
        return cost > 2000;
    }
}
