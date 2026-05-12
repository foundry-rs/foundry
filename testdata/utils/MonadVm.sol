// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.6.2 <0.9.0;
pragma experimental ABIEncoderV2;

/// @title Monad Cheatcodes Interface
/// @notice Cheatcodes for staking lifecycle control in Monad Foundry tests.
/// @dev These cheatcodes live at a separate address from the standard Foundry
///      cheatcode address: 0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA.
interface MonadVm {
    /// Sets the current epoch and delay period for the staking precompile.
    function setEpoch(uint64 epoch, bool inDelayPeriod) external;

    /// Sets the current block proposer validator ID.
    function setProposer(uint64 valId) external;

    /// Directly sets a validator's accumulated reward per token.
    function setAccumulator(uint64 valId, uint256 value) external;

    /// Distribute block reward via the real syscallReward handler.
    function blockReward(address author, uint256 reward) external;

    /// Execute syscallSnapshot.
    function epochSnapshot() external;

    /// Execute syscallOnEpochChange.
    function epochChange(uint64 newEpoch) external;

    /// Convenience: epochSnapshot() then epochChange(newEpoch).
    function epochBoundary(uint64 newEpoch) external;
}
