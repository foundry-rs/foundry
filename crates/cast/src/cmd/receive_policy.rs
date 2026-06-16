use crate::{
    cmd::tip20::{resolve_tip20_signer, send_tip20_transaction},
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, Bytes, U256, keccak256};
use alloy_sol_types::{SolCall, SolValue};
use clap::{Parser, Subcommand};
use eyre::{Result, WrapErr, ensure};
use foundry_cli::{
    json::print_json_success,
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
use foundry_common::{provider::ProviderBuilder, shell};
use foundry_evm_networks::TEMPO_PRECOMPILE_ADDRESSES;
use serde_json::{Value, json};
use std::str::FromStr;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{
    IReceivePolicyGuard, ITIP403Registry, RECEIVE_POLICY_GUARD_ADDRESS, TIP403_REGISTRY_ADDRESS,
};

/// Account-level receive policy operations (Tempo).
#[derive(Debug, Parser, Clone)]
pub enum ReceivePolicySubcommand {
    /// Set the caller's TIP-403 receive policy.
    Set {
        /// Sender policy ID to evaluate for inbound transfer originators.
        sender_policy_id: u64,

        /// Token filter policy ID to evaluate for inbound TIP-20 tokens.
        token_filter_id: u64,

        /// Address authorized to recover held receipts. Defaults to originator recovery.
        #[arg(long, value_name = "ADDRESS", default_value_t = Address::ZERO)]
        recovery_authority: Address,

        /// Print the calldata and receive-policy warning without sending a transaction.
        #[arg(long, visible_alias = "dry-run")]
        preview: bool,

        /// Suppress the originator-recovery/system-sender warning.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Get an account's configured receive policy.
    Get {
        /// Account whose receive policy should be queried.
        #[arg(value_parser = NameOrAddress::from_str)]
        account: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Validate whether an inbound TIP-20 transfer or mint would be credited or held.
    Validate {
        /// TIP-20 token address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// Inbound transfer sender or mint originator.
        #[arg(value_parser = NameOrAddress::from_str)]
        sender: NameOrAddress,

        /// Intended recipient.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Blocked receive-policy receipt utilities.
    Receipt {
        #[command(subcommand)]
        command: ReceivePolicyReceiptSubcommand,
    },

    /// Claim held TIP-20 funds using a blocked receive-policy receipt.
    Claim {
        /// Desired release target. The guard decides onchain whether to resume or reroute.
        #[arg(value_parser = NameOrAddress::from_str)]
        to: NameOrAddress,

        /// ABI-encoded ReceivePolicyGuard claim receipt.
        receipt: Bytes,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

#[derive(Debug, Subcommand, Clone)]
pub enum ReceivePolicyReceiptSubcommand {
    /// Decode an ABI-encoded ReceivePolicyGuard claim receipt.
    Decode {
        /// ABI-encoded ReceivePolicyGuard claim receipt.
        receipt: Bytes,
    },

    /// Query the held TIP-20 balance for a claim receipt.
    Balance {
        /// ABI-encoded ReceivePolicyGuard claim receipt.
        receipt: Bytes,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Burn held funds for a blocked receipt when authorized by the token.
    Burn {
        /// ABI-encoded ReceivePolicyGuard claim receipt.
        receipt: Bytes,

        #[command(flatten)]
        send_tx: Box<SendTxOpts>,

        #[command(flatten)]
        tx: Box<TxParams>,
    },
}

impl ReceivePolicySubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Set {
                sender_policy_id,
                token_filter_id,
                recovery_authority,
                preview,
                force,
                send_tx,
                tx,
            } => {
                set(
                    sender_policy_id,
                    token_filter_id,
                    recovery_authority,
                    preview,
                    force,
                    send_tx,
                    tx,
                )
                .await?
            }
            Self::Get { account, rpc } => get(account, rpc).await?,
            Self::Validate { token, sender, receiver, rpc } => {
                validate(token, sender, receiver, rpc).await?
            }
            Self::Receipt { command } => match command {
                ReceivePolicyReceiptSubcommand::Decode { receipt } => decode_receipt(receipt)?,
                ReceivePolicyReceiptSubcommand::Balance { receipt, rpc } => {
                    receipt_balance(receipt, rpc).await?
                }
                ReceivePolicyReceiptSubcommand::Burn { receipt, send_tx, tx } => {
                    burn_receipt(receipt, *send_tx, *tx).await?
                }
            },
            Self::Claim { to, receipt, send_tx, tx } => claim(to, receipt, send_tx, tx).await?,
        }

        Ok(())
    }
}

async fn set(
    sender_policy_id: u64,
    token_filter_id: u64,
    recovery_authority: Address,
    preview: bool,
    force: bool,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    let warning = if force {
        None
    } else {
        recovery_warning(sender_policy_id, recovery_authority, &send_tx.eth.rpc).await?
    };

    let call = ITIP403Registry::setReceivePolicyCall {
        senderPolicyId: sender_policy_id,
        tokenFilterId: token_filter_id,
        recoveryAuthority: recovery_authority,
    };
    let calldata = Bytes::from(call.abi_encode());

    if preview {
        let payload = json!({
            "action": "set_receive_policy",
            "registry": format!("{TIP403_REGISTRY_ADDRESS}"),
            "sender_policy_id": sender_policy_id,
            "token_filter_id": token_filter_id,
            "recovery_authority": format!("{recovery_authority}"),
            "recovery_mode": recovery_mode(recovery_authority),
            "calldata": format!("{calldata}"),
            "warning": warning,
        });
        if shell::is_json() {
            print_json_success(payload)?;
        } else {
            sh_println!(
                "Registry:           {TIP403_REGISTRY_ADDRESS}\n\
                 Sender policy ID:   {sender_policy_id}\n\
                 Token filter ID:    {token_filter_id}\n\
                 Recovery authority: {recovery_authority}\n\
                 Recovery mode:      {}\n\
                 Calldata:           {calldata}",
                recovery_mode(recovery_authority)
            )?;
            if let Some(warning) = warning.as_deref() {
                sh_warn!("{warning}")?;
            }
        }
        return Ok(());
    }

    if let Some(warning) = warning.as_deref() {
        sh_warn!("{warning}")?;
    }

    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    send_tip20_transaction(
        NameOrAddress::Address(TIP403_REGISTRY_ADDRESS),
        "setReceivePolicy(uint64,uint64,address)",
        vec![
            sender_policy_id.to_string(),
            token_filter_id.to_string(),
            recovery_authority.to_string(),
        ],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn get(account: NameOrAddress, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let account = account.resolve(&provider).await?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let policy = registry.receivePolicy(account).call().await?;

    let payload = json!({
        "account": format!("{account}"),
        "has_receive_policy": policy.hasReceivePolicy,
        "sender_policy_id": policy.senderPolicyId,
        "sender_policy_type": policy_type(policy.senderPolicyType),
        "token_filter_id": policy.tokenFilterId,
        "token_filter_type": policy_type(policy.tokenFilterType),
        "recovery_authority": format!("{}", policy.recoveryAuthority),
        "recovery_mode": recovery_mode(policy.recoveryAuthority),
    });
    print_payload(payload, |payload| {
        sh_println!(
            "Account:            {}\n\
             Has receive policy: {}\n\
             Sender policy ID:   {}\n\
             Sender policy type: {}\n\
             Token filter ID:    {}\n\
             Token filter type:  {}\n\
             Recovery authority: {}\n\
             Recovery mode:      {}",
            payload["account"].as_str().unwrap_or_default(),
            payload["has_receive_policy"].as_bool().unwrap_or_default(),
            payload["sender_policy_id"],
            payload["sender_policy_type"].as_str().unwrap_or_default(),
            payload["token_filter_id"],
            payload["token_filter_type"].as_str().unwrap_or_default(),
            payload["recovery_authority"].as_str().unwrap_or_default(),
            payload["recovery_mode"].as_str().unwrap_or_default(),
        )
    })
}

async fn validate(
    token: NameOrAddress,
    sender: NameOrAddress,
    receiver: NameOrAddress,
    rpc: RpcOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let token = token.resolve(&provider).await?;
    let sender = sender.resolve(&provider).await?;
    let receiver = receiver.resolve(&provider).await?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let result = registry.validateReceivePolicy(token, sender, receiver).call().await?;
    let delivery_state = if result.authorized { "credited" } else { "held" };

    let payload = json!({
        "token": format!("{token}"),
        "sender": format!("{sender}"),
        "receiver": format!("{receiver}"),
        "authorized": result.authorized,
        "blocked_reason": blocked_reason(result.blockedReason),
        "delivery_state": delivery_state,
    });
    print_payload(payload, |payload| {
        sh_println!(
            "Token:          {}\n\
             Sender:         {}\n\
             Receiver:       {}\n\
             Authorized:     {}\n\
             Blocked reason: {}\n\
             Delivery state: {}",
            payload["token"].as_str().unwrap_or_default(),
            payload["sender"].as_str().unwrap_or_default(),
            payload["receiver"].as_str().unwrap_or_default(),
            payload["authorized"].as_bool().unwrap_or_default(),
            payload["blocked_reason"].as_str().unwrap_or_default(),
            payload["delivery_state"].as_str().unwrap_or_default(),
        )
    })
}

fn decode_receipt(receipt: Bytes) -> Result<()> {
    let decoded = decode_claim_receipt(&receipt)?;
    let payload = receipt_payload(&receipt, &decoded, None);
    print_payload(payload, |payload| {
        print_decoded_receipt(payload)?;
        sh_println!(
            "\nClaim path: cast receive-policy claim {} {}",
            payload["recipient"].as_str().unwrap_or_default(),
            payload["receipt"].as_str().unwrap_or_default(),
        )
    })
}

async fn receipt_balance(receipt: Bytes, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let guard = IReceivePolicyGuard::new(RECEIVE_POLICY_GUARD_ADDRESS, provider);
    let amount = guard.balanceOf(receipt.clone()).call().await?;
    let decoded = decode_claim_receipt(&receipt)?;
    let payload = receipt_payload(&receipt, &decoded, Some(amount));
    print_payload(payload, |payload| {
        print_decoded_receipt(payload)?;
        sh_println!("Held balance: {}", payload["held_balance"].as_str().unwrap_or_default())
    })
}

async fn burn_receipt(receipt: Bytes, send_tx: SendTxOpts, tx: TxParams) -> Result<()> {
    decode_claim_receipt(&receipt)?;
    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    send_tip20_transaction(
        NameOrAddress::Address(RECEIVE_POLICY_GUARD_ADDRESS),
        "burnBlockedReceipt(bytes)",
        vec![format!("{receipt}")],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn claim(to: NameOrAddress, receipt: Bytes, send_tx: SendTxOpts, tx: TxParams) -> Result<()> {
    decode_claim_receipt(&receipt)?;
    let config = send_tx.eth.rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let to = to.resolve(&provider).await?;
    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    send_tip20_transaction(
        NameOrAddress::Address(RECEIVE_POLICY_GUARD_ADDRESS),
        "claim(address,bytes)",
        vec![to.to_string(), format!("{receipt}")],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn recovery_warning(
    sender_policy_id: u64,
    recovery_authority: Address,
    rpc: &RpcOpts,
) -> Result<Option<String>> {
    if recovery_authority != Address::ZERO {
        return Ok(None);
    }

    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let mut blocked = Vec::new();
    for address in TEMPO_PRECOMPILE_ADDRESSES {
        if !registry.isAuthorizedSender(sender_policy_id, *address).call().await.unwrap_or(true) {
            blocked.push(*address);
        }
    }

    if blocked.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!(
        "originator recovery is enabled because recovery authority is 0x0, but sender policy \
         {sender_policy_id} blocks {} Tempo system/precompile sender(s): {}. Receipts created \
         for those senders may not be claimable by a user. Choose receiver or third-party \
         recovery authority when blocking system senders, or pass --force if this is intentional.",
        blocked.len(),
        blocked.iter().map(Address::to_string).collect::<Vec<_>>().join(", ")
    )))
}

fn decode_claim_receipt(receipt: &Bytes) -> Result<IReceivePolicyGuard::ClaimReceiptV1> {
    let decoded = IReceivePolicyGuard::ClaimReceiptV1::abi_decode(receipt)
        .wrap_err("invalid ReceivePolicyGuard claim receipt")?;

    ensure!(
        decoded.version == 1,
        "unsupported ReceivePolicyGuard claim receipt version {}",
        decoded.version
    );
    ensure!(decoded.token != Address::ZERO, "ReceivePolicyGuard claim receipt token is zero");
    ensure!(
        decoded.recipient != RECEIVE_POLICY_GUARD_ADDRESS,
        "ReceivePolicyGuard claim receipt recipient cannot be the guard precompile"
    );
    ensure!(
        matches!(
            decoded.blockedReason,
            reason if reason == ITIP403Registry::BlockedReason::TOKEN_FILTER as u8 ||
                reason == ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8
        ),
        "ReceivePolicyGuard claim receipt blocked reason is not claimable"
    );
    ensure!(
        matches!(
            decoded.kind,
            IReceivePolicyGuard::InboundKind::TRANSFER | IReceivePolicyGuard::InboundKind::MINT
        ),
        "ReceivePolicyGuard claim receipt inbound kind is unknown"
    );

    Ok(decoded)
}

fn receipt_payload(
    receipt: &Bytes,
    decoded: &IReceivePolicyGuard::ClaimReceiptV1,
    amount: Option<U256>,
) -> Value {
    let receipt_key = keccak256(receipt);
    let delivery_state = match amount {
        Some(amount) if amount > U256::ZERO => "held",
        Some(_) => "not_held",
        None => "unknown",
    };
    let mut payload = json!({
        "receipt": format!("{receipt}"),
        "receipt_key": format!("{receipt_key}"),
        "version": decoded.version,
        "token": format!("{}", decoded.token),
        "recovery_authority": format!("{}", decoded.recoveryAuthority),
        "recovery_mode": recovery_mode(decoded.recoveryAuthority),
        "originator": format!("{}", decoded.originator),
        "recipient": format!("{}", decoded.recipient),
        "blocked_at": decoded.blockedAt,
        "blocked_nonce": decoded.blockedNonce,
        "blocked_reason": blocked_reason_u8(decoded.blockedReason),
        "kind": inbound_kind(decoded.kind),
        "memo": format!("{}", decoded.memo),
        "delivery_state": delivery_state,
    });
    if let Some(amount) = amount {
        payload["held_balance"] = json!(amount.to_string());
    }
    payload
}

fn print_decoded_receipt(payload: &Value) -> Result<()> {
    sh_println!(
        "Receipt key:        {}\n\
         Token:              {}\n\
         Recovery authority: {}\n\
         Recovery mode:      {}\n\
         Originator:         {}\n\
         Recipient:          {}\n\
         Blocked at:         {}\n\
         Blocked nonce:      {}\n\
         Blocked reason:     {}\n\
         Kind:               {}\n\
         Memo:               {}\n\
         Delivery state:     {}",
        payload["receipt_key"].as_str().unwrap_or_default(),
        payload["token"].as_str().unwrap_or_default(),
        payload["recovery_authority"].as_str().unwrap_or_default(),
        payload["recovery_mode"].as_str().unwrap_or_default(),
        payload["originator"].as_str().unwrap_or_default(),
        payload["recipient"].as_str().unwrap_or_default(),
        payload["blocked_at"],
        payload["blocked_nonce"],
        payload["blocked_reason"].as_str().unwrap_or_default(),
        payload["kind"].as_str().unwrap_or_default(),
        payload["memo"].as_str().unwrap_or_default(),
        payload["delivery_state"].as_str().unwrap_or_default(),
    )
}

fn print_payload<F>(payload: Value, human: F) -> Result<()>
where
    F: FnOnce(&Value) -> Result<()>,
{
    if shell::is_json() {
        print_json_success(payload)?;
    } else {
        human(&payload)?;
    }
    Ok(())
}

fn recovery_mode(recovery_authority: Address) -> &'static str {
    if recovery_authority == Address::ZERO { "originator" } else { "authority" }
}

const fn policy_type(policy_type: ITIP403Registry::PolicyType) -> &'static str {
    match policy_type {
        ITIP403Registry::PolicyType::WHITELIST => "whitelist",
        ITIP403Registry::PolicyType::BLACKLIST => "blacklist",
        ITIP403Registry::PolicyType::COMPOUND => "compound",
        _ => "unknown",
    }
}

const fn blocked_reason(reason: ITIP403Registry::BlockedReason) -> &'static str {
    match reason {
        ITIP403Registry::BlockedReason::NONE => "none",
        ITIP403Registry::BlockedReason::TOKEN_FILTER => "token_filter",
        ITIP403Registry::BlockedReason::RECEIVE_POLICY => "receive_policy",
        _ => "unknown",
    }
}

const fn blocked_reason_u8(reason: u8) -> &'static str {
    match reason {
        0 => "none",
        1 => "token_filter",
        2 => "receive_policy",
        _ => "unknown",
    }
}

const fn inbound_kind(kind: IReceivePolicyGuard::InboundKind) -> &'static str {
    match kind {
        IReceivePolicyGuard::InboundKind::TRANSFER => "transfer",
        IReceivePolicyGuard::InboundKind::MINT => "mint",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256};

    fn sample_receipt() -> Bytes {
        IReceivePolicyGuard::ClaimReceiptV1::new(
            address!("0000000000000000000000000000000000000010"),
            address!("0000000000000000000000000000000000000020"),
            address!("0000000000000000000000000000000000000030"),
            address!("0000000000000000000000000000000000000040"),
            1_780_000_000,
            7,
            ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8,
            IReceivePolicyGuard::InboundKind::TRANSFER,
            b256!("0000000000000000000000000000000000000000000000000000000000000042"),
        )
        .abi_encode()
        .into()
    }

    #[test]
    fn decodes_guard_claim_receipt() {
        let receipt = sample_receipt();
        let decoded = decode_claim_receipt(&receipt).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.token, address!("0000000000000000000000000000000000000010"));
        assert_eq!(decoded.recoveryAuthority, address!("0000000000000000000000000000000000000020"));
        assert_eq!(decoded.originator, address!("0000000000000000000000000000000000000030"));
        assert_eq!(decoded.recipient, address!("0000000000000000000000000000000000000040"));
        assert_eq!(decoded.blockedNonce, 7);
        assert_eq!(decoded.kind, IReceivePolicyGuard::InboundKind::TRANSFER);
    }

    #[test]
    fn rejects_invalid_guard_claim_receipt() {
        let err = decode_claim_receipt(&Bytes::from_static(&[0xde, 0xad])).unwrap_err();
        assert!(err.to_string().contains("invalid ReceivePolicyGuard claim receipt"));
    }

    #[test]
    fn rejects_semantically_invalid_guard_claim_receipts() {
        let receipt = sample_receipt();
        let decoded = IReceivePolicyGuard::ClaimReceiptV1::abi_decode(&receipt).unwrap();

        let mut bad_version = decoded.clone();
        bad_version.version = 2;
        let err = decode_claim_receipt(&bad_version.abi_encode().into()).unwrap_err();
        assert!(err.to_string().contains("unsupported ReceivePolicyGuard claim receipt version"));

        let mut bad_token = decoded.clone();
        bad_token.token = Address::ZERO;
        let err = decode_claim_receipt(&bad_token.abi_encode().into()).unwrap_err();
        assert!(err.to_string().contains("token is zero"));

        let mut bad_recipient = decoded.clone();
        bad_recipient.recipient = RECEIVE_POLICY_GUARD_ADDRESS;
        let err = decode_claim_receipt(&bad_recipient.abi_encode().into()).unwrap_err();
        assert!(err.to_string().contains("recipient cannot be the guard precompile"));

        let mut bad_reason = decoded;
        bad_reason.blockedReason = ITIP403Registry::BlockedReason::NONE as u8;
        let err = decode_claim_receipt(&bad_reason.abi_encode().into()).unwrap_err();
        assert!(err.to_string().contains("blocked reason is not claimable"));
    }

    #[test]
    fn receipt_payload_preserves_delivery_state_confidence() {
        let receipt = sample_receipt();
        let decoded = decode_claim_receipt(&receipt).unwrap();

        let unknown = receipt_payload(&receipt, &decoded, None);
        assert_eq!(unknown["delivery_state"], "unknown");

        let held = receipt_payload(&receipt, &decoded, Some(U256::from(1)));
        assert_eq!(held["delivery_state"], "held");
        assert_eq!(held["blocked_reason"], "receive_policy");
        assert_eq!(held["kind"], "transfer");
        assert_eq!(held["held_balance"], "1");

        let not_held = receipt_payload(&receipt, &decoded, Some(U256::ZERO));
        assert_eq!(not_held["delivery_state"], "not_held");
        assert_eq!(not_held["held_balance"], "0");
    }

    #[test]
    fn preview_calldata_uses_set_receive_policy_selector() {
        let call = ITIP403Registry::setReceivePolicyCall {
            senderPolicyId: 0,
            tokenFilterId: 1,
            recoveryAuthority: Address::ZERO,
        };
        let calldata = call.abi_encode();
        assert_eq!(&calldata[..4], ITIP403Registry::setReceivePolicyCall::SELECTOR);
    }
}
