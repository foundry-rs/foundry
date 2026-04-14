// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

/// @notice Mapping-heavy registry contract. Exercises storage-bound workloads.
contract Registry {
    struct Entry {
        address owner;
        uint256 value;
        uint256 updatedAt;
        bool active;
    }

    mapping(bytes32 => Entry) public entries;
    mapping(address => bytes32[]) public ownerKeys;
    uint256 public totalEntries;

    event Registered(bytes32 indexed key, address indexed owner, uint256 value);
    event Updated(bytes32 indexed key, uint256 oldValue, uint256 newValue);
    event Deactivated(bytes32 indexed key);

    function register(bytes32 key, uint256 value) external {
        require(entries[key].owner == address(0), "Registry: key exists");
        entries[key] = Entry(msg.sender, value, block.timestamp, true);
        ownerKeys[msg.sender].push(key);
        totalEntries++;
        emit Registered(key, msg.sender, value);
    }

    function update(bytes32 key, uint256 newValue) external {
        Entry storage entry = entries[key];
        require(entry.owner == msg.sender, "Registry: not owner");
        require(entry.active, "Registry: inactive");
        uint256 oldValue = entry.value;
        entry.value = newValue;
        entry.updatedAt = block.timestamp;
        emit Updated(key, oldValue, newValue);
    }

    function deactivate(bytes32 key) external {
        Entry storage entry = entries[key];
        require(entry.owner == msg.sender, "Registry: not owner");
        require(entry.active, "Registry: already inactive");
        entry.active = false;
        emit Deactivated(key);
    }

    function batchRegister(bytes32[] calldata keys, uint256[] calldata values) external {
        require(keys.length == values.length, "Registry: length mismatch");
        for (uint256 i = 0; i < keys.length; i++) {
            require(entries[keys[i]].owner == address(0), "Registry: key exists");
            entries[keys[i]] = Entry(msg.sender, values[i], block.timestamp, true);
            ownerKeys[msg.sender].push(keys[i]);
            totalEntries++;
        }
    }

    function getOwnerKeyCount(address owner) external view returns (uint256) {
        return ownerKeys[owner].length;
    }
}
