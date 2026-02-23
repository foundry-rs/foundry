// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.6.2 <0.9.0;
pragma experimental ABIEncoderV2;

/// @title Monad Cheatcodes Interface
/// @notice Cheatcodes for staking lifecycle control in Monad Foundry tests.
/// @dev These cheatcodes live at a separate address from the standard Foundry
///      `CHEATCODE_ADDRESS`: 0xc0FFeeCD43A10e1C2b0De63c6CDCFe5B7d0e0CEA
///
///      State-mutating staking functions (delegate, undelegate, addValidator, etc.)
///      are handled by the staking precompile directly. These cheatcodes provide:
///      1. Direct state control: setEpoch, setProposer, setAccumulator
///      2. Syscall wrappers: blockReward, epochSnapshot, epochChange, epochBoundary
interface MonadVm {
    /// Sets the current epoch and delay period for the staking precompile.
    function setEpoch(uint64 epoch, bool inDelayPeriod) external;

    /// Sets the current block proposer validator ID.
    function setProposer(uint64 valId) external;

    /// Directly sets a validator's accumulated reward per token.
    function setAccumulator(uint64 valId, uint256 value) external;

    /// Distribute block reward via the real syscallReward handler.
    /// Mints `reward` to staking address and distributes via accumulator math
    /// using consensus/snapshot view stake (production-equivalent behavior).
    function blockReward(address author, uint256 reward) external;

    /// Execute syscallSnapshot: copies consensus→snapshot view, rebuilds
    /// consensus set from execution set sorted by stake. Sets in_boundary = true.
    function epochSnapshot() external;

    /// Execute syscallOnEpochChange: increments epoch, clears in_boundary.
    /// `newEpoch` must equal `currentEpoch + 1`.
    function epochChange(uint64 newEpoch) external;

    /// Convenience: epochSnapshot() then epochChange(newEpoch).
    function epochBoundary(uint64 newEpoch) external;
}
