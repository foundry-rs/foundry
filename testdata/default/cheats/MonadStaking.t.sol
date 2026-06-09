// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";
import "utils/MonadVm.sol";

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
    function getConsensusValidatorSet(uint32 startIndex)
        external
        returns (bool isDone, uint32 nextIndex, uint64[] memory valIds);
    function getExecutionValidatorSet(uint32 startIndex)
        external
        returns (bool isDone, uint32 nextIndex, uint64[] memory valIds);
    function addValidator(bytes calldata payload, bytes calldata signedSecp, bytes calldata signedBls)
        external
        payable
        returns (uint64 validatorId);
}

contract MonadStakingTest is Test {
    IMonadStaking constant STAKING = IMonadStaking(address(0x1000));
    MonadVm constant monad = MonadVm(0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA);

    function setUp() public {
        (bool ok, bytes memory ret) = address(STAKING).call(abi.encodeWithSelector(IMonadStaking.getEpoch.selector));
        if (!ok || ret.length < 64) {
            vm.skip(true, "Monad staking precompile is only available with --network monad");
        }
    }

    function _buildPayload(address auth, uint256 stake, uint256 commission) internal pure returns (bytes memory) {
        bytes memory secp = new bytes(33);
        bytes memory bls = new bytes(48);
        for (uint256 i = 0; i < 20; i++) {
            bls[i] = bytes20(auth)[i];
        }
        return abi.encodePacked(secp, bls, auth, stake, commission);
    }

    function _createValidator(address auth, uint256 stake, uint256 commission) internal returns (uint64) {
        bytes memory dummySig64 = new bytes(64);
        bytes memory dummySig96 = new bytes(96);
        vm.deal(address(this), stake);
        return STAKING.addValidator{value: stake}(_buildPayload(auth, stake, commission), dummySig64, dummySig96);
    }

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
        (authAddress, flags, stake, accRewardPerToken, commission, unclaimedRewards) =
            abi.decode(ret, (address, uint64, uint256, uint256, uint256, uint256));
    }

    function _getValidatorViews(uint64 valId)
        internal
        returns (uint256 consensusStake, uint256 consensusCommission, uint256 snapshotStake, uint256 snapshotCommission)
    {
        (bool ok, bytes memory ret) =
            address(STAKING).call(abi.encodeWithSelector(IMonadStaking.getValidator.selector, valId));
        require(ok, "getValidator call failed");
        assembly {
            consensusStake := mload(add(ret, 224))
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

    function testEpochSnapshot() public {
        monad.setEpoch(1, false);

        address auth1 = address(0xA1);
        address auth2 = address(0xA2);
        uint64 valId1 = _createValidator(auth1, 20_000_000 ether, 0.1e18);
        uint64 valId2 = _createValidator(auth2, 10_000_000 ether, 0.05e18);

        (bool isDone,, uint64[] memory execSet) = STAKING.getExecutionValidatorSet(0);
        assertTrue(isDone, "exec set should be complete");
        assertEq(execSet.length, 2, "should have 2 validators in exec set");

        monad.epochSnapshot();

        uint64[] memory consSet;
        (isDone,, consSet) = STAKING.getConsensusValidatorSet(0);
        assertTrue(isDone, "consensus set should be complete");
        assertEq(consSet.length, 2, "should have 2 validators in consensus set");
        assertEq(consSet[0], valId1, "highest stake validator first");
        assertEq(consSet[1], valId2, "lower stake validator second");

        (uint256 consStake1, uint256 consComm1,,) = _getValidatorViews(valId1);
        assertEq(consStake1, 20_000_000 ether, "consensus stake for val1");
        assertEq(consComm1, 0.1e18, "consensus commission for val1");

        (uint256 consStake2, uint256 consComm2,,) = _getValidatorViews(valId2);
        assertEq(consStake2, 10_000_000 ether, "consensus stake for val2");
        assertEq(consComm2, 0.05e18, "consensus commission for val2");

        (, bool inDelay) = STAKING.getEpoch();
        assertTrue(inDelay, "should be in delay period after snapshot");
    }

    function testEpochChange() public {
        monad.setEpoch(5, false);
        _createValidator(address(0xB1), 10_000_000 ether, 0);
        monad.epochSnapshot();

        (, bool inDelay) = STAKING.getEpoch();
        assertTrue(inDelay, "should be in delay after snapshot");

        monad.epochChange(6);

        (uint64 epoch, bool inDelayAfter) = STAKING.getEpoch();
        assertEq(epoch, 6, "epoch should be 6");
        assertTrue(!inDelayAfter, "in_boundary should be cleared");
    }

    function testEpochBoundary() public {
        monad.setEpoch(10, false);

        address auth = address(0xC1);
        uint64 valId = _createValidator(auth, 15_000_000 ether, 0.1e18);

        monad.epochBoundary(11);

        (uint64 epoch, bool inDelay) = STAKING.getEpoch();
        assertEq(epoch, 11, "epoch should be 11");
        assertTrue(!inDelay, "should not be in delay after full boundary");

        (bool isDone,, uint64[] memory consSet) = STAKING.getConsensusValidatorSet(0);
        assertTrue(isDone);
        assertEq(consSet.length, 1, "should have 1 validator");
        assertEq(consSet[0], valId);

        (uint256 consStake, uint256 consComm,,) = _getValidatorViews(valId);
        assertEq(consStake, 15_000_000 ether, "consensus stake should match");
        assertEq(consComm, 0.1e18, "consensus commission should match");
    }

    function testBlockReward() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        uint64 valId = _createValidator(auth, 10_000_000 ether, 0.1e18);

        monad.epochBoundary(2);
        monad.blockReward(auth, 10 ether);

        (,,, uint256 accReward,, uint256 unclaimed) = _getValidatorCore(valId);
        assertEq(unclaimed, 9 ether, "unclaimed should track delegator reward only");
        assertTrue(accReward > 0, "accumulator should be positive");
    }

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

    function testBlockRewardMultiple() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        _createValidator(auth, 10_000_000 ether, 0.1e18);

        monad.epochBoundary(2);
        monad.blockReward(auth, 10 ether);
        monad.blockReward(auth, 20 ether);

        (,,,,, uint256 unclaimed) = _getValidatorCore(1);
        assertEq(unclaimed, 27 ether, "accumulated unclaimed mismatch");
    }

    function testBlockRewardUnknownAuthor() public {
        monad.setEpoch(1, false);

        _createValidator(address(this), 10_000_000 ether, 0);
        monad.epochBoundary(2);

        (bool ok, bytes memory ret) =
            address(monad).call(abi.encodeWithSelector(MonadVm.blockReward.selector, address(0xDEAD), 10 ether));
        assertTrue(!ok, "unknown author should revert");
        assertTrue(_contains(ret, bytes("blockReward failed: not in validator set")), "revert reason mismatch");
    }

    function testRewardCalculationE2E() public {
        monad.setEpoch(1, false);

        address auth = address(this);
        uint64 valId = _createValidator(auth, 10_000_000 ether, 0.1e18);

        monad.epochBoundary(2);
        monad.blockReward(auth, 10 ether);

        (,,, uint256 valAcc,, uint256 unclaimed) = _getValidatorCore(valId);
        assertEq(unclaimed, 9 ether, "delegator reward goes to unclaimed");
        assertTrue(valAcc > 0, "accumulator should be positive");

        uint256 pendingRewards = valAcc * 10_000_000 ether / 1e36;
        assertEq(pendingRewards, 9 ether, "delegator should earn 9 MON");
    }

    function testSnapshotThenReward() public {
        monad.setEpoch(1, false);

        address auth1 = address(0xD1);
        address auth2 = address(0xD2);
        uint64 valId1 = _createValidator(auth1, 20_000_000 ether, 0.1e18);
        uint64 valId2 = _createValidator(auth2, 10_000_000 ether, 0.05e18);

        monad.epochBoundary(2);
        monad.blockReward(auth1, 20 ether);
        monad.blockReward(auth2, 10 ether);

        (,,,,, uint256 unclaimed1) = _getValidatorCore(valId1);
        (,,,,, uint256 unclaimed2) = _getValidatorCore(valId2);
        assertEq(unclaimed1, 18 ether, "val1 unclaimed");
        assertEq(unclaimed2, 9.5 ether, "val2 unclaimed");
    }
}
