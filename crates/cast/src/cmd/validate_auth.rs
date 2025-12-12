use std::collections::HashMap;

use alloy_consensus::Transaction;
use alloy_eips::{BlockId, eip7702::SignedAuthorization};
use alloy_network::{AnyNetwork, TransactionResponse};
use alloy_primitives::{Address, B256};
use alloy_provider::Provider;
use eyre::OptionExt;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig, init_progress},
};
use foundry_common::shell;

#[derive(Debug, clap::Parser)]
pub struct ValidateAuthArgs {
    /// Transaction hash.
    tx_hash: B256,

    #[command(flatten)]
    rpc: RpcOpts,
}

/// Validates an authorization and updates the running nonce map if valid.
/// Returns (valid_chain, valid_nonce, expected_nonce, authority) if signature recovery succeeds.
async fn validate_and_update_nonce<P: Provider<AnyNetwork>>(
    auth: &SignedAuthorization,
    chain_id: u64,
    block_number: u64,
    running_nonces: &mut HashMap<Address, u64>,
    provider: &P,
) -> eyre::Result<Option<(bool, bool, u64, Address)>> {
    let valid_chain = auth.chain_id == chain_id || auth.chain_id == 0;

    match auth.recover_authority() {
        Ok(authority) => {
            // Get expected nonce for this authority
            let expected_nonce = if let Some(&nonce) = running_nonces.get(&authority) {
                nonce
            } else {
                // Fetch nonce at block - 1 (state before this block)
                let prev_block = BlockId::number(block_number - 1);
                provider.get_transaction_count(authority).block_id(prev_block).await?
            };

            let valid_nonce = auth.nonce == expected_nonce;

            // If authorization was valid, update running nonce
            if valid_chain && valid_nonce {
                running_nonces.insert(authority, expected_nonce + 1);
            }

            Ok(Some((valid_chain, valid_nonce, expected_nonce, authority)))
        }
        Err(_) => Ok(None),
    }
}

impl ValidateAuthArgs {
    /// Validates all the authorizations in an EIP-7702 transaction. It does so by validating the
    /// nonce and chain id for each of the recovered authority from the given authorizations.
    ///
    /// For nonce validation, it builds a "running nonce" map by processing all transactions
    /// before the target transaction in the same block:
    /// - For each transaction sender, tracks their next expected nonce (tx.nonce + 1)
    /// - For each valid authorization in previous transactions, also increments that authority's
    ///   running nonce (since valid authorizations execute and increment the authority's nonce)
    /// - If an authority is not in the running nonce map, fetches their nonce at block - 1
    ///
    /// Then, for each authorization in the target transaction:
    /// - Validates nonce against the running nonce
    /// - If the authorization is valid (both chain and nonce), increments the running nonce for
    ///   that authority (for subsequent authorizations in the same transaction)
    ///
    /// For chain id validation, it checks if it is zero or the same chainid as the current chain.
    pub async fn run(self) -> eyre::Result<()> {
        let config = self.rpc.load_config()?;
        let provider = utils::get_provider(&config)?;

        let tx = provider
            .get_transaction_by_hash(self.tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", self.tx_hash))?;

        // Get block info for nonce calculation
        let block_number = tx.block_number.ok_or_eyre("transaction is not yet mined")?;
        let tx_index = tx.transaction_index.ok_or_eyre("transaction index not available")?;

        // Fetch the block to get all transactions up to this one
        let block = provider
            .get_block_by_number(block_number.into())
            .full()
            .await?
            .ok_or_else(|| eyre::eyre!("block not found: {}", block_number))?;

        let chain_id = provider.get_chain_id().await?;

        // Build a map of address -> running nonce from txs in this block up to (but not
        // including) our tx. We need to process both sender nonces AND any valid authorizations
        // from previous transactions, since those affect the running nonce.
        let mut running_nonces: HashMap<Address, u64> = HashMap::new();

        // Check if there are any previous transactions
        if tx_index > 0 {
            if !shell::is_json() {
                sh_println!("Executing previous transactions from the block.")?;
            }

            let pb = init_progress(tx_index as u64, "tx");
            pb.set_position(0);

            // Process all transactions BEFORE our target transaction
            for (index, block_tx) in block.transactions.txns().take(tx_index as usize).enumerate() {
                let from = block_tx.from();
                let nonce = block_tx.nonce();
                // Track the next expected nonce (current nonce + 1)
                running_nonces.insert(from, nonce + 1);

                // Also process any valid authorizations in this previous transaction
                if let Some(auth_list) = block_tx.authorization_list() {
                    for auth in auth_list {
                        validate_and_update_nonce(
                            auth,
                            chain_id,
                            block_number,
                            &mut running_nonces,
                            &provider,
                        )
                        .await?;
                    }
                }

                pb.set_position((index + 1) as u64);
            }

            if !shell::is_json() {
                sh_println!()?;
            }
        }

        // Also track our target transaction's sender nonce
        running_nonces.insert(tx.from(), tx.nonce() + 1);

        // Extract authorization list from EIP-7702 transaction
        let auth_list =
            tx.authorization_list().ok_or_eyre("Transaction has no authorization list")?;

        sh_println!("Transaction: {}", self.tx_hash)?;
        sh_println!("Block: {} (tx index: {})", block_number, tx_index)?;
        sh_println!()?;

        if auth_list.is_empty() {
            sh_println!("Authorization list is empty")?;
        } else {
            for (i, auth) in auth_list.iter().enumerate() {
                sh_println!("Authorization #{}", i)?;
                sh_println!("  Decoded:")?;
                sh_println!("    Chain ID: {}", auth.chain_id,)?;
                sh_println!("    Address: {}", auth.address)?;
                sh_println!("    Nonce: {}", auth.nonce)?;
                sh_println!("    r: {}", auth.r())?;
                sh_println!("    s: {}", auth.s())?;
                sh_println!("    v: {}", auth.y_parity())?;

                match validate_and_update_nonce(
                    auth,
                    chain_id,
                    block_number,
                    &mut running_nonces,
                    &provider,
                )
                .await?
                {
                    Some((valid_chain, valid_nonce, expected_nonce, authority)) => {
                        sh_println!("  Recovered Authority: {}", authority)?;

                        sh_println!("  Validation Status:")?;
                        sh_println!(
                            "    Chain: {}",
                            if valid_chain {
                                "VALID".to_string()
                            } else {
                                format!("INVALID (expected: 0 or {chain_id})")
                            }
                        )?;

                        if valid_nonce {
                            sh_println!("    Nonce: VALID")?;
                        } else {
                            sh_println!(
                                "    Nonce: INVALID (expected: {}, got: {})",
                                expected_nonce,
                                auth.nonce
                            )?;
                        }

                        // Check if the authority's code was set to the delegated address
                        // Fetch code at the transaction's block to see state after tx execution
                        let code = provider
                            .get_code_at(authority)
                            .block_id(BlockId::number(block_number))
                            .await?;
                        sh_println!("  Code Status (at end of block {}):", block_number)?;
                        if code.is_empty() {
                            sh_println!("    No delegation (account has no code)")?;
                        } else if code.len() == 23 && code[0..3] == [0xef, 0x01, 0x00] {
                            // EIP-7702 delegation designator: 0xef0100 followed by 20-byte
                            // address
                            let delegated_to = Address::from_slice(&code[3..23]);
                            if delegated_to == auth.address {
                                sh_println!("    ACTIVE (delegated to {})", delegated_to)?;
                            } else {
                                sh_println!("    SUPERSEDED (delegated to {})", delegated_to)?;
                            }
                        } else {
                            sh_println!("    Account has contract code (not a delegation)")?;
                        }
                    }
                    None => {
                        sh_println!("  Authority: UNKNOWN")?;
                        sh_println!("  Signature: INVALID (recovery failed)")?;
                    }
                }
                sh_println!()?;
            }
        }
        Ok(())
    }
}
