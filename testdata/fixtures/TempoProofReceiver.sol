// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import {TempoProofVerifier} from "./TempoProofVerifier.sol";
import {LibBLS12381 as BLS} from "commonware/certificate/LibBLS12381.sol";

/// @dev Test receiver that accepts proofs only under its deployment-time development identity.
contract TempoProofReceiver {
    BLS.G2Point private trustedKey;
    TempoProofVerifier private immutable verifier;

    error InvalidProof();

    event ProofAccepted(
        bytes32 indexed digest, uint256 indexed height, address indexed account, bytes32 slot, uint256 value
    );

    constructor(BLS.G2Point memory key) {
        trustedKey = key;
        verifier = new TempoProofVerifier();
    }

    /// @dev Revert on invalid evidence or a mismatched value; record only verified claims.
    function submit(
        TempoProofVerifier.Evidence calldata evidence,
        uint256 expectedHeight,
        address account,
        bytes32 slot,
        uint256 expectedValue
    ) external {
        if (!verifier.verify(trustedKey, evidence, expectedHeight, account, slot, expectedValue)) {
            revert InvalidProof();
        }
        emit ProofAccepted(evidence.subject.payload, expectedHeight, account, slot, expectedValue);
    }
}
