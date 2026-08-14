use crate::tx::{CastTxBuilder, SenderKind, validate_authorizations};
use alloy_network::Network;
use eyre::Result;
use foundry_cli::opts::CliAuthorizationList;
use foundry_common::shell;

/// Validates the authorization sender and confirms that the user intends to disclose an EIP-7702
/// authorization to an RPC endpoint.
///
/// Returns `false` when the user declines and the command should exit without sending the
/// authorization.
pub(super) fn confirm_auth_rpc_disclosure<N: Network, P, S>(
    builder: &CastTxBuilder<N, P, S>,
    sender: &SenderKind<'_>,
    force: bool,
) -> Result<bool> {
    builder.validate_auth(sender)?;

    confirm_auth_rpc_disclosure_after_validation(force)
}

/// Validates and confirms disclosure before an execution network is resolved from the RPC.
pub(super) fn confirm_auth_rpc_disclosure_before_network_resolution(
    authorizations: &[CliAuthorizationList],
    sender: &SenderKind<'_>,
    force: bool,
) -> Result<bool> {
    validate_authorizations(authorizations, sender)?;

    confirm_auth_rpc_disclosure_after_validation(force)
}

fn confirm_auth_rpc_disclosure_after_validation(force: bool) -> Result<bool> {
    if force {
        return Ok(true);
    }
    if shell::is_quiet() {
        eyre::bail!(
            "EIP-7702 authorization disclosure requires confirmation; pass `--force` to continue with `--quiet`"
        );
    }

    sh_warn!(
        "This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid."
    )?;
    let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
    if !matches!(response.trim(), "y" | "Y") {
        sh_status!("Aborted.")?;
        return Ok(false);
    }

    Ok(true)
}

/// Confirms disclosure when building the transaction will send an EIP-7702 authorization to an
/// RPC endpoint.
pub(super) fn confirm_auth_rpc_disclosure_during_build<'a, N: Network, P, S>(
    builder: &CastTxBuilder<N, P, S>,
    sender: impl Into<SenderKind<'a>>,
    force: bool,
) -> Result<bool> {
    if !builder.will_disclose_auth_during_build() {
        return Ok(true);
    }

    confirm_auth_rpc_disclosure(builder, &sender.into(), force)
}
