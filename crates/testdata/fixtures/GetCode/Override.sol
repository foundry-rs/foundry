// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract Override {
    event Payload(address sender, address target, bytes data);

    function emitPayload(address target, bytes calldata message) external payable returns (uint256) {
        emit Payload(msg.sender, target, message);
        return 0;
    }
}
