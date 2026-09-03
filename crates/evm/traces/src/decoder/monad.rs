use alloy_sol_types::sol;

sol!(
    #[sol(abi)]
    "src/decoder/monad/IMonadStaking.sol"
);

sol!(
    #[sol(abi)]
    "src/decoder/monad/IReserveBalance.sol"
);

alloy_sol_types::sol! {
    /// Monad staking syscalls are intentionally not part of the public monad-std interface, but
    /// the trace decoder still knows them so syscall traces get named.
    #[sol(abi)]
    interface IMonadStakingSyscalls {
        function syscallOnEpochChange(uint64 epoch) external;
        function syscallReward(address blockAuthor) external;
        function syscallSnapshot() external;
    }
}
