//! Deposits to Tempo L1 portals and authenticated zone withdrawals.

use crate::tempo::tempo_provider;
use alloy_network::{EthereumWallet, Network, primitives::ReceiptResponse};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::{PendingTransactionBuilder, Provider, ProviderBuilder};
use alloy_rpc_types::BlockId;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, WrapErr, ensure};
use foundry_cli::{json::print_scalar, opts::RpcOpts, utils::LoadConfig};
use foundry_common::{sh_status, shell};
use foundry_wallets::{WalletOpts, WalletSigner};
use std::time::Duration;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{ITIP20, PATH_USD_ADDRESS};

mod abi;
mod auth;
mod earn;
mod encryption;
mod l1;

// Mirrored from zones/crates/primitives and zones/crates/precompiles at a1c15e9f.
const MAX_CALLBACK_GAS_LIMIT: u64 = 10_000_000;
const MAX_CALLBACK_DATA_SIZE: usize = 1024;

/// Tempo zone operations.
#[derive(Debug, Parser)]
pub struct ZoneArgs {
    #[command(subcommand)]
    command: ZoneSubcommand,
}

#[derive(Debug, Parser)]
enum ZoneSubcommand {
    /// Encrypt a deposit and submit it to a portal on Tempo L1.
    ///
    /// --rpc-url selects the L1 RPC. The receipt confirms L1 submission, not zone completion.
    Deposit(DepositArgs),
    /// Request a withdrawal using the authenticated zone RPC.
    ///
    /// --rpc-url selects the zone RPC. The receipt confirms the request, not L1 settlement.
    Withdraw(WithdrawArgs),
    /// Build encrypted callbacks for the scoped Earn router.
    Earn(earn::EarnArgs),
}

#[derive(Debug, Parser)]
struct TransferArgs {
    /// TIP-20 token address on the source chain.
    #[arg(long, default_value_t = PATH_USD_ADDRESS)]
    token: Address,
    /// Amount in the token's smallest units.
    #[arg(long)]
    amount: u128,
    /// Destination address. Defaults to the signing wallet.
    #[arg(long)]
    to: Option<Address>,
    /// Transfer memo.
    #[arg(long, default_value_t = B256::ZERO)]
    memo: B256,
    /// Approve the required amount (including withdrawal fees) if allowance is insufficient.
    #[arg(long)]
    approve: bool,
    #[command(flatten)]
    rpc: RpcOpts,
    #[command(flatten)]
    wallet: WalletOpts,
}

#[derive(Debug, Parser)]
struct DepositArgs {
    /// Zone portal address on Tempo L1.
    #[arg(long, env = "L1_PORTAL_ADDRESS")]
    portal: Address,
    /// Tempo L1 refund recipient if the deposit fails. Defaults to the signing wallet.
    #[arg(long)]
    refund_recipient: Option<Address>,
    #[command(flatten)]
    transfer: TransferArgs,
}

#[derive(Debug, Parser)]
struct WithdrawArgs {
    /// Zone ID used to scope the RPC authorization token.
    #[arg(long, env = "ZONE_ID")]
    zone_id: u32,
    /// Zone chain ID used to sign the RPC authorization token before connecting.
    #[arg(long, env = "ZONE_CHAIN_ID")]
    zone_chain_id: u64,
    /// L1 callback gas limit. Zero disables the callback.
    #[arg(long, default_value_t = 0)]
    callback_gas_limit: u64,
    /// Zone recipient for bounced withdrawals if L1 execution fails. Defaults to the signing
    /// wallet.
    #[arg(long)]
    fallback_recipient: Option<Address>,
    /// Data passed to the L1 receiver's withdrawal callback, without a function selector.
    #[arg(long, default_value = "0x")]
    callback_data: Bytes,
    /// Compressed secp256k1 key for revealing the withdrawal sender.
    #[arg(long, default_value = "0x")]
    reveal_to: Bytes,
    /// Wait for successful L1 delivery, including callback execution. Does not wait for a return
    /// deposit.
    #[arg(long)]
    wait_l1: bool,
    /// Maximum seconds to wait for L1 settlement after zone inclusion.
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(1..))]
    wait_timeout: u64,
    #[command(flatten)]
    l1: l1::L1Args,
    #[command(flatten)]
    transfer: TransferArgs,
}

impl ZoneArgs {
    pub async fn run(self) -> Result<()> {
        match self.command {
            ZoneSubcommand::Deposit(args) => args.run().await,
            ZoneSubcommand::Withdraw(args) => args.run().await,
            ZoneSubcommand::Earn(args) => args.run().await,
        }
    }
}

impl TransferArgs {
    async fn signer(&self) -> Result<WalletSigner> {
        ensure!(!self.rpc.curl, "zone operations do not support --curl");
        ensure!(self.amount > 0, "amount must be greater than zero");
        let signer = self.wallet.signer().await?;
        if let Some(from) = self.wallet.from {
            ensure!(from == signer.address(), "--from does not match the signing wallet");
        }
        Ok(signer)
    }

    async fn ensure_allowance(
        &self,
        provider: &impl Provider<TempoNetwork>,
        sender: Address,
        spender: Address,
        amount: u128,
        gas_price: Option<u128>,
    ) -> Result<()> {
        let token = ITIP20::new(self.token, provider);
        let amount = U256::from(amount);
        if token.allowance(sender, spender).from(sender).call().await? < amount {
            ensure!(
                self.approve,
                "insufficient token allowance for {spender}; use --approve to approve the deposit or withdrawal amount"
            );
            sh_status!("Approving {} for {}", amount, spender)?;
            let mut approval = token.approve(spender, amount).from(sender);
            if let Some(gas_price) = gas_price {
                approval = approval.max_fee_per_gas(gas_price).max_priority_fee_per_gas(0);
            }
            let receipt = submitted_receipt("token approval", approval.send().await?).await?;
            ensure!(receipt.status(), "token approval reverted: {}", receipt.transaction_hash());
        }
        Ok(())
    }
}

impl DepositArgs {
    async fn run(self) -> Result<()> {
        let signer = self.transfer.signer().await?;
        let sender = signer.address();
        let (_, provider) = tempo_provider(&self.transfer.rpc)?;
        let provider = ProviderBuilder::new_with_network::<TempoNetwork>()
            .wallet(EthereumWallet::from(signer))
            .connect_provider(provider);
        let portal = abi::IZonePortal::new(self.portal, &provider);
        let block = provider.get_block_number().await?;
        let key = portal
            .encryptionKeyAtBlock(block)
            .block(BlockId::number(block))
            .call()
            .await
            .wrap_err("failed to fetch portal encryption key")?;
        let payload = encryption::encrypt_deposit(
            key.x,
            key.yParity,
            self.transfer.to.unwrap_or(sender),
            self.transfer.memo,
            sender,
            self.portal,
            key.keyIndex,
        )?;
        self.transfer
            .ensure_allowance(&provider, sender, self.portal, self.transfer.amount, None)
            .await?;
        let pending = portal
            .deposit(
                self.transfer.token,
                self.transfer.amount,
                key.keyIndex,
                payload,
                self.refund_recipient.unwrap_or(sender),
            )
            .from(sender)
            .send()
            .await?;
        let receipt = submitted_receipt("deposit", pending).await?;
        ensure!(receipt.status(), "deposit reverted: {}", receipt.transaction_hash());
        let _ = sh_status!("Deposit submitted on L1; zone processing is asynchronous.");
        print_receipt(&receipt)
    }
}

impl WithdrawArgs {
    async fn run(self) -> Result<()> {
        validate_callback(&self.callback_data, self.callback_gas_limit)?;
        ensure!(self.zone_id != 0, "--zone-id must be nonzero");
        ensure!(self.zone_chain_id != 0, "--zone-chain-id must be nonzero");
        if !self.reveal_to.is_empty() {
            ensure!(
                self.reveal_to.len() == 33
                    && k256::PublicKey::from_sec1_bytes(&self.reveal_to).is_ok(),
                "--reveal-to must be a compressed secp256k1 public key"
            );
        }
        let signer = self.transfer.signer().await?;
        let sender = signer.address();
        let token = auth::sign_token(&signer, self.zone_id, self.zone_chain_id)
            .await
            .wrap_err("wallet could not sign zone RPC authorization")?;
        let mut config = self.transfer.rpc.load_config()?;
        let headers = config.eth_rpc_headers.get_or_insert_default();
        headers.retain(|header| {
            !header
                .split_once(':')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(auth::HEADER))
        });
        headers.push(format!("{}: {token}", auth::HEADER));
        let provider =
            foundry_common::provider::ProviderBuilder::<TempoNetwork>::from_config(&config)?
                .build()?;
        let provider = ProviderBuilder::new_with_network::<TempoNetwork>()
            .wallet(EthereumWallet::from(signer))
            .connect_provider(provider);
        ensure!(
            provider.get_chain_id().await? == self.zone_chain_id,
            "zone RPC chain ID differs from --zone-chain-id"
        );
        // Snapshot L1 before submitting, so fast settlement cannot be missed.
        let wait = if self.wait_l1 {
            let l1_provider = self.l1.provider(self.zone_chain_id, self.zone_id).await?;
            let from_block = l1_provider.get_block_number().await?;
            Some((l1_provider, from_block))
        } else {
            None
        };
        let outbox = abi::IZoneOutbox::new(abi::OUTBOX, &provider);
        let fee =
            outbox.calculateWithdrawalFee(self.callback_gas_limit).from(sender).call().await?;
        let total = self
            .transfer
            .amount
            .checked_add(fee)
            .ok_or_else(|| eyre::eyre!("withdrawal amount plus fee exceeds uint128"))?;
        // Use the private RPC's gas-price quote without requiring fee-history support.
        let gas_price = provider.get_gas_price().await?;
        self.transfer
            .ensure_allowance(&provider, sender, abi::OUTBOX, total, Some(gas_price))
            .await?;
        let to = self.transfer.to.unwrap_or(sender);
        let pending = outbox
            .requestWithdrawal(
                self.transfer.token,
                to,
                self.transfer.amount,
                self.transfer.memo,
                self.callback_gas_limit,
                self.fallback_recipient.unwrap_or(sender),
                self.callback_data,
                self.reveal_to,
            )
            .max_fee_per_gas(gas_price)
            .max_priority_fee_per_gas(0)
            .from(sender)
            .send()
            .await?;
        let receipt = submitted_receipt("withdrawal", pending).await?;
        ensure!(receipt.status(), "withdrawal reverted: {}", receipt.transaction_hash());
        let hash = receipt.transaction_hash();
        if let Some((l1_provider, from_block)) = wait {
            let _ = sh_status!("Withdrawal requested: {hash}; waiting for L1 delivery.");
            let result = l1::wait_for_withdrawal(
                &provider,
                &l1_provider,
                self.l1.portal()?,
                from_block,
                receipt.block_number().ok_or_else(|| eyre::eyre!("missing zone receipt block"))?,
                hash,
            );
            let l1_hash = tokio::time::timeout(Duration::from_secs(self.wait_timeout), result)
                .await
                .wrap_err_with(|| {
                    format!(
                        "L1 wait timed out; withdrawal {hash} is already submitted; do not resubmit"
                    )
                })?
                .wrap_err_with(|| format!("L1 wait failed for submitted withdrawal {hash}"))?;
            let _ = sh_status!("Withdrawal delivered on L1: {l1_hash}");
        } else {
            let _ = sh_status!("Withdrawal requested on the zone; L1 settlement is asynchronous.");
        }
        print_receipt(&receipt)
    }
}

async fn submitted_receipt(
    action: &str,
    pending: PendingTransactionBuilder<TempoNetwork>,
) -> Result<<TempoNetwork as Network>::ReceiptResponse> {
    let hash = *pending.tx_hash();
    let receipt = pending.with_timeout(Some(Duration::from_secs(120))).get_receipt();
    tokio::time::timeout(Duration::from_secs(120), receipt)
        .await
        .wrap_err_with(|| {
            format!("{action} {hash} was submitted; receipt polling timed out; do not resubmit")
        })?
        .wrap_err_with(|| {
            format!("{action} {hash} was submitted; receipt polling failed; do not resubmit")
        })
}

fn validate_callback(data: &Bytes, gas_limit: u64) -> Result<()> {
    ensure!(
        data.is_empty() || gas_limit > 0,
        "--callback-data requires a nonzero --callback-gas-limit"
    );
    ensure!(
        data.len() <= MAX_CALLBACK_DATA_SIZE,
        "--callback-data exceeds the protocol limit of {MAX_CALLBACK_DATA_SIZE} bytes"
    );
    ensure!(
        gas_limit <= MAX_CALLBACK_GAS_LIMIT,
        "--callback-gas-limit exceeds the protocol limit of {MAX_CALLBACK_GAS_LIMIT}"
    );
    Ok(())
}

fn print_receipt(receipt: &(impl ReceiptResponse + serde::Serialize)) -> Result<()> {
    if shell::is_json() {
        foundry_common::sh_println!("{}", serde_json::to_string(receipt)?)?;
        Ok(())
    } else {
        print_scalar(receipt.transaction_hash())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::tempo::{TempoArgs, TempoSubcommand};

    #[test]
    fn zone_commands_accept_cast_keystore_wallets() {
        for args in [
            vec![
                "tempo",
                "zone",
                "deposit",
                "--portal",
                "0x1111111111111111111111111111111111111111",
                "--amount",
                "1000000",
                "--account",
                "zone-test",
                "--password-file",
                "/tmp/password",
                "--approve",
            ],
            vec![
                "tempo",
                "zone",
                "withdraw",
                "--zone-id",
                "42",
                "--zone-chain-id",
                "1337",
                "--amount",
                "1000000",
                "--account",
                "zone-test",
                "--password-file",
                "/tmp/password",
                "--approve",
            ],
        ] {
            let tempo = TempoArgs::try_parse_from(args).unwrap();
            assert!(matches!(tempo.command, TempoSubcommand::Zone(_)));
        }
    }

    #[test]
    fn callback_limits_match_zone_protocol() {
        assert!(validate_callback(&Bytes::from(vec![0; 1024]), 10_000_000).is_ok());
        assert!(validate_callback(&Bytes::from(vec![0; 1025]), 10_000_000).is_err());
        assert!(validate_callback(&Bytes::new(), 10_000_001).is_err());
        assert!(validate_callback(&Bytes::from_static(&[1]), 0).is_err());
    }

    #[test]
    fn parses_settlement_wait() {
        assert!(
            TempoArgs::try_parse_from([
                "tempo",
                "zone",
                "withdraw",
                "--zone-id",
                "42",
                "--zone-chain-id",
                "1337",
                "--amount",
                "1",
                "--wait-l1",
            ])
            .is_ok()
        );
    }
}
