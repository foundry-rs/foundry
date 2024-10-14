use crate::eth::{
    error::{BlockchainError, Result},
    macros::node_info,
    EthApi,
};
use alloy_network::BlockResponse;
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rpc_types::{
    trace::{
        otterscan::{
            BlockDetails, ContractCreator, InternalOperation, OtsBlock, OtsBlockTransactions,
            OtsReceipt, OtsSlimBlock, OtsTransactionReceipt, TraceEntry, TransactionsWithReceipts,
        },
        parity::{
            Action, CallAction, CallType, CreateAction, CreateOutput, LocalizedTransactionTrace,
            RewardAction, TraceOutput,
        },
    },
    AnyNetworkBlock, Block, BlockId, BlockNumberOrTag as BlockNumber, BlockTransactions,
    Transaction,
};
use alloy_serde::WithOtherFields;
use itertools::Itertools;

use futures::future::join_all;

pub fn mentions_address(trace: LocalizedTransactionTrace, address: Address) -> Option<B256> {
    match (trace.trace.action, trace.trace.result) {
        (Action::Call(CallAction { from, to, .. }), _) if from == address || to == address => {
            trace.transaction_hash
        }
        (_, Some(TraceOutput::Create(CreateOutput { address: created_address, .. })))
            if created_address == address =>
        {
            trace.transaction_hash
        }
        (Action::Create(CreateAction { from, .. }), _) if from == address => trace.transaction_hash,
        (Action::Reward(RewardAction { author, .. }), _) if author == address => {
            trace.transaction_hash
        }
        _ => None,
    }
}

/// Converts the list of traces for a transaction into the expected Otterscan format.
///
/// Follows format specified in the [`ots_traceTransaction`](https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_tracetransaction) spec.
pub fn batch_build_ots_traces(traces: Vec<LocalizedTransactionTrace>) -> Vec<TraceEntry> {
    traces
        .into_iter()
        .filter_map(|trace| {
            let output = trace
                .trace
                .result
                .map(|r| match r {
                    TraceOutput::Call(output) => output.output,
                    TraceOutput::Create(output) => output.code,
                })
                .unwrap_or_default();
            match trace.trace.action {
                Action::Call(call) => Some(TraceEntry {
                    r#type: match call.call_type {
                        CallType::Call => "CALL",
                        CallType::CallCode => "CALLCODE",
                        CallType::DelegateCall => "DELEGATECALL",
                        CallType::StaticCall => "STATICCALL",
                        CallType::AuthCall => "AUTHCALL",
                        CallType::None => "NONE",
                    }
                    .to_string(),
                    depth: trace.trace.trace_address.len() as u32,
                    from: call.from,
                    to: call.to,
                    value: call.value,
                    input: call.input,
                    output,
                }),
                Action::Create(_) | Action::Selfdestruct(_) | Action::Reward(_) => None,
            }
        })
        .collect()
}

impl EthApi {
    /// Otterscan currently requires this endpoint, even though it's not part of the `ots_*`.
    /// Ref: <https://github.com/otterscan/otterscan/blob/071d8c55202badf01804f6f8d53ef9311d4a9e47/src/useProvider.ts#L71>
    ///
    /// As a faster alternative to `eth_getBlockByNumber` (by excluding uncle block
    /// information), which is not relevant in the context of an anvil node
    pub async fn erigon_get_header_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AnyNetworkBlock>> {
        node_info!("ots_getApiLevel");

        self.backend.block_by_number(number).await
    }

    /// As per the latest Otterscan source code, at least version 8 is needed.
    /// Ref: <https://github.com/otterscan/otterscan/blob/071d8c55202badf01804f6f8d53ef9311d4a9e47/src/params.ts#L1C2-L1C2>
    pub async fn ots_get_api_level(&self) -> Result<u64> {
        node_info!("ots_getApiLevel");

        // as required by current otterscan's source code
        Ok(8)
    }

    /// Trace internal ETH transfers, contracts creation (CREATE/CREATE2) and self-destructs for a
    /// certain transaction.
    pub async fn ots_get_internal_operations(&self, hash: B256) -> Result<Vec<InternalOperation>> {
        node_info!("ots_getInternalOperations");

        self.backend
            .mined_transaction(hash)
            .map(|tx| tx.ots_internal_operations())
            .ok_or_else(|| BlockchainError::DataUnavailable)
    }

    /// Check if an ETH address contains code at a certain block number.
    pub async fn ots_has_code(&self, address: Address, block_number: BlockNumber) -> Result<bool> {
        node_info!("ots_hasCode");
        let block_id = Some(BlockId::Number(block_number));
        Ok(self.get_code(address, block_id).await?.len() > 0)
    }

    /// Trace a transaction and generate a trace call tree.
    pub async fn ots_trace_transaction(&self, hash: B256) -> Result<Vec<TraceEntry>> {
        node_info!("ots_traceTransaction");

        Ok(batch_build_ots_traces(self.backend.trace_transaction(hash).await?))
    }

    /// Given a transaction hash, returns its raw revert reason.
    pub async fn ots_get_transaction_error(&self, hash: B256) -> Result<Bytes> {
        node_info!("ots_getTransactionError");

        if let Some(receipt) = self.backend.mined_transaction_receipt(hash) {
            if !receipt.inner.inner.as_receipt_with_bloom().receipt.status.coerce_status() {
                return Ok(receipt.out.map(|b| b.0.into()).unwrap_or(Bytes::default()));
            }
        }

        Ok(Bytes::default())
    }

    /// For simplicity purposes, we return the entire block instead of emptying the values that
    /// Otterscan doesn't want. This is the original purpose of the endpoint (to save bandwidth),
    /// but it doesn't seem necessary in the context of an anvil node
    pub async fn ots_get_block_details(&self, number: BlockNumber) -> Result<BlockDetails> {
        node_info!("ots_getBlockDetails");

        if let Some(block) = self.backend.block_by_number(number).await? {
            let ots_block = self.build_ots_block_details(block).await?;
            Ok(ots_block)
        } else {
            Err(BlockchainError::BlockNotFound)
        }
    }

    /// For simplicity purposes, we return the entire block instead of emptying the values that
    /// Otterscan doesn't want. This is the original purpose of the endpoint (to save bandwidth),
    /// but it doesn't seem necessary in the context of an anvil node
    pub async fn ots_get_block_details_by_hash(&self, hash: B256) -> Result<BlockDetails> {
        node_info!("ots_getBlockDetailsByHash");

        if let Some(block) = self.backend.block_by_hash(hash).await? {
            let ots_block = self.build_ots_block_details(block).await?;
            Ok(ots_block)
        } else {
            Err(BlockchainError::BlockNotFound)
        }
    }

    /// Gets paginated transaction data for a certain block. Return data is similar to
    /// eth_getBlockBy* + eth_getTransactionReceipt.
    pub async fn ots_get_block_transactions(
        &self,
        number: u64,
        page: usize,
        page_size: usize,
    ) -> Result<OtsBlockTransactions<WithOtherFields<Transaction>>> {
        node_info!("ots_getBlockTransactions");

        match self.backend.block_by_number_full(number.into()).await? {
            Some(block) => self.build_ots_block_tx(block, page, page_size).await,
            None => Err(BlockchainError::BlockNotFound),
        }
    }

    /// Address history navigation. searches backwards from certain point in time.
    pub async fn ots_search_transactions_before(
        &self,
        address: Address,
        block_number: u64,
        page_size: usize,
    ) -> Result<TransactionsWithReceipts> {
        node_info!("ots_searchTransactionsBefore");

        let best = self.backend.best_number();
        // we go from given block (defaulting to best) down to first block
        // considering only post-fork
        let from = if block_number == 0 { best } else { block_number - 1 };
        let to = self.get_fork().map(|f| f.block_number() + 1).unwrap_or(1);

        let first_page = from >= best;
        let mut last_page = false;

        let mut res: Vec<_> = vec![];

        for n in (to..=from).rev() {
            if let Some(traces) = self.backend.mined_parity_trace_block(n) {
                let hashes = traces
                    .into_iter()
                    .rev()
                    .filter_map(|trace| mentions_address(trace, address))
                    .unique();

                if res.len() >= page_size {
                    break;
                }

                res.extend(hashes);
            }

            if n == to {
                last_page = true;
            }
        }

        self.build_ots_search_transactions(res, first_page, last_page).await
    }

    /// Address history navigation. searches forward from certain point in time.
    pub async fn ots_search_transactions_after(
        &self,
        address: Address,
        block_number: u64,
        page_size: usize,
    ) -> Result<TransactionsWithReceipts> {
        node_info!("ots_searchTransactionsAfter");

        let best = self.backend.best_number();
        // we go from the first post-fork block, up to the tip
        let first_block = self.get_fork().map(|f| f.block_number() + 1).unwrap_or(1);
        let from = if block_number == 0 { first_block } else { block_number + 1 };
        let to = best;

        let mut first_page = from >= best;
        let mut last_page = false;

        let mut res: Vec<_> = vec![];

        for n in from..=to {
            if n == first_block {
                last_page = true;
            }

            if let Some(traces) = self.backend.mined_parity_trace_block(n) {
                let hashes = traces
                    .into_iter()
                    .rev()
                    .filter_map(|trace| mentions_address(trace, address))
                    .unique();

                if res.len() >= page_size {
                    break;
                }

                res.extend(hashes);
            }

            if n == to {
                first_page = true;
            }
        }

        // Results are always sent in reverse chronological order, according to the Otterscan spec
        res.reverse();
        self.build_ots_search_transactions(res, first_page, last_page).await
    }

    /// Given a sender address and a nonce, returns the tx hash or null if not found. It returns
    /// only the tx hash on success, you can use the standard eth_getTransactionByHash after that to
    /// get the full transaction data.
    pub async fn ots_get_transaction_by_sender_and_nonce(
        &self,
        address: Address,
        nonce: U256,
    ) -> Result<Option<B256>> {
        node_info!("ots_getTransactionBySenderAndNonce");

        let from = self.get_fork().map(|f| f.block_number() + 1).unwrap_or_default();
        let to = self.backend.best_number();

        for n in (from..=to).rev() {
            if let Some(txs) = self.backend.mined_transactions_by_block_number(n.into()).await {
                for tx in txs {
                    if U256::from(tx.nonce) == nonce && tx.from == address {
                        return Ok(Some(tx.hash));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Given an ETH contract address, returns the tx hash and the direct address who created the
    /// contract.
    pub async fn ots_get_contract_creator(&self, addr: Address) -> Result<Option<ContractCreator>> {
        node_info!("ots_getContractCreator");

        let from = self.get_fork().map(|f| f.block_number()).unwrap_or_default();
        let to = self.backend.best_number();

        // loop in reverse, since we want the latest deploy to the address
        for n in (from..=to).rev() {
            if let Some(traces) = self.backend.mined_parity_trace_block(n) {
                for trace in traces.into_iter().rev() {
                    match (trace.trace.action, trace.trace.result) {
                        (
                            Action::Create(CreateAction { from, .. }),
                            Some(TraceOutput::Create(CreateOutput { address, .. })),
                        ) if address == addr => {
                            return Ok(Some(ContractCreator {
                                hash: trace.transaction_hash.unwrap(),
                                creator: from,
                            }));
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(None)
    }
    /// The response for ots_getBlockDetails includes an `issuance` object that requires computing
    /// the total gas spent in a given block.
    ///
    /// The only way to do this with the existing API is to explicitly fetch all receipts, to get
    /// their `gas_used`. This would be extremely inefficient in a real blockchain RPC, but we can
    /// get away with that in this context.
    ///
    /// The [original spec](https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_getblockdetails)
    /// also mentions we can hardcode `transactions` and `logsBloom` to an empty array to save
    /// bandwidth, because fields weren't intended to be used in the Otterscan UI at this point.
    ///
    /// This has two problems though:
    ///   - It makes the endpoint too specific to Otterscan's implementation
    ///   - It breaks the abstraction built in `OtsBlock<TX>` which computes `transaction_count`
    ///     based on the existing list.
    ///
    /// Therefore we keep it simple by keeping the data in the response
    pub async fn build_ots_block_details(&self, block: AnyNetworkBlock) -> Result<BlockDetails> {
        if block.transactions.is_uncle() {
            return Err(BlockchainError::DataUnavailable);
        }
        let receipts_futs = block
            .transactions
            .hashes()
            .map(|hash| async move { self.transaction_receipt(hash).await });

        // fetch all receipts
        let receipts = join_all(receipts_futs)
            .await
            .into_iter()
            .map(|r| match r {
                Ok(Some(r)) => Ok(r),
                _ => Err(BlockchainError::DataUnavailable),
            })
            .collect::<Result<Vec<_>>>()?;

        let total_fees = receipts
            .iter()
            .fold(0, |acc, receipt| acc + receipt.gas_used * receipt.effective_gas_price);

        let Block { header, uncles, transactions, size, withdrawals } = block.inner;

        let block = OtsSlimBlock {
            header,
            uncles,
            transaction_count: transactions.len(),
            size,
            withdrawals,
        };

        Ok(BlockDetails {
            block,
            total_fees: U256::from(total_fees),
            // issuance has no meaningful value in anvil's backend. just default to 0
            issuance: Default::default(),
        })
    }

    /// Fetches all receipts for the blocks's transactions, as required by the
    /// [`ots_getBlockTransactions`] endpoint spec, and returns the final response object.
    ///
    /// [`ots_getBlockTransactions`]: https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_getblockdetails
    pub async fn build_ots_block_tx(
        &self,
        mut block: AnyNetworkBlock,
        page: usize,
        page_size: usize,
    ) -> Result<OtsBlockTransactions<WithOtherFields<Transaction>>> {
        if block.transactions.is_uncle() {
            return Err(BlockchainError::DataUnavailable);
        }

        block.transactions = match block.transactions() {
            BlockTransactions::Full(txs) => BlockTransactions::Full(
                txs.iter().skip(page * page_size).take(page_size).cloned().collect(),
            ),
            BlockTransactions::Hashes(txs) => BlockTransactions::Hashes(
                txs.iter().skip(page * page_size).take(page_size).cloned().collect(),
            ),
            BlockTransactions::Uncle => unreachable!(),
        };

        let receipt_futs = block.transactions.hashes().map(|hash| self.transaction_receipt(hash));

        let receipts = join_all(receipt_futs.map(|r| async {
            if let Ok(Some(r)) = r.await {
                let block = self.block_by_number(r.block_number.unwrap().into()).await?;
                let timestamp = block.ok_or(BlockchainError::BlockNotFound)?.header.timestamp;
                let receipt = r.map_inner(OtsReceipt::from);
                Ok(OtsTransactionReceipt { receipt, timestamp: Some(timestamp) })
            } else {
                Err(BlockchainError::BlockNotFound)
            }
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

        let transaction_count = block.transactions().len();
        let fullblock = OtsBlock { block: block.inner, transaction_count };

        let ots_block_txs =
            OtsBlockTransactions::<WithOtherFields<Transaction>> { fullblock, receipts };

        Ok(ots_block_txs)
    }

    pub async fn build_ots_search_transactions(
        &self,
        hashes: Vec<B256>,
        first_page: bool,
        last_page: bool,
    ) -> Result<TransactionsWithReceipts> {
        let txs_futs = hashes.iter().map(|hash| async { self.transaction_by_hash(*hash).await });

        let txs = join_all(txs_futs)
            .await
            .into_iter()
            .map(|t| match t {
                Ok(Some(t)) => Ok(t.inner),
                _ => Err(BlockchainError::DataUnavailable),
            })
            .collect::<Result<Vec<_>>>()?;

        let receipt_futs = hashes.iter().map(|hash| self.transaction_receipt(*hash));

        let receipts = join_all(receipt_futs.map(|r| async {
            if let Ok(Some(r)) = r.await {
                let block = self.block_by_number(r.block_number.unwrap().into()).await?;
                let timestamp = block.ok_or(BlockchainError::BlockNotFound)?.header.timestamp;
                let receipt = r.map_inner(OtsReceipt::from);
                Ok(OtsTransactionReceipt { receipt, timestamp: Some(timestamp) })
            } else {
                Err(BlockchainError::BlockNotFound)
            }
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

        Ok(TransactionsWithReceipts { txs, receipts, first_page, last_page })
    }
}
