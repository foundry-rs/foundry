use crate::{
    cmd::send::SendTxArgs,
    tempo::{print_payload, tempo_provider},
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, B256, keccak256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use eyre::{Result, WrapErr};
use foundry_cli::opts::RpcOpts;
use serde_json::json;
use std::{fmt, str::FromStr};
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::IRolesAuth;

/// Name of the root admin role, whose identifier is the zero hash.
const DEFAULT_ADMIN_ROLE: &str = "DEFAULT_ADMIN_ROLE";

/// Names of the TIP-20 roles whose identifier is the keccak256 hash of the name.
const HASHED_ROLES: [&str; 5] =
    ["ISSUER_ROLE", "PAUSE_ROLE", "UNPAUSE_ROLE", "BURN_BLOCKED_ROLE", "BURN_AT_ROLE"];

/// A TIP-20 role identifier, parsed from a role name or a raw 32-byte hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tip20Role(B256);

impl Tip20Role {
    /// Returns the canonical name of a role defined by the TIP-20 precompile.
    fn name(&self) -> Option<&'static str> {
        if self.0.is_zero() {
            return Some(DEFAULT_ADMIN_ROLE);
        }
        HASHED_ROLES.into_iter().find(|name| keccak256(name) == self.0)
    }

    /// Returns the role name, or its hash for roles the TIP-20 precompile does not define.
    fn label(&self) -> String {
        self.name().map_or_else(|| self.0.to_string(), str::to_string)
    }
}

impl FromStr for Tip20Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(hash) = s.parse() {
            return Ok(Self(hash));
        }

        // Accept `issuer`, `burn-at`, `BURN_AT_ROLE`, and similar spellings.
        let mut name = s.to_ascii_uppercase().replace('-', "_");
        if !name.ends_with("_ROLE") {
            name.push_str("_ROLE");
        }
        if name == DEFAULT_ADMIN_ROLE || name == "ADMIN_ROLE" {
            return Ok(Self(B256::ZERO));
        }
        if HASHED_ROLES.contains(&name.as_str()) {
            return Ok(Self(keccak256(&name)));
        }
        Err(format!(
            "unknown TIP-20 role `{s}`; expected one of admin, issuer, pause, unpause, \
             burn-blocked, burn-at, or a 32-byte role hash"
        ))
    }
}

impl fmt::Display for Tip20Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name} ({})", self.0),
            None => write!(f, "{}", self.0),
        }
    }
}

/// Whether a role membership update grants or revokes the role.
#[derive(Clone, Copy, Debug)]
pub(super) enum RoleUpdate {
    Grant,
    Revoke,
}

/// Grants or revokes `role` for `account` on `token`.
pub(super) async fn update(
    update: RoleUpdate,
    token: NameOrAddress,
    role: Tip20Role,
    account: NameOrAddress,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;
    let token = token.resolve(&provider).await?;
    let account = account.resolve(&provider).await?;

    let (signer, access_key) = super::resolve_tip20_signer(&send_tx, &tx).await?;
    // The sender is only known up front for local signers and access keys. Browser wallets skip
    // the check, which the precompile still enforces on-chain.
    let sender = match (&access_key, &signer) {
        (Some(wallet), _) => Some(wallet.account()),
        (None, Some(signer)) => Some(signer.address()),
        (None, None) => None,
    };
    if let Some(sender) = sender {
        ensure_role_admin(&provider, update, token, role, sender).await?;
    }

    let data = match update {
        RoleUpdate::Grant => IRolesAuth::grantRoleCall { role: role.0, account }.abi_encode(),
        RoleUpdate::Revoke => IRolesAuth::revokeRoleCall { role: role.0, account }.abi_encode(),
    };
    SendTxArgs::contract_call(NameOrAddress::Address(token), data, send_tx, tx)
        .run_generic::<TempoNetwork>(signer, access_key)
        .await
}

/// Prints whether `account` holds `role` on `token`.
pub(super) async fn has_role(
    token: NameOrAddress,
    role: Tip20Role,
    account: NameOrAddress,
    rpc: RpcOpts,
) -> Result<()> {
    let (_, provider) = tempo_provider(&rpc)?;
    let token = token.resolve(&provider).await?;
    let account = account.resolve(&provider).await?;
    let has_role = IRolesAuth::new(token, &provider)
        .hasRole(account, role.0)
        .call()
        .await
        .wrap_err_with(|| format!("failed to read roles of TIP-20 token {token}"))?;

    let payload = json!({
        "token": format!("{token}"),
        "role": format!("{}", role.0),
        "role_name": role.name(),
        "account": format!("{account}"),
        "has_role": has_role,
    });
    print_payload(payload, |_| {
        sh_println!(
            "Token:    {token}\n\
             Role:     {role}\n\
             Account:  {account}\n\
             Has role: {has_role}"
        )
    })
}

/// Fails early when `sender` does not hold the admin role of `role`, instead of submitting a
/// transaction the precompile rejects with a bare `Unauthorized()`.
async fn ensure_role_admin<P: Provider<TempoNetwork>>(
    provider: &P,
    update: RoleUpdate,
    token: Address,
    role: Tip20Role,
    sender: Address,
) -> Result<()> {
    let roles = IRolesAuth::new(token, provider);
    let read_err = || format!("failed to read roles of TIP-20 token {token}");
    let admin_role = Tip20Role(roles.getRoleAdmin(role.0).call().await.wrap_err_with(read_err)?);
    if !roles.hasRole(sender, admin_role.0).call().await.wrap_err_with(read_err)? {
        let action = match update {
            RoleUpdate::Grant => "grant",
            RoleUpdate::Revoke => "revoke",
        };
        eyre::bail!(
            "{sender} cannot {action} {} on TIP-20 token {token}: it does not hold {}, the role's \
             admin role",
            role.label(),
            admin_role.label()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn parses_role_names_and_hashes() {
        let burn_at = Tip20Role(keccak256("BURN_AT_ROLE"));
        for spelling in ["burn-at", "burn_at", "BURN_AT", "BURN_AT_ROLE", "burn-at-role"] {
            assert_eq!(spelling.parse(), Ok(burn_at), "{spelling}");
        }
        for spelling in ["admin", "default-admin", "DEFAULT_ADMIN_ROLE"] {
            assert_eq!(spelling.parse(), Ok(Tip20Role(B256::ZERO)), "{spelling}");
        }

        let custom = B256::with_last_byte(0xab);
        assert_eq!(custom.to_string().parse(), Ok(Tip20Role(custom)));
        assert_eq!(Tip20Role(custom).name(), None);
        assert_eq!(burn_at.name(), Some("BURN_AT_ROLE"));

        assert!("minter".parse::<Tip20Role>().is_err());
    }
}
