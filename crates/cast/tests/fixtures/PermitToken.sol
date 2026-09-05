// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// ERC-2612 fixture with optional EIP-5267 discovery and a configurable domain version.
contract TestToken {
    string public constant name = "Permit Token";
    string public version = "1";
    bool public discovery = true;
    mapping(address => uint256) public nonces;
    mapping(address => mapping(address => uint256)) public allowance;
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function setDomain(string calldata newVersion, bool discover) external {
        version = newVersion;
        discovery = discover;
    }

    function DOMAIN_SEPARATOR() public view returns (bytes32) {
        return keccak256(abi.encode(
            keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
            keccak256(bytes(name)), keccak256(bytes(version)), block.chainid, address(this)
        ));
    }

    function eip712Domain() external view returns (
        bytes1, string memory, string memory, uint256, address, bytes32, uint256[] memory
    ) {
        require(discovery, "discovery disabled");
        return (hex"0f", name, version, block.chainid, address(this), bytes32(0), new uint256[](0));
    }

    function permit(address owner, address spender, uint256 value, uint256 deadline,
        uint8 v, bytes32 r, bytes32 s) external {
        require(block.timestamp <= deadline, "expired");
        bytes32 digest = keccak256(abi.encodePacked(hex"1901", DOMAIN_SEPARATOR(), keccak256(abi.encode(
            keccak256("Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"),
            owner, spender, value, nonces[owner]++, deadline
        ))));
        require(owner != address(0) && ecrecover(digest, v, r, s) == owner, "invalid signature");
        allowance[owner][spender] = value;
        emit Approval(owner, spender, value);
    }
}
