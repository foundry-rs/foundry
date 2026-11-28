// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Protocol fee configuration. The owner may set any fee; when keeper mode is
/// enabled, the keeper may set the fee up to `MAX_KEEPER_BPS`.
contract FeeConfig {
    uint16 public constant MAX_KEEPER_BPS = 50;

    address public immutable owner;
    address public keeper;
    bool public keeperMode;
    uint16 public feeBps;

    constructor(address owner_, address keeper_) {
        owner = owner_;
        keeper = keeper_;
        feeBps = 25;
    }

    function setKeeperMode(bool enabled) external {
        require(msg.sender == owner, "not owner");
        keeperMode = enabled;
    }

    function setFee(uint16 bps) external {
        require(bps <= 1_000, "fee too high");
        // BUG: the keeper branch checks the mode and cap but never checks
        // `msg.sender == keeper`, so anyone can set small fees in keeper mode.
        require(msg.sender == owner || (keeperMode && bps <= MAX_KEEPER_BPS), "unauthorized");
        feeBps = bps;
    }
}
