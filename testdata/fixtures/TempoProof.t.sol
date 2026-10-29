// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import {TempoProofVerifier} from "../src/TempoProofVerifier.sol";
import {LibBLS12381 as BLS} from "commonware/certificate/LibBLS12381.sol";
import {LibSimplex as Simplex} from "commonware/simplex/LibSimplex.sol";
import {Vm} from "../src/Vm.sol";

contract TempoProofTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    TempoProofVerifier private verifier;
    BLS.G2Point private initialKey;
    BLS.G2Point private nextKey;
    TempoProofVerifier.Evidence private evidence;
    TempoProofVerifier.Evidence private boundary;
    TempoProofVerifier.Evidence private successor;
    address private account;
    uint256 private height;
    uint256 private boundaryHeight;
    uint256 private successorHeight;
    BLS.G2Point private forkKey;
    BLS.G2Point private upstreamKey;
    TempoProofVerifier.Evidence private forkEvidence;
    TempoProofVerifier.Evidence private forkUpdated;
    TempoProofVerifier.Evidence private forkImported;
    address private forkAccount;
    address private forkRemote;
    uint256 private forkHeight;
    uint256 private forkUpdatedHeight;
    uint256 private forkImportedHeight;

    function setUp() public {
        string memory json = vm.readFile("fixtures/proofs.json");
        verifier = new TempoProofVerifier();
        initialKey = abi.decode(vm.parseJsonBytes(json, ".initialKey"), (BLS.G2Point));
        nextKey = abi.decode(vm.parseJsonBytes(json, ".nextKey"), (BLS.G2Point));
        account = vm.parseJsonAddress(json, ".account");
        height = vm.parseJsonUint(json, ".initial.height");
        boundaryHeight = vm.parseJsonUint(json, ".boundary.height");
        successorHeight = vm.parseJsonUint(json, ".successor.height");
        evidence = _readEvidence(json, ".initial.evidence");
        boundary = _readEvidence(json, ".boundary.evidence");
        successor = _readEvidence(json, ".successor.evidence");
        forkKey = abi.decode(vm.parseJsonBytes(json, ".fork.key"), (BLS.G2Point));
        upstreamKey = abi.decode(vm.parseJsonBytes(json, ".fork.upstreamKey"), (BLS.G2Point));
        forkAccount = vm.parseJsonAddress(json, ".fork.account");
        forkRemote = vm.parseJsonAddress(json, ".fork.remote");
        forkHeight = vm.parseJsonUint(json, ".fork.initial.height");
        forkUpdatedHeight = vm.parseJsonUint(json, ".fork.updated.height");
        forkImportedHeight = vm.parseJsonUint(json, ".fork.imported.height");
        forkEvidence = _readEvidence(json, ".fork.initial.evidence");
        forkUpdated = _readEvidence(json, ".fork.updated.evidence");
        forkImported = _readEvidence(json, ".fork.imported.evidence");
    }

    function _readEvidence(string memory json, string memory path)
        private
        pure
        returns (TempoProofVerifier.Evidence memory)
    {
        return abi.decode(
            vm.parseJsonType(
                json,
                path,
                "Evidence(bytes rawHeader,Subject subject,bytes vote,bytes seed,bytes[] accountProof,bytes[] storageProof)"
                "Subject(uint8 kind,uint64 epoch,uint64 viewNumber,uint64 parent,bytes32 payload)"
            ),
            (TempoProofVerifier.Evidence)
        );
    }

    function test_certified_storage_proof() public view {
        require(verifier.verify(initialKey, evidence, height, account, bytes32(0), 42), "accepted value");
    }

    function test_reject_wrong_value() public view {
        require(!verifier.verify(initialKey, evidence, height, account, bytes32(0), 43), "wrong value accepted");
    }

    function test_reject_changed_header() public {
        evidence.rawHeader[10] = bytes1(uint8(evidence.rawHeader[10]) ^ 1);
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
        require(this.verifyAfterHandoff(), "authenticated rotated storage");
    }

    function test_rotation_rejects_old_key() public view {
        try verifier.verify(initialKey, successor, successorHeight, account, bytes32(0), 42) returns (bool valid) {
            require(!valid, "old key accepted");
        } catch {}
    }

    function test_rotation_rejects_unannounced_key() public view {
        _rejectHandoff(initialKey, successor.subject.epoch);
    }

    function test_rotation_rejects_wrong_epoch() public view {
        _rejectHandoff(nextKey, successor.subject.epoch + 1);
    }

    function test_rotation_rejects_malformed_next_key() public view {
        BLS.G2Point memory changed = nextKey;
        changed.yC0Lo ^= bytes32(uint256(1));
        _rejectHandoff(changed, successor.subject.epoch);
    }

    function test_rotation_rejects_noncanonical_next_key() public view {
        BLS.G2Point memory changed = nextKey;
        changed.xC0Hi |= bytes32(uint256(1) << 128);
        _rejectHandoff(changed, successor.subject.epoch);
    }

    function _rejectHandoff(BLS.G2Point memory key, uint64 nextEpoch) private view {
        try verifier.verifyHandoff(initialKey, boundary, boundaryHeight, key, nextEpoch) returns (bool valid) {
            require(!valid, "invalid handoff accepted");
        } catch {}
    }

    function _reject() private view {
        try verifier.verify(initialKey, evidence, height, account, bytes32(0), 42) returns (bool valid) {
            require(!valid, "invalid evidence accepted");
        } catch {}
    }

    function test_reject_wrong_slot() public view {
        try verifier.verify(initialKey, evidence, height, account, bytes32(uint256(1)), 42) returns (bool valid) {
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

    function test_rotation_rejects_unauthenticated_boundary() public {
        initialKey = nextKey;
        _rejectAfterHandoff();
    }

    function test_rotation_rejects_changed_successor_epoch() public {
        ++successor.subject.epoch;
        _rejectAfterHandoff();
    }

    function test_rotation_rejects_changed_successor_proof() public {
        successor.storageProof[0][10] = bytes1(uint8(successor.storageProof[0][10]) ^ 1);
        _rejectAfterHandoff();
    }

    function _rejectAfterHandoff() private view {
        try this.verifyAfterHandoff() returns (bool valid) {
            require(!valid, "invalid rotated evidence accepted");
        } catch {}
    }

    function verifyAfterHandoff() external view returns (bool) {
        return verifier.verifyHandoff(initialKey, boundary, boundaryHeight, nextKey, successor.subject.epoch)
            && verifier.verify(nextKey, successor, successorHeight, account, bytes32(0), 42);
    }

    function test_fork_historical_certified_storage_proof() public view {
        require(
            verifier.verify(forkKey, forkEvidence, forkHeight, forkAccount, bytes32(0), 42), "historical fork value"
        );
    }

    function test_fork_updated_certified_storage_proof() public view {
        require(
            verifier.verify(forkKey, forkUpdated, forkUpdatedHeight, forkAccount, bytes32(0), 43), "updated fork value"
        );
    }

    function test_fork_imported_account_certified_storage_proof() public view {
        require(
            verifier.verify(forkKey, forkImported, forkImportedHeight, forkRemote, bytes32(0), 8), "imported fork value"
        );
    }

    function test_fork_reject_upstream_key() public {
        forkKey = upstreamKey;
        _rejectFork(42);
    }

    function test_fork_reject_mixed_account_proof() public {
        forkEvidence.accountProof = forkUpdated.accountProof;
        forkEvidence.storageProof = forkUpdated.storageProof;
        _rejectFork(43);
    }

    function test_fork_reject_mixed_storage_proof() public {
        forkEvidence.storageProof = forkUpdated.storageProof;
        _rejectFork(43);
    }

    function _rejectFork(uint256 expectedValue) private view {
        try verifier.verify(forkKey, forkEvidence, forkHeight, forkAccount, bytes32(0), expectedValue) returns (
            bool valid
        ) {
            require(!valid, "invalid fork evidence accepted");
        } catch {}
    }
}
