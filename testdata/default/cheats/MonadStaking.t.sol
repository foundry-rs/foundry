// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";
import "utils/MonadVm.sol";

/// @dev Minimal staking precompile interface for verification.
interface IMonadStaking {
    function getEpoch() external returns (uint64 epoch, bool inEpochDelayPeriod);
    function getProposerValId() external returns (uint64 val_id);
    function getValidator(uint64 validatorId)
        external
        returns (
            address authAddress,
            uint64 flags,
            uint256 stake,
            uint256 accRewardPerToken,
            uint256 commission,
            uint256 unclaimedRewards,
            uint256 consensusStake,
            uint256 consensusCommission,
            uint256 snapshotStake,
            uint256 snapshotCommission,
            bytes memory secpPubkey,
            bytes memory blsPubkey
        );
    function getDelegator(uint64 validatorId, address delegator)
        external
        returns (
            uint256 stake,
            uint256 accRewardPerToken,
            uint256 unclaimedRewards,
            uint256 deltaStake,
            uint256 nextDeltaStake,
            uint64 deltaEpoch,
            uint64 nextDeltaEpoch
        );
    function getWithdrawalRequest(uint64 validatorId, address delegator, uint8 withdrawId)
        external
        returns (uint256 withdrawalAmount, uint256 accRewardPerToken, uint64 withdrawEpoch);
    function getConsensusValidatorSet(uint32 startIndex)
        external
        returns (bool isDone, uint32 nextIndex, uint64[] memory valIds);
    function getSnapshotValidatorSet(uint32 startIndex)
        external
        returns (bool isDone, uint32 nextIndex, uint64[] memory valIds);
    function getExecutionValidatorSet(uint32 startIndex)
        external
        returns (bool isDone, uint32 nextIndex, uint64[] memory valIds);
    function addValidator(bytes calldata payload, bytes calldata signedSecp, bytes calldata signedBls)
        external
        payable
        returns (uint64 validatorId);
    function claimRewards(uint64 validatorId) external returns (uint256 rewards);
}

contract MonadStakingTest is Test {
    IMonadStaking constant STAKING = IMonadStaking(address(0x1000));
    MonadVm constant monad = MonadVm(0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA);

    /// @dev Build addValidator payload.
    /// Layout: secp_pubkey(33) + bls_pubkey(48) + auth_address(20) + stake(32) + commission(32) = 165 bytes.
    function _buildPayload(address auth, uint256 stake, uint256 commission) internal pure returns (bytes memory) {
        bytes memory secp = new bytes(33);
        bytes memory bls = new bytes(48);
        // Use auth address bytes in BLS pubkey to ensure uniqueness per validator.
        for (uint256 i = 0; i < 20; i++) {
            bls[i] = bytes20(auth)[i];
        }
        return abi.encodePacked(secp, bls, auth, stake, commission);
    }

    /// @dev Create a validator via the real addValidator precompile function.
    /// Deals balance and returns the assigned validator ID.
    function _createValidator(address auth, uint256 stake, uint256 commission) internal returns (uint64) {
        bytes memory payload = _buildPayload(auth, stake, commission);
        bytes memory dummySig64 = new bytes(64);
        bytes memory dummySig96 = new bytes(96);
        vm.deal(address(this), stake);
        return STAKING.addValidator{value: stake}(payload, dummySig64, dummySig96);
    }

    /// @dev Helper to call getValidator and decode only the first 6 fields.
    /// Avoids stack-too-deep from destructuring all 12 return values.
    /// Uses `call` instead of `staticcall` because the staking precompile rejects STATICCALL.
    function _getValidatorCore(uint64 valId)
        internal
        returns (
            address authAddress,
            uint64 flags,
            uint256 stake,
            uint256 accRewardPerToken,
            uint256 commission,
            uint256 unclaimedRewards
        )
    {
        (bool ok, bytes memory ret) = address(STAKING)
            .call(abi.encodeWithSelector(IMonadStaking.getValidator.selector, valId));
        require(ok, "getValidator call failed");
        // Decode only the first 6 fixed fields (skip dynamic bytes at the end)
        (authAddress, flags, stake, accRewardPerToken, commission, unclaimedRewards) =
            abi.decode(ret, (address, uint64, uint256, uint256, uint256, uint256));
    }

    /// @dev Helper to get consensus and snapshot view fields.
    /// Uses `call` instead of `staticcall` because the staking precompile rejects STATICCALL.
    function _getValidatorViews(uint64 valId)
        internal
        returns (uint256 consensusStake, uint256 consensusCommission, uint256 snapshotStake, uint256 snapshotCommission)
    {
        (bool ok, bytes memory ret) =
            address(STAKING).call(abi.encodeWithSelector(IMonadStaking.getValidator.selector, valId));
        require(ok, "getValidator call failed");
        // Skip first 6 fields (6 * 32 = 192 bytes), then read next 4
        assembly {
            consensusStake := mload(add(ret, 224)) // offset 192 + 32 (length prefix)
            consensusCommission := mload(add(ret, 256))
            snapshotStake := mload(add(ret, 288))
            snapshotCommission := mload(add(ret, 320))
        }
    }

    function _contains(bytes memory haystack, bytes memory needle) internal pure returns (bool) {
        if (needle.length == 0) return true;
        if (needle.length > haystack.length) return false;
        for (uint256 i = 0; i <= haystack.length - needle.length; i++) {
            bool ok = true;
            for (uint256 j = 0; j < needle.length; j++) {
                if (haystack[i + j] != needle[j]) {
                    ok = false;
                    break;
                }
            }
            if (ok) return true;
        }
        return false;
    }

    // =====================================================================
    // Direct State Control Tests (kept from previous version)
    // =====================================================================

    function testSetEpoch() public {
        monad.setEpoch(42, false);
        (uint64 epoch, bool inDelay) = STAKING.getEpoch();
        assertEq(epoch, 42, "epoch mismatch");
        assertTrue(!inDelay, "should not be in delay");

        monad.setEpoch(100, true);
        (epoch, inDelay) = STAKING.getEpoch();
        assertEq(epoch, 100, "epoch mismatch after update");
        assertTrue(inDelay, "should be in delay");
    }

    function testSetProposer() public {
        monad.setProposer(42);
        uint64 proposer = STAKING.getProposerValId();
        assertEq(proposer, 42, "proposer mismatch");
    }

    function testSetAccumulator() public {
        uint64 valId = _createValidator(address(this), 10_000_000 ether, 0);
        uint256 accValue = 123456789e18;
        monad.setAccumulator(valId, accValue);

        (,,, uint256 gotAcc,,) = _getValidatorCore(valId);
        assertEq(gotAcc, accValue, "accumulator mismatch");
    }

    // =====================================================================
    // Syscall Cheatcode Tests
    // =====================================================================

    /// @dev epochSnapshot rebuilds consensus set from execution set and populates views.
    function testEpochSnapshot() public {
        monad.setEpoch(1, false);

        // Create two validators with different stakes (>= 10M MON for ACTIVE_VALIDATOR_STAKE threshold)
        address auth1 = address(0xA1);
        address auth2 = address(0xA2);
        uint64 valId1 = _createValidator(auth1, 20_000_000 ether, 0.1e18);
        uint64 valId2 = _createValidator(auth2, 10_000_000 ether, 0.05e18);

        // Before snapshot: execution set has both, consensus/snapshot empty
        (bool isDone,, uint64[] memory execSet) = STAKING.getExecutionValidatorSet(0);
        assertTrue(isDone, "exec set should be complete");
        assertEq(execSet.length, 2, "should have 2 validators in exec set");

        // Run epoch snapshot
        monad.epochSnapshot();

        // After snapshot: consensus set should be rebuilt from execution set (sorted by stake desc)
        uint64[] memory consSet;
        (isDone,, consSet) = STAKING.getConsensusValidatorSet(0);
        assertTrue(isDone, "consensus set should be complete");
        assertEq(consSet.length, 2, "should have 2 validators in consensus set");
        // Sorted by stake descending: valId1 (200k) first, valId2 (100k) second
        assertEq(consSet[0], valId1, "highest stake validator first");
        assertEq(consSet[1], valId2, "lower stake validator second");

        // Consensus views should be populated with live stake/commission
        (uint256 consStake1, uint256 consComm1,,) = _getValidatorViews(valId1);
        assertEq(consStake1, 20_000_000 ether, "consensus stake for val1");
        assertEq(consComm1, 0.1e18, "consensus commission for val1");

        (uint256 consStake2, uint256 consComm2,,) = _getValidatorViews(valId2);
        assertEq(consStake2, 10_000_000 ether, "consensus stake for val2");
        assertEq(consComm2, 0.05e18, "consensus commission for val2");

        // in_boundary should now be true
        (, bool inDelay) = STAKING.getEpoch();
        assertTrue(inDelay, "should be in delay period after snapshot");
    }

    /// @dev epochChange increments epoch and clears in_boundary.
    function testEpochChange() public {
        monad.setEpoch(5, false);

        // Need to do snapshot first (epochChange requires in_boundary check indirectly)
        // Create a validator so snapshot has something to work with
        _createValidator(address(0xB1), 10_000_000 ether, 0);
        monad.epochSnapshot();

        // Verify in_boundary is true after snapshot
        (, bool inDelay) = STAKING.getEpoch();
        assertTrue(inDelay, "should be in delay after snapshot");

        // Run epoch change
        monad.epochChange(6);

        // Verify epoch incremented and in_boundary cleared
        (uint64 epoch, bool inDelayAfter) = STAKING.getEpoch();
        assertEq(epoch, 6, "epoch should be 6");
        assertTrue(!inDelayAfter, "in_boundary should be cleared");
    }

    /// @dev epochBoundary is a convenience: snapshot + change.
    function testEpochBoundary() public {
        monad.setEpoch(10, false);

        address auth = address(0xC1);
        uint64 valId = _createValidator(auth, 15_000_000 ether, 0.1e18);

        monad.epochBoundary(11);

        // Epoch should be 11 and not in delay
        (uint64 epoch, bool inDelay) = STAKING.getEpoch();
        assertEq(epoch, 11, "epoch should be 11");
        assertTrue(!inDelay, "should not be in delay after full boundary");

        // Consensus set should have been rebuilt
        (bool isDone,, uint64[] memory consSet) = STAKING.getConsensusValidatorSet(0);
        assertTrue(isDone);
        assertEq(consSet.length, 1, "should have 1 validator");
        assertEq(consSet[0], valId);

        // Consensus views should be populated
        (uint256 consStake, uint256 consComm,,) = _getValidatorViews(valId);
        assertEq(consStake, 15_000_000 ether, "consensus stake should match");
        assertEq(consComm, 0.1e18, "consensus commission should match");
    }

    /// @dev blockReward distributes reward via production-equivalent syscallReward handler.
    function testBlockReward() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        uint64 valId = _createValidator(auth, 10_000_000 ether, 0.1e18);

        // Must run epoch lifecycle to populate consensus views before blockReward
        monad.epochBoundary(2);

        // Distribute 10 MON block reward
        monad.blockReward(auth, 10 ether);

        (,,, uint256 accReward,, uint256 unclaimed) = _getValidatorCore(valId);

        // commission = 10% => del_reward = 9 MON goes to unclaimed.
        assertEq(unclaimed, 9 ether, "unclaimed should track delegator reward only");
        // Accumulator should reflect delegator share
        assertTrue(accReward > 0, "accumulator should be positive");
    }

    /// @dev blockReward with zero commission — all delegator reward goes to accumulator.
    function testBlockRewardZeroCommission() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        uint64 valId = _createValidator(auth, 10_000_000 ether, 0);

        monad.epochBoundary(2);
        monad.blockReward(auth, 10 ether);

        (,,, uint256 accReward,, uint256 unclaimed) = _getValidatorCore(valId);
        assertEq(unclaimed, 10 ether, "unclaimed should be full reward");
        assertTrue(accReward > 0, "full reward should go to accumulator");
    }

    /// @dev Multiple blockReward calls accumulate correctly.
    function testBlockRewardMultiple() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        _createValidator(auth, 10_000_000 ether, 0.1e18);

        monad.epochBoundary(2);
        monad.blockReward(auth, 10 ether);
        monad.blockReward(auth, 20 ether);

        (,,,,, uint256 unclaimed) = _getValidatorCore(1);
        // del_reward = 9 + 18 = 27 MON with 10% commission.
        assertEq(unclaimed, 27 ether, "accumulated unclaimed mismatch");
    }

    /// @dev blockReward with unknown author reverts.
    function testBlockRewardUnknownAuthor() public {
        monad.setEpoch(1, false);

        _createValidator(address(this), 10_000_000 ether, 0);
        monad.epochBoundary(2);

        (bool ok, bytes memory ret) =
            address(monad).call(abi.encodeWithSelector(MonadVm.blockReward.selector, address(0xDEAD), 10 ether));
        assertTrue(!ok, "unknown author should revert");
        assertTrue(_contains(ret, bytes("blockReward failed: not in validator set")), "revert reason mismatch");
    }

    /// @dev Full E2E: create validator, epoch lifecycle, block reward, verify accumulator math.
    function testRewardCalculationE2E() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        uint64 valId = _createValidator(auth, 10_000_000 ether, 0.1e18);

        // Run full epoch lifecycle
        monad.epochBoundary(2);

        // Distribute 10 MON block reward
        monad.blockReward(auth, 10 ether);

        (,,, uint256 valAcc,, uint256 unclaimed) = _getValidatorCore(valId);
        assertEq(unclaimed, 9 ether, "delegator reward goes to unclaimed");
        assertTrue(valAcc > 0, "accumulator should be positive");

        // Verify accumulator math:
        // commission = 10 * 0.1 = 1 MON
        // del_reward = 10 - 1 = 9 MON
        // acc_delta = 9e18 * 1e36 / 10_000_000e18
        // pending_rewards = stake * acc_delta / 1e36 = 10_000_000e18 * acc_delta / 1e36 = 9e18
        uint256 pendingRewards = valAcc * 10_000_000 ether / 1e36;
        assertEq(pendingRewards, 9 ether, "delegator should earn 9 MON (all delegator reward)");
    }

    /// @dev Snapshot builds views from live execution set, used by blockReward.
    function testSnapshotThenReward() public {
        monad.setEpoch(1, false);

        // Create two validators
        address auth1 = address(0xD1);
        address auth2 = address(0xD2);
        uint64 valId1 = _createValidator(auth1, 20_000_000 ether, 0.1e18);
        uint64 valId2 = _createValidator(auth2, 10_000_000 ether, 0.05e18);

        // Run epoch lifecycle
        monad.epochBoundary(2);

        // Reward both validators
        monad.blockReward(auth1, 20 ether);
        monad.blockReward(auth2, 10 ether);

        // Verify both got rewards
        (,,,,, uint256 unclaimed1) = _getValidatorCore(valId1);
        (,,,,, uint256 unclaimed2) = _getValidatorCore(valId2);
        assertEq(unclaimed1, 18 ether, "val1 unclaimed");
        assertEq(unclaimed2, 9.5 ether, "val2 unclaimed");
    }
}
