// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {FeeConfig} from "../src/FeeConfig.sol";
import {FuzzBase, vm} from "./utils/FuzzBase.sol";

contract FeeConfigHandler {
    address public constant OWNER = address(0xA11CE);
    address public constant KEEPER = address(0xB0B);

    FeeConfig public immutable config;
    uint16 public expectedFee;

    constructor() {
        config = new FeeConfig(OWNER, KEEPER);
        expectedFee = config.feeBps();
    }

    function ownerSetKeeperMode(bool enabled) external {
        vm.prank(OWNER);
        config.setKeeperMode(enabled);
    }

    function ownerSetFee(uint16 bps) external {
        vm.prank(OWNER);
        config.setFee(bps);
        expectedFee = bps;
    }

    function keeperSetFee(uint16 bps) external {
        vm.prank(KEEPER);
        config.setFee(bps);
        expectedFee = bps;
    }

    function strangerSetFee(address stranger, uint16 bps) external {
        if (stranger == OWNER || stranger == KEEPER) return;
        vm.prank(stranger);
        config.setFee(bps);
    }
}

contract FeeConfigInvariantTest is FuzzBase {
    FeeConfigHandler handler;

    function setUp() public {
        handler = new FeeConfigHandler();
        targetContract(address(handler));
    }

    function invariant_onlyAuthorizedFeeChanges() public view {
        require(handler.config().feeBps() == handler.expectedFee(), "unauthorized fee change");
    }
}
