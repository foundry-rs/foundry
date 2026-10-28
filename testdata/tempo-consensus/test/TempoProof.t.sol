// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import {TempoProofVerifier} from "../src/TempoProofVerifier.sol";
import {LibBLS12381 as BLS} from "commonware/certificate/LibBLS12381.sol";
import {LibSimplex as Simplex} from "commonware/simplex/LibSimplex.sol";

interface Vm {
    function envOr(string calldata name, string calldata defaultValue) external view returns (string memory);
    function readFile(string calldata path) external view returns (string memory);
    function parseJsonBytes(string calldata json, string calldata key) external pure returns (bytes memory);
    function parseJsonBytesArray(string calldata json, string calldata key) external pure returns (bytes[] memory);
    function parseJsonBytes32(string calldata json, string calldata key) external pure returns (bytes32);
    function parseJsonUint(string calldata json, string calldata key) external pure returns (uint256);
    function parseJsonAddress(string calldata json, string calldata key) external pure returns (address);
    function parseJsonBool(string calldata json, string calldata key) external pure returns (bool);
}

contract TempoProofTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    TempoProofVerifier private verifier;
    BLS.G2Point private trustedKey;
    BLS.G2Point private mainnetKey;
    TempoProofVerifier.Evidence private evidence;
    address private account;
    uint256 private height;
    bool private hasRotation;
    BLS.G2Point private initialKey;
    TempoProofVerifier.Evidence private boundary;
    uint256 private boundaryHeight;

    function setUp() public {
        string memory json = vm.readFile(vm.envOr("TEMPO_PROOF_FIXTURE", string("fixtures/standalone.json")));
        verifier = new TempoProofVerifier();
        trustedKey = abi.decode(vm.parseJsonBytes(json, ".key"), (BLS.G2Point));
        mainnetKey = abi.decode(vm.parseJsonBytes(json, ".mainnetKey"), (BLS.G2Point));
        account = vm.parseJsonAddress(json, ".account");
        height = vm.parseJsonUint(json, ".height");
        evidence = _readEvidence(json, "");
        hasRotation = vm.parseJsonBool(json, ".hasRotation");
        if (hasRotation) {
            initialKey = abi.decode(vm.parseJsonBytes(json, ".boundary.key"), (BLS.G2Point));
            boundaryHeight = vm.parseJsonUint(json, ".boundary.height");
            boundary = _readEvidence(json, ".boundary");
        }
    }

    function _readEvidence(string memory json, string memory prefix)
        private
        pure
        returns (TempoProofVerifier.Evidence memory parsed)
    {
        parsed.rawHeader = vm.parseJsonBytes(json, string.concat(prefix, ".rawHeader"));
        parsed.subject = Simplex.Subject({
            kind: Simplex.Kind.Finalization,
            epoch: uint64(vm.parseJsonUint(json, string.concat(prefix, ".epoch"))),
            viewNumber: uint64(vm.parseJsonUint(json, string.concat(prefix, ".view"))),
            parent: uint64(vm.parseJsonUint(json, string.concat(prefix, ".parent"))),
            payload: vm.parseJsonBytes32(json, string.concat(prefix, ".digest"))
        });
        parsed.vote = vm.parseJsonBytes(json, string.concat(prefix, ".vote"));
        parsed.seed = vm.parseJsonBytes(json, string.concat(prefix, ".seed"));
        parsed.accountProof = vm.parseJsonBytesArray(json, string.concat(prefix, ".accountProof"));
        parsed.storageProof = vm.parseJsonBytesArray(json, string.concat(prefix, ".storageProof"));
    }

    function test_certified_storage_proof() public view {
        require(verifier.verify(trustedKey, evidence, height, account, bytes32(0), 42), "accepted value");
    }

    function test_reject_wrong_value() public view {
        require(!verifier.verify(trustedKey, evidence, height, account, bytes32(0), 43), "wrong value accepted");
    }

    function test_reject_changed_header() public {
        evidence.rawHeader[10] = bytes1(uint8(evidence.rawHeader[10]) ^ 1);
        _reject();
    }

    function test_reject_changed_digest() public {
        evidence.subject.payload ^= bytes32(uint256(1));
        _reject();
    }

    function test_reject_changed_account_proof() public {
        evidence.accountProof[0][10] = bytes1(uint8(evidence.accountProof[0][10]) ^ 1);
        _reject();
    }

    function test_reject_changed_storage_proof() public {
        evidence.storageProof[0][10] = bytes1(uint8(evidence.storageProof[0][10]) ^ 1);
        _reject();
    }

    function test_reject_mainnet_trusted_key() public {
        trustedKey = mainnetKey;
        _reject();
    }

    function test_reject_wrong_account() public {
        account = address(uint160(account) ^ 1);
        _reject();
    }

    function test_reject_changed_vote() public {
        evidence.vote = evidence.seed;
        _reject();
    }

    function test_reject_changed_seed() public {
        evidence.seed = evidence.vote;
        _reject();
    }

    function test_reject_wrong_height() public {
        ++height;
        _reject();
    }

    function test_rotation_authenticates_next_key_and_storage() public view {
        require(hasRotation, "rotation fixture required");
        require(
            verifier.verifyAfterHandoff(
                initialKey, boundary, boundaryHeight, trustedKey, evidence, height, account, bytes32(0), 42
            ),
            "authenticated rotated storage"
        );
    }

    function test_rotation_rejects_old_key() public {
        require(hasRotation, "rotation fixture required");
        trustedKey = initialKey;
        _reject();
    }

    function test_rotation_rejects_changed_boundary() public {
        require(hasRotation, "rotation fixture required");
        boundary.rawHeader[10] = bytes1(uint8(boundary.rawHeader[10]) ^ 1);
        _rejectHandoff(trustedKey, evidence.subject.epoch);
    }

    function test_rotation_rejects_unannounced_key() public view {
        require(hasRotation, "rotation fixture required");
        _rejectHandoff(mainnetKey, evidence.subject.epoch);
    }

    function test_rotation_rejects_wrong_epoch() public view {
        require(hasRotation, "rotation fixture required");
        _rejectHandoff(trustedKey, evidence.subject.epoch + 1);
    }

    function test_rotation_rejects_malformed_next_key() public view {
        require(hasRotation, "rotation fixture required");
        BLS.G2Point memory changed = trustedKey;
        changed.yC0Lo ^= bytes32(uint256(1));
        _rejectHandoff(changed, evidence.subject.epoch);
    }

    function test_rotation_rejects_noncanonical_next_key() public view {
        require(hasRotation, "rotation fixture required");
        BLS.G2Point memory changed = trustedKey;
        changed.xC0Hi |= bytes32(uint256(1) << 128);
        _rejectHandoff(changed, evidence.subject.epoch);
    }

    function _rejectHandoff(BLS.G2Point memory nextKey, uint64 nextEpoch) private view {
        try verifier.verifyHandoff(initialKey, boundary, boundaryHeight, nextKey, nextEpoch) returns (bool valid) {
            require(!valid, "invalid handoff accepted");
        } catch {}
    }

    function _reject() private view {
        try verifier.verify(trustedKey, evidence, height, account, bytes32(0), 42) returns (bool valid) {
            require(!valid, "invalid evidence accepted");
        } catch {}
    }

    function test_reject_wrong_slot() public view {
        try verifier.verify(trustedKey, evidence, height, account, bytes32(uint256(1)), 42) returns (bool valid) {
            require(!valid, "wrong slot accepted");
        } catch {}
    }

    function test_reject_changed_epoch() public {
        ++evidence.subject.epoch;
        _reject();
    }

    function test_reject_changed_view() public {
        ++evidence.subject.viewNumber;
        _reject();
    }

    function test_reject_changed_parent() public {
        ++evidence.subject.parent;
        _reject();
    }

    function test_reject_nonfinalization() public {
        evidence.subject.kind = Simplex.Kind.Notarization;
        _reject();
    }

    function test_reject_truncated_vote() public {
        evidence.vote.pop();
        _reject();
    }

    function test_reject_truncated_seed() public {
        evidence.seed.pop();
        _reject();
    }

    function test_reject_infinity_vote() public {
        evidence.vote = new bytes(96);
        _reject();
    }

    function test_reject_infinity_seed() public {
        evidence.seed = new bytes(96);
        _reject();
    }

    function test_rotation_rejects_unauthenticated_boundary() public {
        require(hasRotation, "rotation fixture required");
        initialKey = mainnetKey;
        _rejectAfterHandoff();
    }

    function test_rotation_rejects_changed_boundary_vote() public {
        require(hasRotation, "rotation fixture required");
        boundary.vote = boundary.seed;
        _rejectAfterHandoff();
    }

    function test_rotation_rejects_changed_successor_epoch() public {
        require(hasRotation, "rotation fixture required");
        ++evidence.subject.epoch;
        _rejectAfterHandoff();
    }

    function test_rotation_rejects_changed_successor_proof() public {
        require(hasRotation, "rotation fixture required");
        evidence.storageProof[0][10] = bytes1(uint8(evidence.storageProof[0][10]) ^ 1);
        _rejectAfterHandoff();
    }

    function _rejectAfterHandoff() private view {
        try verifier.verifyAfterHandoff(
            initialKey, boundary, boundaryHeight, trustedKey, evidence, height, account, bytes32(0), 42
        ) returns (
            bool valid
        ) {
            require(!valid, "invalid rotated evidence accepted");
        } catch {}
    }
}
