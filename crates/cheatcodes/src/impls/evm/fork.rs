use super::{Cheatcode, CheatsCtxt, DatabaseExt, Result};
use crate::{impls::CreateFork, Cheatcodes, Vm::*};
use alloy_primitives::{B256, U256};
use alloy_sol_types::SolValue;
use ethers_core::types::Filter;
use ethers_providers::Middleware;
use foundry_common::ProviderBuilder;
use foundry_compilers::utils::RuntimeOrHandle;
use foundry_utils::types::{ToAlloy, ToEthers};

impl Cheatcode for activeForkCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        ccx.data
            .db
            .active_fork_id()
            .map(|id| id.abi_encode())
            .ok_or_else(|| fmt_err!("no active fork"))
    }
}

impl Cheatcode for createFork_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias } = self;
        create_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createFork_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createFork_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for createSelectFork_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias } = self;
        create_select_fork(ccx, urlOrAlias, None)
    }
}

impl Cheatcode for createSelectFork_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias, blockNumber } = self;
        create_select_fork(ccx, urlOrAlias, Some(blockNumber.saturating_to()))
    }
}

impl Cheatcode for createSelectFork_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { urlOrAlias, txHash } = self;
        create_select_fork_at_transaction(ccx, urlOrAlias, txHash)
    }
}

impl Cheatcode for rollFork_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { blockNumber } = self;
        ccx.data.db.roll_fork(None, *blockNumber, ccx.data.env, &mut ccx.data.journaled_state)?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { txHash } = self;
        ccx.data.db.roll_fork_to_transaction(
            None,
            *txHash,
            ccx.data.env,
            &mut ccx.data.journaled_state,
        )?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { forkId, blockNumber } = self;
        ccx.data.db.roll_fork(
            Some(*forkId),
            *blockNumber,
            ccx.data.env,
            &mut ccx.data.journaled_state,
        )?;
        Ok(Default::default())
    }
}

impl Cheatcode for rollFork_3Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { forkId, txHash } = self;
        ccx.data.db.roll_fork_to_transaction(
            Some(*forkId),
            *txHash,
            ccx.data.env,
            &mut ccx.data.journaled_state,
        )?;
        Ok(Default::default())
    }
}

impl Cheatcode for selectForkCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { forkId } = self;
        check_broadcast(ccx.state)?;

        // No need to correct since the sender's nonce does not get incremented when selecting a
        // fork.
        ccx.state.corrected_nonce = true;

        ccx.data.db.select_fork(*forkId, ccx.data.env, &mut ccx.data.journaled_state)?;
        Ok(Default::default())
    }
}

impl Cheatcode for transact_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { txHash } = *self;
        ccx.data.db.transact(
            None,
            txHash,
            ccx.data.env,
            &mut ccx.data.journaled_state,
            ccx.state,
        )?;
        Ok(Default::default())
    }
}

impl Cheatcode for transact_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { forkId, txHash } = *self;
        ccx.data.db.transact(
            Some(forkId),
            txHash,
            ccx.data.env,
            &mut ccx.data.journaled_state,
            ccx.state,
        )?;
        Ok(Default::default())
    }
}

impl Cheatcode for allowCheatcodesCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account } = self;
        ccx.data.db.allow_cheatcode_access(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account } = self;
        ccx.data.db.add_persistent_account(*account);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account0, account1 } = self;
        ccx.data.db.add_persistent_account(*account0);
        ccx.data.db.add_persistent_account(*account1);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_2Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account0, account1, account2 } = self;
        ccx.data.db.add_persistent_account(*account0);
        ccx.data.db.add_persistent_account(*account1);
        ccx.data.db.add_persistent_account(*account2);
        Ok(Default::default())
    }
}

impl Cheatcode for makePersistent_3Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { accounts } = self;
        ccx.data.db.extend_persistent_accounts(accounts.iter().copied());
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account } = self;
        ccx.data.db.remove_persistent_account(account);
        Ok(Default::default())
    }
}

impl Cheatcode for revokePersistent_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { accounts } = self;
        ccx.data.db.remove_persistent_accounts(accounts.iter().copied());
        Ok(Default::default())
    }
}

impl Cheatcode for isPersistentCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account } = self;
        Ok(ccx.data.db.is_persistent(account).abi_encode())
    }
}

impl Cheatcode for rpcCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { method, params } = self;
        let url =
            ccx.data.db.active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let provider = ProviderBuilder::new(url).build()?;

        let params_json: serde_json::Value = serde_json::from_str(params)?;
        let result = RuntimeOrHandle::new()
            .block_on(provider.request(method, params_json))
            .map_err(|err| fmt_err!("{method:?}: {err}"))?;

        let result_as_tokens = crate::impls::json::value_to_token(&result)
            .map_err(|err| fmt_err!("failed to parse result: {err}"))?;

        Ok(result_as_tokens.abi_encode())
    }
}

impl Cheatcode for eth_getLogsCall {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { fromBlock, toBlock, addr, topics } = self;
        let (Ok(from_block), Ok(to_block)) = (u64::try_from(fromBlock), u64::try_from(toBlock))
        else {
            bail!("blocks in block range must be less than 2^64 - 1")
        };

        if topics.len() > 4 {
            bail!("topics array must contain at most 4 elements")
        }

        let url =
            ccx.data.db.active_fork_url().ok_or_else(|| fmt_err!("no active fork URL found"))?;
        let provider = ProviderBuilder::new(url).build()?;
        let mut filter =
            Filter::new().address(addr.to_ethers()).from_block(from_block).to_block(to_block);
        for (i, topic) in topics.iter().enumerate() {
            let topic = topic.to_ethers();
            match i {
                0 => filter = filter.topic0(topic),
                1 => filter = filter.topic1(topic),
                2 => filter = filter.topic2(topic),
                3 => filter = filter.topic3(topic),
                _ => unreachable!(),
            };
        }

        let logs = RuntimeOrHandle::new()
            .block_on(provider.get_logs(&filter))
            .map_err(|e| fmt_err!("eth_getLogs: {e}"))?;

        let eth_logs = logs
            .into_iter()
            .map(|log| EthGetLogs {
                emitter: log.address.to_alloy(),
                topics: log.topics.into_iter().map(ToAlloy::to_alloy).collect(),
                data: log.data.0.into(),
                blockNumber: U256::from(log.block_number.unwrap_or_default().to_alloy()),
                transactionHash: log.transaction_hash.unwrap_or_default().to_alloy(),
                transactionIndex: U256::from(log.transaction_index.unwrap_or_default().to_alloy()),
                blockHash: log.block_hash.unwrap_or_default().to_alloy(),
                logIndex: log.log_index.unwrap_or_default().to_alloy(),
                removed: log.removed.unwrap_or(false),
            })
            .collect::<Vec<_>>();

        Ok(eth_logs.abi_encode())
    }
}

/// Creates and then also selects the new fork
fn create_select_fork<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result {
    check_broadcast(ccx.state)?;

    // No need to correct since the sender's nonce does not get incremented when selecting a fork.
    ccx.state.corrected_nonce = true;

    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let id = ccx.data.db.create_select_fork(fork, ccx.data.env, &mut ccx.data.journaled_state)?;
    Ok(id.abi_encode())
}

/// Creates a new fork
fn create_fork<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, block)?;
    let id = ccx.data.db.create_fork(fork)?;
    Ok(id.abi_encode())
}

/// Creates and then also selects the new fork at the given transaction
fn create_select_fork_at_transaction<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    check_broadcast(ccx.state)?;

    // No need to correct since the sender's nonce does not get incremented when selecting a fork.
    ccx.state.corrected_nonce = true;

    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let id = ccx.data.db.create_select_fork_at_transaction(
        fork,
        ccx.data.env,
        &mut ccx.data.journaled_state,
        *transaction,
    )?;
    Ok(id.abi_encode())
}

/// Creates a new fork at the given transaction
fn create_fork_at_transaction<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    url_or_alias: &str,
    transaction: &B256,
) -> Result {
    let fork = create_fork_request(ccx, url_or_alias, None)?;
    let id = ccx.data.db.create_fork_at_transaction(fork, *transaction)?;
    Ok(id.abi_encode())
}

/// Creates the request object for a new fork request
fn create_fork_request<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    url_or_alias: &str,
    block: Option<u64>,
) -> Result<CreateFork> {
    let url = ccx.state.config.rpc_url(url_or_alias)?;
    let mut evm_opts = ccx.state.config.evm_opts.clone();
    evm_opts.fork_block_number = block;
    let fork = CreateFork {
        enable_caching: ccx.state.config.rpc_storage_caching.enable_for_endpoint(&url),
        url,
        env: ccx.data.env.clone(),
        evm_opts,
    };
    Ok(fork)
}

#[inline]
fn check_broadcast(state: &Cheatcodes) -> Result<()> {
    if state.broadcast.is_none() {
        Ok(())
    } else {
        Err(fmt_err!("cannot select forks during a broadcast"))
    }
}
