// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

/// @notice Anvil activity-simulation target: emits events, churns storage, reverts on demand.
contract Activity {
    event Ping(uint256 indexed id, address indexed sender);
    event Data(uint256 indexed id, bytes payload);
    event Multi(address indexed a, address indexed b, uint256 indexed c, uint256 d);

    uint256 public counter;
    mapping(uint256 => uint256) public values;
    bytes[] public blobs;
    bytes32 public sink;

    function emitEvents(uint256 n) external {
        uint256 id = counter;
        for (uint256 i = 0; i < n; i++) {
            uint256 shape = (id + i) % 3;
            if (shape == 0) emit Ping(id + i, msg.sender);
            else if (shape == 1) emit Data(id + i, abi.encodePacked(id + i, msg.sender));
            else emit Multi(msg.sender, address(this), id + i, block.number);
        }
        counter = id + n;
    }

    function write(uint256 slot, uint256 value) external {
        values[slot] = value;
    }

    function push(bytes calldata data) external {
        blobs.push(data);
    }

    function revertWith(string calldata reason) external pure {
        revert(reason);
    }

    function burnGas(uint256 rounds) external {
        bytes32 acc = sink;
        for (uint256 i = 0; i < rounds; i++) {
            acc = keccak256(abi.encodePacked(acc, i));
        }
        sink = acc;
    }
}
