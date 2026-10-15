// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

interface VmComputeCreateAddresses {
    function computeCreateAddress(address deployer, uint256 nonce) external pure returns (address);
    function computeCreate2Address(bytes32 salt, bytes32 initCodeHash, address deployer) external pure returns (address);
    function computeCreate2Address(bytes32 salt, bytes32 initCodeHash) external pure returns (address);
}

contract ComputeCreateAddressesTest {
    VmComputeCreateAddresses constant vm =
        VmComputeCreateAddresses(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant DEFAULT_CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    // CREATE vectors are the last 20 bytes of keccak256 of independently encoded RLP inputs.
    function testCreateNonceZero() public view {
        assertAddress(
            abi.encodeCall(vm.computeCreateAddress, (address(0x1234), 0)), 0xFa0F05A76eBcbf400856cCcCF29FF66d8acC843f
        );
    }

    function testCreateNonceNormal() public view {
        assertAddress(
            abi.encodeCall(vm.computeCreateAddress, (address(0x1234), 42)), 0x8460fDFbEed93E0434238b5e706c8F3145568D51
        );
    }

    function testCreateNonceMax() public view {
        assertAddress(
            abi.encodeCall(vm.computeCreateAddress, (address(0x1234), type(uint64).max)),
            0x43626B4fBD5B317FC6902D6D5fFb184037CD6784
        );
    }

    function testCreateNonceOverflow() public view {
        assertNonceError(uint256(1) << 64);
    }

    function testCreateNonceUint256Max() public view {
        assertNonceError(type(uint256).max);
    }

    // CREATE2 vectors use independently hashed 0xff || deployer || salt || initCodeHash inputs.
    function testCreate2DefaultDeployerZeroFields() public view {
        assertDefaultCreate2(bytes32(0), bytes32(0), 0x778a4590f20dB0c23cB7c1beFC8dA04549f2aa95);
    }

    function testCreate2DefaultDeployerNonzeroFields() public view {
        assertDefaultCreate2(
            bytes32(uint256(0xcafe)), keccak256(hex"60006000f3"), 0x593F48fba98D1F1a108B7bBA75DC6841E5609AE4
        );
    }

    function testCreate2CustomDeployerZeroSalt() public view {
        // EIP-1014 example 0: zero deployer and salt, with init code 0x00.
        assertAddress(
            abi.encodeWithSignature(
                "computeCreate2Address(bytes32,bytes32,address)", bytes32(0), keccak256(hex"00"), address(0)
            ),
            0x4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38
        );
    }

    function testCreate2CustomDeployerNonzeroSalt() public view {
        assertAddress(
            abi.encodeWithSignature(
                "computeCreate2Address(bytes32,bytes32,address)",
                bytes32(uint256(0xbeef)),
                keccak256(""),
                0xdEADBEeF00000000000000000000000000000000
            ),
            0x27CCd58fF0E3ceC390A143F6FF8524850606f113
        );
    }

    function assertAddress(bytes memory input, address expected) internal view {
        (bool success, bytes memory data) = address(vm).staticcall(input);
        require(success, "address computation failed");
        require(keccak256(data) == keccak256(abi.encode(expected)), "unexpected address bytes");
    }

    function assertDefaultCreate2(bytes32 salt, bytes32 initCodeHash, address expected) internal view {
        assertAddress(abi.encodeWithSignature("computeCreate2Address(bytes32,bytes32)", salt, initCodeHash), expected);
        assertAddress(
            abi.encodeWithSignature(
                "computeCreate2Address(bytes32,bytes32,address)", salt, initCodeHash, DEFAULT_CREATE2_DEPLOYER
            ),
            expected
        );
    }

    function assertNonceError(uint256 nonce) internal view {
        (bool success, bytes memory data) =
            address(vm).staticcall(abi.encodeCall(vm.computeCreateAddress, (address(0), nonce)));
        require(!success, "out-of-range nonce succeeded");
        bytes memory expected =
            abi.encodeWithSignature("CheatcodeError(string)", "vm.computeCreateAddress: nonce must be less than 2^64");
        require(keccak256(data) == keccak256(expected), "unexpected nonce error bytes");
    }
}
