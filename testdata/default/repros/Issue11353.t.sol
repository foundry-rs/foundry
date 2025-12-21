// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import "utils/Test.sol";

contract Blobhash {
    function getIndices(uint256[] calldata blobIndices) public view returns (bytes32[] memory) {
        bytes32[] memory blobHashes = new bytes32[](blobIndices.length);
        for (uint256 i = 0; i < blobIndices.length; i++) {
            uint256 blobIndex = blobIndices[i];
            bytes32 blobHash = blobhash(blobIndex);
            require(blobHash != 0, "blob not found");
            blobHashes[i] = blobHash;
        }
        return blobHashes;
    }
}

// https://github.com/foundry-rs/foundry/issues/11353
contract Issue11353Test is Test {
    Blobhash public blobhashContract;

    function setUp() public {
        blobhashContract = new Blobhash();
    }

    function test_blobhashes() public {
        uint256[] memory blobIndices = new uint256[](1);
        blobIndices[0] = 0;

        bytes32[] memory blobHashes = new bytes32[](1);
        blobHashes[0] = keccak256(abi.encode(0));
        vm.blobhashes(blobHashes);

        vm.assertEq(blobhashContract.getIndices(blobIndices), blobHashes);
    }
}
