// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import {LibBLS12381 as BLS} from "commonware/certificate/LibBLS12381.sol";
import {LibBLS12381Threshold as Threshold} from "commonware/certificate/LibBLS12381Threshold.sol";
import {LibSimplex as Simplex} from "commonware/simplex/LibSimplex.sol";
import {LibSimplexBLS12381Threshold as Scheme} from "commonware/simplex/LibSimplexBLS12381Threshold.sol";
import {LibCodec} from "commonware/codec/LibCodec.sol";
import {MerkleTrie} from "src/libraries/trie/MerkleTrie.sol";
import {RLPReader} from "src/libraries/rlp/RLPReader.sol";
import {Bytes} from "src/libraries/Bytes.sol";

/// @dev Test acceptance adapter, using upstream Commonware cryptography and Optimism MPT verification.
contract TempoProofVerifier {
    struct Evidence {
        bytes rawHeader;
        Simplex.Subject subject;
        bytes vote;
        bytes seed;
        bytes[] accountProof;
        bytes[] storageProof;
    }

    function verify(
        BLS.G2Point memory trustedKey,
        Evidence calldata evidence,
        uint256 expectedHeight,
        address account,
        bytes32 slot,
        uint256 expectedValue
    ) public view returns (bool) {
        RLPReader.RLPItem[] memory header = _header(trustedKey, evidence, expectedHeight);
        bytes memory stateRoot = RLPReader.readBytes(header[3]);
        require(stateRoot.length == 32, "state root length");
        bytes memory accountRlp = MerkleTrie.get(
            abi.encodePacked(keccak256(abi.encodePacked(account))),
            evidence.accountProof,
            abi.decode(stateRoot, (bytes32))
        );
        RLPReader.RLPItem[] memory accountFields = RLPReader.readList(accountRlp);
        require(accountFields.length == 4, "account fields");
        bytes memory storageRoot = RLPReader.readBytes(accountFields[2]);
        require(storageRoot.length == 32, "storage root length");
        bytes memory valueRlp = MerkleTrie.get(
            abi.encodePacked(keccak256(abi.encodePacked(slot))),
            evidence.storageProof,
            abi.decode(storageRoot, (bytes32))
        );
        return _integer(RLPReader.readBytes(valueRlp)) == expectedValue;
    }

    /// @dev Authenticate the next 1-of-1 committee from the old key's signed boundary header.
    function verifyHandoff(
        BLS.G2Point memory oldKey,
        Evidence calldata boundary,
        uint256 boundaryHeight,
        BLS.G2Point memory nextKey,
        uint64 nextEpoch
    ) public view returns (bool) {
        RLPReader.RLPItem[] memory header = _header(oldKey, boundary, boundaryHeight);
        // Reject malformed coordinates before compressing the supplied key for comparison.
        BLS.G2Point memory infinity;
        (bool validKey,) = BLS.addG2(nextKey, infinity);
        require(validKey, "next key point");
        bytes memory dkg = RLPReader.readBytes(header[12]);
        require(nextEpoch == boundary.subject.epoch + 1, "next epoch");
        bytes memory epoch = LibCodec.encodeVarint(nextEpoch);
        require(keccak256(Bytes.slice(dkg, 0, epoch.length)) == keccak256(epoch), "announced epoch");
        // Commonware 2026.9 Output: summary32, mode0, total_u32_be=1, polynomial_len=1, key96.
        uint256 offset = epoch.length + 32;
        require(bytes6(Bytes.slice(dkg, offset, 6)) == hex"000000000101", "one-participant DKG");
        offset += 6;
        require(keccak256(Bytes.slice(dkg, offset, 96)) == keccak256(_compressed(nextKey)), "announced key");
        return true;
    }

    function _header(BLS.G2Point memory trustedKey, Evidence calldata evidence, uint256 expectedHeight)
        private
        view
        returns (RLPReader.RLPItem[] memory header)
    {
        require(evidence.subject.kind == Simplex.Kind.Finalization, "finalization required");
        require(keccak256(evidence.rawHeader) == evidence.subject.payload, "header digest");
        require(Scheme.verifyMinSig(evidence.vote, trustedKey, bytes("TEMPO"), evidence.subject), "vote signature");
        bytes memory round = bytes.concat(
            LibCodec.encodeVarint(evidence.subject.epoch), LibCodec.encodeVarint(evidence.subject.viewNumber)
        );
        require(Threshold.verifyMinSig(evidence.seed, trustedKey, bytes("TEMPO_SEED"), round), "seed signature");

        RLPReader.RLPItem[] memory outer = RLPReader.readList(evidence.rawHeader);
        require(outer.length == 5, "native Tempo header");
        header = RLPReader.readList(outer[3]);
        require(_integer(RLPReader.readBytes(header[8])) == expectedHeight, "block height");
        RLPReader.RLPItem[] memory context = RLPReader.readList(outer[4]);
        require(context.length == 4, "consensus context");
        require(_integer(RLPReader.readBytes(context[0])) == evidence.subject.epoch, "context epoch");
        require(_integer(RLPReader.readBytes(context[1])) == evidence.subject.viewNumber, "context view");
        require(_integer(RLPReader.readBytes(context[2])) == evidence.subject.parent, "context parent");
    }

    function _integer(bytes memory value) private pure returns (uint256 result) {
        require(value.length <= 32, "integer length");
        for (uint256 i; i < value.length; ++i) {
            result = (result << 8) | uint8(value[i]);
        }
    }

    /// @dev Reverse the off-chain decompression so the authenticated compressed DKG key binds the EVM key.
    function _compressed(BLS.G2Point memory key) private pure returns (bytes memory encoded) {
        encoded = abi.encodePacked(uint128(uint256(key.xC1Hi)), key.xC1Lo, uint128(uint256(key.xC0Hi)), key.xC0Lo);
        bytes32 yHi = key.yC1Hi;
        bytes32 yLo = key.yC1Lo;
        if (yHi == 0 && yLo == 0) {
            yHi = key.yC0Hi;
            yLo = key.yC0Lo;
        }
        uint256 halfHi = 0x0d0088f51cbff34d258dd3db21a5d66b;
        uint256 halfLo = 0xb23ba5c279c2895fb39869507b587b120f55ffff58a9ffffdcff7fffffffd555;
        bool sign = uint256(yHi) > halfHi || (uint256(yHi) == halfHi && uint256(yLo) > halfLo);
        encoded[0] |= sign ? bytes1(0xa0) : bytes1(0x80);
    }
}

contract ProofStore {
    uint256 public value;

    function set(uint256 next) external {
        value = next;
    }
}
