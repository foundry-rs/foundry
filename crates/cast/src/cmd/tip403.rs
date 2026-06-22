use crate::{
    cmd::tip20::{resolve_tip20_signer, send_tip20_transaction},
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::Address;
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::{
    json::print_json_success,
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
use foundry_common::shell;
use serde_json::{Value, json};
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
    let config = send_tx.eth.rpc.load_config()?;
    let provider = get_provider(&config)?;
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
    let policy_type_sol = policy_type.to_sol();
    let expected_id = if members.is_empty() {
        registry.createPolicy(admin, policy_type_sol).call().await?
    } else {
        registry.createPolicyWithAccounts(admin, policy_type_sol, members.clone()).call().await?
    };
    // Non-authoritative: the real ID is the one in the PolicyCreated event of this transaction.
    sh_status!(
        "Expected policy ID: {expected_id} (only if this tx is mined before any other policy \
         creation; read the PolicyCreated event for the authoritative ID)"
    )?;

    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    let type_arg = (policy_type_sol as u8).to_string();
    let (sig, args) = if members.is_empty() {
        ("createPolicy(address,uint8)", vec![admin.to_string(), type_arg])
    } else {
        (
            "createPolicyWithAccounts(address,uint8,address[])",
            vec![admin.to_string(), type_arg, address_array(&members)],
        )
    };
    send_tip20_transaction(
        NameOrAddress::Address(TIP403_REGISTRY_ADDRESS),
        sig,
        args,
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn modify(kind: PolicyKind, args: MembershipArgs) -> Result<()> {
    let MembershipArgs { action, policy_id, account, send_tx, tx } = args;
    let config = send_tx.eth.rpc.load_config()?;
    let provider = get_provider(&config)?;
    let account = account.resolve(&provider).await?;
    warn_if_virtual(account)?;

    let flag = matches!(action, MembershipAction::Add);
    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    let sig = match kind {
        PolicyKind::Whitelist => "modifyPolicyWhitelist(uint64,address,bool)",
        PolicyKind::Blacklist => "modifyPolicyBlacklist(uint64,address,bool)",
    };
    send_tip20_transaction(
        NameOrAddress::Address(TIP403_REGISTRY_ADDRESS),
        sig,
        vec![policy_id.to_string(), account.to_string(), flag.to_string()],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn info(policy_id: u64, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let builtin = builtin_label(policy_id);

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
    let config = rpc.load_config()?;
    let provider = get_provider(&config)?;
    let address = address.resolve(&provider).await?;
    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, provider);
    let authorized = match role {
        None => registry.isAuthorized(policy_id, address).call().await?,
        Some(PolicyRole::Sender) => registry.isAuthorizedSender(policy_id, address).call().await?,
        Some(PolicyRole::Recipient) => {
            registry.isAuthorizedRecipient(policy_id, address).call().await?
        }
        Some(PolicyRole::MintRecipient) => {
            registry.isAuthorizedMintRecipient(policy_id, address).call().await?
        }
    };

    let payload = json!({
        "policy_id": policy_id,
        "address": format!("{address}"),
        "role": role_label(role),
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

fn address_array(accounts: &[Address]) -> String {
    format!("[{}]", accounts.iter().map(Address::to_string).collect::<Vec<_>>().join(","))
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

impl PolicyKind {
    const fn to_sol(self) -> ITIP403Registry::PolicyType {
        match self {
            Self::Whitelist => ITIP403Registry::PolicyType::WHITELIST,
            Self::Blacklist => ITIP403Registry::PolicyType::BLACKLIST,
        }
    }
}

const fn policy_type_label(policy_type: ITIP403Registry::PolicyType) -> &'static str {
    match policy_type {
        ITIP403Registry::PolicyType::WHITELIST => "whitelist",
        ITIP403Registry::PolicyType::BLACKLIST => "blacklist",
        ITIP403Registry::PolicyType::COMPOUND => "compound",
        _ => "unknown",
    }
}

const fn builtin_label(policy_id: u64) -> Option<&'static str> {
    match policy_id {
        0 => Some("reject-all"),
        1 => Some("allow-all"),
        _ => None,
    }
}

const fn role_label(role: Option<PolicyRole>) -> &'static str {
    match role {
        None => "transfer",
        Some(PolicyRole::Sender) => "sender",
        Some(PolicyRole::Recipient) => "recipient",
        Some(PolicyRole::MintRecipient) => "mint-recipient",
    }
}
