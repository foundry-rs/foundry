use crate::eth::{
    error::{BlockchainError, Result},
    macros::node_info,
    EthApi,
};

use ethers::types::{
    Action, Address, Block, BlockId, BlockNumber, Bytes, Call, Create, CreateResult, Res, Reward,
    Transaction, TxHash, H256, U256, U64,
};
use itertools::Itertools;

use super::types::{
    OtsBlockDetails, OtsBlockTransactions, OtsContractCreator, OtsInternalOperation,
    OtsSearchTransactions, OtsTrace,
};

impl EthApi {
    /// Otterscan currently requires this endpoint, even though it's not part of the ots_*
    /// https://github.com/otterscan/otterscan/blob/071d8c55202badf01804f6f8d53ef9311d4a9e47/src/useProvider.ts#L71
    ///
    /// As a faster alternative to eth_getBlockByNumber (by excluding uncle block
    /// information), which is not relevant in the context of an anvil node
    pub async fn erigon_get_header_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<Block<TxHash>>> {
        node_info!("ots_getApiLevel");

        self.backend.block_by_number(number).await
    }

    /// As per the latest Otterscan source code, at least version 8 is needed
    /// https://github.com/otterscan/otterscan/blob/071d8c55202badf01804f6f8d53ef9311d4a9e47/src/params.ts#L1C2-L1C2
    pub async fn ots_get_api_level(&self) -> Result<u64> {
        node_info!("ots_getApiLevel");

        // as required by current otterscan's source code
        Ok(8)
    }

    /// Trace internal ETH transfers, contracts creation (CREATE/CREATE2) and self-destructs for a
    /// certain transaction.
    pub async fn ots_get_internal_operations(
        &self,
        hash: H256,
    ) -> Result<Vec<OtsInternalOperation>> {
        node_info!("ots_getInternalOperations");

        self.backend
            .mined_parity_trace_transaction(hash)
            .map(OtsInternalOperation::batch_build)
            .ok_or_else(|| BlockchainError::DataUnavailable)
    }

    /// Check if an ETH address contains code at a certain block number.
    pub async fn ots_has_code(&self, address: Address, block_number: BlockNumber) -> Result<bool> {
        node_info!("ots_hasCode");
        let block_id = Some(BlockId::Number(block_number));
        Ok(self.get_code(address, block_id).await?.len() > 0)
    }

    /// Trace a transaction and generate a trace call tree.
    pub async fn ots_trace_transaction(&self, hash: H256) -> Result<Vec<OtsTrace>> {
        node_info!("ots_traceTransaction");

        Ok(OtsTrace::batch_build(self.backend.trace_transaction(hash).await?))
    }

    /// Given a transaction hash, returns its raw revert reason.
    pub async fn ots_get_transaction_error(&self, hash: H256) -> Result<Option<Bytes>> {
        node_info!("ots_getTransactionError");

        if let Some(receipt) = self.backend.mined_transaction_receipt(hash) {
            if receipt.inner.status == Some(U64::zero()) {
                return Ok(receipt.out)
            }
        }

        Ok(Default::default())
    }

    /// For simplicity purposes, we return the entire block instead of emptying the values that
    /// Otterscan doesn't want. This is the original purpose of the endpoint (to save bandwidth),
    /// but it doesn't seem necessary in the context of an anvil node
    pub async fn ots_get_block_details(&self, number: BlockNumber) -> Result<OtsBlockDetails> {
        node_info!("ots_getBlockDetails");

        if let Some(block) = self.backend.block_by_number(number).await? {
            let ots_block = OtsBlockDetails::build(block, &self.backend).await?;

            Ok(ots_block)
        } else {
            Err(BlockchainError::BlockNotFound)
        }
    }

    /// For simplicity purposes, we return the entire block instead of emptying the values that
    /// Otterscan doesn't want. This is the original purpose of the endpoint (to save bandwidth),
    /// but it doesn't seem necessary in the context of an anvil node
    pub async fn ots_get_block_details_by_hash(&self, hash: H256) -> Result<OtsBlockDetails> {
        node_info!("ots_getBlockDetailsByHash");

        if let Some(block) = self.backend.block_by_hash(hash).await? {
            let ots_block = OtsBlockDetails::build(block, &self.backend).await?;

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
    ) -> Result<OtsBlockTransactions> {
        node_info!("ots_getBlockTransactions");

        match self.backend.block_by_number_full(number.into()).await? {
            Some(block) => OtsBlockTransactions::build(block, &self.backend, page, page_size).await,
            None => Err(BlockchainError::BlockNotFound),
        }
    }

    /// Address history navigation. searches backwards from certain point in time.
    pub async fn ots_search_transactions_before(
        &self,
        address: Address,
        block_number: u64,
        page_size: usize,
    ) -> Result<OtsSearchTransactions> {
        node_info!("ots_searchTransactionsBefore");

        let best = self.backend.best_number().as_u64();
        // we go from given block (defaulting to best) down to first block
        // considering only post-fork
        let from = if block_number == 0 { best } else { block_number };
        let to = self.get_fork().map(|f| f.block_number() + 1).unwrap_or(1);

        let first_page = from == best;
        let mut last_page = false;

        let mut res: Vec<_> = vec![];

        dbg!(to, from);
        for n in (to..=from).rev() {
            if n == to {
                last_page = true;
            }

            if let Some(traces) = self.backend.mined_parity_trace_block(n) {
                let hashes = traces
                    .into_iter()
                    .rev()
                    .filter_map(|trace| match trace.action {
                        Action::Call(Call { from, to, .. }) if from == address || to == address => {
                            trace.transaction_hash
                        }
                        _ => None,
                    })
                    .unique();

                res.extend(hashes);

                if res.len() >= page_size {
                    break
                }
            }
        }

        OtsSearchTransactions::build(res, &self.backend, first_page, last_page).await
    }

    /// Address history navigation. searches forward from certain point in time.
    pub async fn ots_search_transactions_after(
        &self,
        address: Address,
        block_number: u64,
        page_size: usize,
    ) -> Result<OtsSearchTransactions> {
        node_info!("ots_searchTransactionsAfter");

        let best = self.backend.best_number().as_u64();
        // we go from the first post-fork block, up to the tip
        let from = if block_number == 0 {
            self.get_fork().map(|f| f.block_number() + 1).unwrap_or(1)
        } else {
            block_number
        };
        let to = best;

        let first_page = from == best;
        let mut last_page = false;

        let mut res: Vec<_> = vec![];

        for n in from..=to {
            if n == to {
                last_page = true;
            }

            if let Some(traces) = self.backend.mined_parity_trace_block(n) {
                let hashes = traces
                    .into_iter()
                    .rev()
                    .filter_map(|trace| match trace.action {
                        Action::Call(Call { from, to, .. }) if from == address || to == address => {
                            trace.transaction_hash
                        }
                        Action::Create(Create { from, .. }) if from == address => {
                            trace.transaction_hash
                        }
                        Action::Reward(Reward { author, .. }) if author == address => {
                            trace.transaction_hash
                        }
                        _ => None,
                    })
                    .unique();

                res.extend(hashes);

                if res.len() >= page_size {
                    break
                }
            }
        }

        OtsSearchTransactions::build(res, &self.backend, first_page, last_page).await
    }

    /// Given a sender address and a nonce, returns the tx hash or null if not found. It returns
    /// only the tx hash on success, you can use the standard eth_getTransactionByHash after that to
    /// get the full transaction data.
    pub async fn ots_get_transaction_by_sender_and_nonce(
        &self,
        address: Address,
        nonce: U256,
    ) -> Result<Option<Transaction>> {
        node_info!("ots_getTransactionBySenderAndNonce");

        let from = self.get_fork().map(|f| f.block_number() + 1).unwrap_or_default();
        let to = self.backend.best_number().as_u64();

        for n in (from..=to).rev() {
            if let Some(txs) = self.backend.mined_transactions_by_block_number(n.into()).await {
                for tx in txs {
                    if tx.nonce == nonce && tx.from == address {
                        return Ok(Some(tx))
                    }
                }
            }
        }

        Ok(None)
    }

    /// Given an ETH contract address, returns the tx hash and the direct address who created the
    /// contract.
    pub async fn ots_get_contract_creator(
        &self,
        addr: Address,
    ) -> Result<Option<OtsContractCreator>> {
        node_info!("ots_getContractCreator");

        let from = self.get_fork().map(|f| f.block_number()).unwrap_or_default();
        let to = self.backend.best_number().as_u64();

        // loop in reverse, since we want the latest deploy to the address
        for n in (from..=to).rev() {
            if let Some(traces) = dbg!(self.backend.mined_parity_trace_block(n)) {
                for trace in traces.into_iter().rev() {
                    match (trace.action, trace.result) {
                        (
                            Action::Create(Create { from, .. }),
                            Some(Res::Create(CreateResult { address, .. })),
                        ) if address == addr => {
                            return Ok(Some(OtsContractCreator {
                                hash: trace.transaction_hash.unwrap(),
                                creator: from,
                            }))
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(None)
    }
}
