// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

contract HedgerPool {
    struct Position {
        address hedger;
        uint96 size;
        uint96 filled;
        uint96 margin;
        uint96 price;
        uint32 time;
        uint32 updated;
        int128 unrealized;
        int128 realized;
        uint16 leverage;
        bool active;
        uint128 backed;
        uint64 opened;
    }
    mapping(uint256 => Position) public positions;
    uint128 public totalMargin;

    constructor(address timeProvider) {
        totalMargin = uint128(uint160(timeProvider));
    }

    function exitHedgePosition(uint256) external pure returns (int256) {
        return 0;
    }
    function upgradeToAndCall(address, bytes memory) external payable {}
}

contract Empty {
    constructor() payable {}
}
