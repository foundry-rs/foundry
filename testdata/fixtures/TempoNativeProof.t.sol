// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import {TempoProofVerifier} from "../src/TempoProofVerifier.sol";
import {LibBLS12381 as BLS} from "commonware/certificate/LibBLS12381.sol";
import {Vm} from "../src/Vm.sol";

/// @dev The native chain supplies the anchor; the chosen Anvil key authenticates a local partial trie.
contract TempoNativeProofTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    TempoProofVerifier private verifier;
    BLS.G2Point private trustedKey;
    TempoProofVerifier.Evidence private evidence;
    uint256 private height;
    address private account;

    function setUp() public {
        string memory json = vm.readFile("fixtures/native-proof.json");
        verifier = new TempoProofVerifier();
        trustedKey = abi.decode(vm.parseJsonBytes(json, ".trustedKey"), (BLS.G2Point));
        height = vm.parseJsonUint(json, ".local.height");
        account = vm.parseJsonAddress(json, ".account");
        require(height > vm.parseJsonUint(json, ".anchor.height"), "local block after anchor");
        evidence = abi.decode(
            vm.parseJsonType(
                json,
                ".local.evidence",
                "Evidence(bytes rawHeader,Subject subject,bytes vote,bytes seed,bytes[] accountProof,bytes[] storageProof)"
                "Subject(uint8 kind,uint64 epoch,uint64 viewNumber,uint64 parent,bytes32 payload)"
            ),
            (TempoProofVerifier.Evidence)
        );
        require(evidence.subject.payload != vm.parseJsonBytes32(json, ".anchor.hash"), "local block digest");
    }

    function test_native_fork_certified_storage_proof() public view {
        require(verifier.verify(trustedKey, evidence, height, account, bytes32(0), 42), "accepted value");
    }

    function test_native_fork_rejects_wrong_value() public view {
        require(!verifier.verify(trustedKey, evidence, height, account, bytes32(0), 43), "wrong value accepted");
    }
}
