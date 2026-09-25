use super::{
    SafeOperation,
    contracts::ISafe,
    rpc_provider,
    service::{SafeServiceOpts, SafeTransaction},
    signing::sign_safe_hash,
};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_signer::Signer;
use clap::Args;
use eyre::Result;
use foundry_cli::{json::print_scalar, opts::RpcOpts, utils::parse_ether_value};
use foundry_common::abi::{encode_function_args, get_func};
use foundry_wallets::WalletOpts;
use reqwest::Method;
use serde_json::json;

/// CLI arguments for `cast safe propose`.
#[derive(Args, Debug)]
pub struct ProposeArgs {
    /// Safe account address.
    safe: Address,

    /// Transaction target.
    to: Address,

    /// Function signature to call.
    sig: Option<String>,

    /// Function arguments.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// Raw calldata. Cannot be combined with a function signature or arguments.
    #[arg(long, conflicts_with_all = ["sig", "args"])]
    data: Option<Bytes>,

    /// Native token value sent by the Safe.
    #[arg(long, default_value = "0", value_parser = parse_ether_value)]
    value: U256,

    /// Safe operation type.
    #[arg(long, value_enum, default_value_t = SafeOperation::Call)]
    operation: SafeOperation,

    /// Safe transaction gas.
    #[arg(long, default_value = "0")]
    safe_tx_gas: U256,

    /// Base gas reimbursed by the Safe.
    #[arg(long, default_value = "0")]
    base_gas: U256,

    /// Gas price reimbursed by the Safe.
    #[arg(long, default_value = "0")]
    gas_price: U256,

    /// Token used for gas reimbursement.
    #[arg(long, default_value_t = Address::ZERO)]
    gas_token: Address,

    /// Gas reimbursement receiver.
    #[arg(long, default_value_t = Address::ZERO)]
    refund_receiver: Address,

    /// Safe nonce. Defaults to the next queued Transaction Service nonce.
    #[arg(long)]
    nonce: Option<U256>,

    /// Optional origin shown by Safe clients.
    #[arg(long)]
    origin: Option<String>,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,

    #[command(flatten)]
    wallet: Box<WalletOpts>,
}

impl ProposeArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self {
            safe,
            to,
            sig,
            args,
            data,
            value,
            operation,
            safe_tx_gas,
            base_gas,
            gas_price,
            gas_token,
            refund_receiver,
            nonce,
            origin,
            service,
            rpc,
            wallet,
        } = self;
        let data = match (data, sig) {
            (Some(data), None) => data,
            (None, Some(sig)) => encode_function_args(&get_func(&sig)?, &args)?.into(),
            (None, None) => Bytes::new(),
            (Some(_), Some(_)) => unreachable!("enforced by clap"),
        };
        let (provider, chain_id) = rpc_provider(&rpc).await?;
        let nonce = match nonce {
            Some(nonce) => nonce,
            None => {
                let onchain_nonce = ISafe::new(safe, &provider).nonce().call().await?;
                service.next_nonce(chain_id, safe, onchain_nonce).await?
            }
        };
        let mut transaction = SafeTransaction {
            safe,
            to,
            value: value.to_string(),
            data,
            operation: operation as u8,
            safe_tx_gas: safe_tx_gas.to_string(),
            base_gas: base_gas.to_string(),
            gas_price: gas_price.to_string(),
            gas_token,
            refund_receiver,
            nonce: nonce.to_string(),
            safe_tx_hash: B256::ZERO,
            confirmations: Vec::new(),
            is_executed: false,
            transaction_hash: None,
        };
        transaction.safe_tx_hash = transaction.calculate_hash(&provider).await?;
        transaction.show_transaction_summary()?;
        let signer = wallet.signer().await?;
        let signature = sign_safe_hash(&signer, transaction.safe_tx_hash).await?;
        let url = service.endpoint(
            chain_id,
            &format!("v2/safes/{}/multisig-transactions/", transaction.safe.to_checksum(None)),
        )?;
        let body = transaction.proposal_body(signer.address(), signature, origin);
        service.empty_response(service.request(Method::POST, url).json(&body)).await?;
        print_scalar(transaction.safe_tx_hash)
    }
}

/// CLI arguments for `cast safe sign`.
#[derive(Args, Debug)]
pub struct SignArgs {
    /// Safe account address.
    safe: Address,

    /// Safe transaction hash from the Transaction Service.
    safe_tx_hash: B256,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,

    #[command(flatten)]
    wallet: Box<WalletOpts>,
}

impl SignArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, safe_tx_hash, service, rpc, wallet } = self;
        let (provider, chain_id) = rpc_provider(&rpc).await?;
        let transaction = service.get_transaction(chain_id, "v1", safe_tx_hash).await?;
        transaction.verify_hash(safe, &provider).await?;
        transaction.show_transaction_summary()?;
        let signature = sign_safe_hash(&wallet.signer().await?, safe_tx_hash).await?;
        let url = service.endpoint(
            chain_id,
            &format!("v1/multisig-transactions/{safe_tx_hash}/confirmations/"),
        )?;
        let body = json!({ "signature": signature });
        service.empty_response(service.request(Method::POST, url).json(&body)).await?;
        print_scalar(signature)
    }
}
