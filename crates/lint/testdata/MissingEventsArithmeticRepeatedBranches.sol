//@compile-flags: --only-lint missing-events-arithmetic

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

abstract contract RepeatedBranchesBase {
    address public owner = msg.sender;
    uint256 internal fee;
    uint256 internal scratch;

    event Updated();

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function quote(uint256 amount) external view returns (uint256) {
        return amount * fee;
    }

    function branch(bool flag) internal {
        if (flag) scratch = 1;
        if (!flag) scratch = 2;
    }
}

contract MissingEventsArithmeticRepeatedBranches is RepeatedBranchesBase {
    function setFee(uint256 next, bool flag) external onlyOwner {
        fee = next; //~WARN: `fee` is changed without an event but is used in arithmetic
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        fee += 1;
        if (flag) return;
        emit Updated();
    }

    function setFeeWithEvent(uint256 next, bool flag) external onlyOwner {
        fee = next;
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        branch(flag);
        emit Updated();
    }
}
