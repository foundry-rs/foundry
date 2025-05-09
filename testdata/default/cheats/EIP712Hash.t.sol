// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract EIP712HashTypeCall is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    bytes32 typeHash;

    // CANONICAL TYPES
    bytes32 public constant _PERMIT_DETAILS_TYPEHASH = keccak256(
        "PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
    );
    bytes32 public constant _PERMIT_SINGLE_TYPEHASH = keccak256(
        "PermitSingle(PermitDetails details,address spender,uint256 sigDeadline)PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
    );
    bytes32 public constant _PERMIT_BATCH_TRANSFER_FROM_TYPEHASH = keccak256(
        "PermitBatchTransferFrom(TokenPermissions[] permitted,address spender,uint256 nonce,uint256 deadline)TokenPermissions(address token,uint256 amount)"
    );

    function test_canHashCanonicalTypes() public {
        typeHash = vm.eip712HashType("PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)");
        assertEq(typeHash, _PERMIT_DETAILS_TYPEHASH);

        typeHash = vm.eip712HashType(
            "PermitSingle(PermitDetails details,address spender,uint256 sigDeadline)PermitDetails(address token,uint160 amount,uint48 expiration,uint48 nonce)"
        );
        assertEq(typeHash, _PERMIT_SINGLE_TYPEHASH);

        typeHash = vm.eip712HashType(
            "PermitBatchTransferFrom(TokenPermissions[] permitted,address spender,uint256 nonce,uint256 deadline)TokenPermissions(address token,uint256 amount)"
        );
        assertEq(typeHash, _PERMIT_BATCH_TRANSFER_FROM_TYPEHASH);
    }

    function test_canHashMessyTypes() public {
        typeHash = vm.eip712HashType("PermitDetails(address token, uint160 amount, uint48 expiration, uint48 nonce)");
        assertEq(typeHash, _PERMIT_DETAILS_TYPEHASH);

        typeHash = vm.eip712HashType(
            "PermitDetails(address token, uint160 amount, uint48 expiration, uint48 nonce) PermitSingle(PermitDetails details, address spender, uint256 sigDeadline)"
        );
        assertEq(typeHash, _PERMIT_SINGLE_TYPEHASH);

        typeHash = vm.eip712HashType(
            "TokenPermissions(address token, uint256 amount) PermitBatchTransferFrom(TokenPermissions[] permitted, address spender, uint256 nonce, uint256 deadline)"
        );
        assertEq(typeHash, _PERMIT_BATCH_TRANSFER_FROM_TYPEHASH);
    }

    function testRevert_cannotHashTypesWithMissingComponents() public {
        vm._expectCheatcodeRevert();
        typeHash = vm.eip712HashType(
            "PermitSingle(PermitDetails details, address spender, uint256 sigDeadline)"
        );
    }
}
