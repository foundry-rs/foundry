//! Broadcast state and transactions shared by execution engines.

use alloy_network::{Ethereum, Network};
use alloy_primitives::Address;
use foundry_common::TransactionMaybeSigned;
use std::collections::VecDeque;

/// Active script broadcast context.
#[derive(Clone, Debug, Default)]
pub struct Broadcast {
    /// Address of the transaction origin.
    pub new_origin: Address,
    /// Original caller.
    pub original_caller: Address,
    /// Original `tx.origin`.
    pub original_origin: Address,
    /// Depth of the broadcast.
    pub depth: usize,
    /// Whether the broadcast stops after the next call.
    pub single_call: bool,
    /// Whether `vm.deployCode` is deploying from code.
    pub deploy_from_code: bool,
}

/// Transaction collected from a script for later simulation and broadcast.
#[derive(Clone, Debug)]
pub struct BroadcastableTransaction<N: Network = Ethereum> {
    /// Optional RPC URL.
    pub rpc: Option<String>,
    /// Transaction to broadcast.
    pub transaction: TransactionMaybeSigned<N>,
}

/// Ordered transactions collected from a script.
pub type BroadcastableTransactions<N> = VecDeque<BroadcastableTransaction<N>>;
