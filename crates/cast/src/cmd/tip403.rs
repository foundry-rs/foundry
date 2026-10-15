use crate::{
    cmd::{rpc_provider, tip20::send_tip20_transaction},
    tempo::print_payload,
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::Address;
use alloy_sol_types::SolCall;
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::opts::RpcOpts;
use serde_json::json;
use std::str::FromStr;
use tempo_contracts::precompiles::{ITIP403Registry, TIP403_REGISTRY_ADDRESS};
use tempo_primitives::TempoAddressExt;

/// TIP-403 policy registry operations (Tempo).
///
/// Policies created here are referenced by ID from `cast receive-policy set` (sender policy and
/// token filter) and by TIP-20 token compliance configuration.
#[derive(Debug, Parser, Clone)]
pub enum Tip403Subcommand {
    /// Create a new simple (whitelist or blacklist) policy.
    Create {
        /// Policy type to create.
        #[arg(value_enum)]
        policy_type: PolicyKind,

        /// Address authorized to modify the policy.
        #[arg(long, value_parser = NameOrAddress::from_str)]
        admin: NameOrAddress,

        /// Initial member(s) to seed the policy with. Can be specified multiple times.
        #[arg(long = "member", value_name = "ADDRESS", value_parser = NameOrAddress::from_str)]
        accounts: Vec<NameOrAddress>,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Add or remove an account from a whitelist policy.
    Whitelist {
        #[command(flatten)]
        args: MembershipArgs,
    },

    /// Add or remove an account from a blacklist policy.
    Blacklist {
        #[command(flatten)]
        args: MembershipArgs,
    },

    /// Show a policy's type and admin.
    Info {
        /// Policy ID to inspect.
        policy_id: u64,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Check whether an address is authorized by a policy.
    Check {
        /// Policy ID to evaluate.
        policy_id: u64,

        /// Address to check.
        #[arg(value_parser = NameOrAddress::from_str)]
        address: NameOrAddress,

        /// Role to evaluate (defaults to the transfer check). Role variants require T2+.
        #[arg(long, value_enum)]
        role: Option<PolicyRole>,

        #[command(flatten)]
        rpc: RpcOpts,
    },
}

#[derive(Debug, Clone, clap::Args)]
pub struct MembershipArgs {
    /// Whether to add or remove the account.
    #[arg(value_enum)]
    pub action: MembershipAction,

    /// Policy ID to modify.
    pub policy_id: u64,

    /// Account to add or remove.
    #[arg(value_parser = NameOrAddress::from_str)]
    pub account: NameOrAddress,

    #[command(flatten)]
    pub send_tx: SendTxOpts,

    #[command(flatten)]
    pub tx: TxParams,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PolicyKind {
    Whitelist,
    Blacklist,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum MembershipAction {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PolicyRole {
    Sender,
    Recipient,
    MintRecipient,
}

impl Tip403Subcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Create { policy_type, admin, accounts, send_tx, tx } => {
                create(policy_type, admin, accounts, send_tx, tx).await
            }
            Self::Whitelist { args } => modify(PolicyKind::Whitelist, args).await,
            Self::Blacklist { args } => modify(PolicyKind::Blacklist, args).await,
            Self::Info { policy_id, rpc } => info(policy_id, rpc).await,
            Self::Check { policy_id, address, role, rpc } => {
                check(policy_id, address, role, rpc).await
            }
        }
    }
}

async fn create(
    policy_type: PolicyKind,
    admin: NameOrAddress,
    accounts: Vec<NameOrAddress>,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    let provider = rpc_provider(&send_tx.eth.rpc)?;
    let admin = admin.resolve(&provider).await?;

    let mut members = Vec::with_capacity(accounts.len());
    for account in accounts {
        let account = account.resolve(&provider).await?;
        warn_if_virtual(account)?;
        members.push(account);
    }

    // Preview the policy ID the registry would assign. This is the next counter value, so it is
    // only accurate if no other policy is created before this transaction lands.
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, &provider);
    let policy_type = policy_type.to_sol();
    let (expected_id, data) = if members.is_empty() {
        let call = registry.createPolicy(admin, policy_type);
        (call.call().await?, call.calldata().to_vec())
    } else {
        let call = registry.createPolicyWithAccounts(admin, policy_type, members);
        (call.call().await?, call.calldata().to_vec())
    };
    sh_status!(
        "Expected policy ID: {expected_id} (only if this tx is mined before any other policy \
         creation; read the PolicyCreated event for the authoritative ID)"
    )?;

    send_tip20_transaction(TIP403_REGISTRY_ADDRESS, data, send_tx, tx).await
}

async fn modify(kind: PolicyKind, args: MembershipArgs) -> Result<()> {
    let MembershipArgs { action, policy_id, account, send_tx, tx } = args;
    let provider = rpc_provider(&send_tx.eth.rpc)?;
    let account = account.resolve(&provider).await?;
    warn_if_virtual(account)?;

    let flag = matches!(action, MembershipAction::Add);
    let data = match kind {
        PolicyKind::Whitelist => ITIP403Registry::modifyPolicyWhitelistCall {
            policyId: policy_id,
            account,
            allowed: flag,
        }
        .abi_encode(),
        PolicyKind::Blacklist => ITIP403Registry::modifyPolicyBlacklistCall {
            policyId: policy_id,
            account,
            restricted: flag,
        }
        .abi_encode(),
    };
    send_tip20_transaction(TIP403_REGISTRY_ADDRESS, data, send_tx, tx).await
}

async fn info(policy_id: u64, rpc: RpcOpts) -> Result<()> {
    let provider = rpc_provider(&rpc)?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let builtin = match policy_id {
        0 => Some("reject-all"),
        1 => Some("allow-all"),
        _ => None,
    };

    if !registry.policyExists(policy_id).call().await? {
        let payload = json!({ "policy_id": policy_id, "exists": false, "builtin": builtin });
        return print_payload(payload, |_| sh_println!("Policy {policy_id} does not exist"));
    }

    let data = registry.policyData(policy_id).call().await?;
    let payload = json!({
        "policy_id": policy_id,
        "exists": true,
        "builtin": builtin,
        "policy_type": policy_type_label(data.policyType),
        "admin": format!("{}", data.admin),
    });
    print_payload(payload, |payload| {
        sh_println!(
            "Policy ID: {}\n\
             Built-in:  {}\n\
             Type:      {}\n\
             Admin:     {}",
            payload["policy_id"],
            payload["builtin"].as_str().unwrap_or("no"),
            payload["policy_type"].as_str().unwrap_or_default(),
            payload["admin"].as_str().unwrap_or_default(),
        )
    })
}

async fn check(
    policy_id: u64,
    address: NameOrAddress,
    role: Option<PolicyRole>,
    rpc: RpcOpts,
) -> Result<()> {
    let provider = rpc_provider(&rpc)?;
    let address = address.resolve(&provider).await?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let (role_label, authorized) = match role {
        None => ("transfer", registry.isAuthorized(policy_id, address).call().await?),
        Some(PolicyRole::Sender) => {
            ("sender", registry.isAuthorizedSender(policy_id, address).call().await?)
        }
        Some(PolicyRole::Recipient) => {
            ("recipient", registry.isAuthorizedRecipient(policy_id, address).call().await?)
        }
        Some(PolicyRole::MintRecipient) => {
            ("mint-recipient", registry.isAuthorizedMintRecipient(policy_id, address).call().await?)
        }
    };

    let payload = json!({
        "policy_id": policy_id,
        "address": format!("{address}"),
        "role": role_label,
        "authorized": authorized,
    });
    print_payload(payload, |payload| {
        sh_println!(
            "Policy ID:  {}\n\
             Address:    {}\n\
             Role:       {}\n\
             Authorized: {}",
            payload["policy_id"],
            payload["address"].as_str().unwrap_or_default(),
            payload["role"].as_str().unwrap_or_default(),
            payload["authorized"].as_bool().unwrap_or_default(),
        )
    })
}

/// Warn (but don't fail) on virtual members; only T3+ chains reject them on-chain.
fn warn_if_virtual(account: Address) -> Result<()> {
    if account.is_virtual() {
        sh_warn!(
            "{account} looks like a TIP-1022 virtual address; on T3+ chains it is rejected as a \
             literal policy member. Resolve it to its master with `cast vaddr resolve {account}`."
        )?;
    }
    Ok(())
}

impl PolicyKind {
    const fn to_sol(self) -> ITIP403Registry::PolicyType {
        match self {
            Self::Whitelist => ITIP403Registry::PolicyType::WHITELIST,
            Self::Blacklist => ITIP403Registry::PolicyType::BLACKLIST,
        }
    }
}

pub(super) const fn policy_type_label(policy_type: ITIP403Registry::PolicyType) -> &'static str {
    match policy_type {
        ITIP403Registry::PolicyType::WHITELIST => "whitelist",
        ITIP403Registry::PolicyType::BLACKLIST => "blacklist",
        ITIP403Registry::PolicyType::COMPOUND => "compound",
        _ => "unknown",
    }
}
