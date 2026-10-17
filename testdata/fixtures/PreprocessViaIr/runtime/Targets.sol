// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.22 <0.9.0;

contract DeploymentState {
    address public caller = msg.sender;
    address public origin = tx.origin;
    address public deployedAt = address(this);
    uint256 public value = msg.value;

    constructor() payable {}
}

contract Empty is DeploymentState {
    constructor() payable {}
}

contract Target is DeploymentState {
    uint256 public first;
    uint256 public second;

    constructor(uint256 first_, uint256 second_) payable {
        first = first_;
        second = second_;
    }
}

contract Reverting {
    error ConstructorReverted(uint256 value);

    constructor(uint256 value) {
        revert ConstructorReverted(value);
    }
}
