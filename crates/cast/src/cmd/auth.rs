use super::confirm_continue;
use crate::tx::{CastTxBuilder, InputState, SenderKind, validate_authorizations};
use alloy_network::{Network, TransactionBuilder};
use alloy_provider::Provider;
use eyre::Result;
use foundry_cli::{
    opts::CliAuthorizationList,
    utils::{ResolvedLane, maybe_print_resolved_lane},
};
use foundry_common::{FoundryTransactionBuilder, shell};
use foundry_wallets::TempoAccountsWallet;

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
    confirm_continue()
}

/// Confirms the authorization disclosure, builds the transaction for `sender` and prints the
/// resolved lane. `rpc_signs` marks modes where the RPC signs the transaction, which discloses
/// every authorization regardless of how the request is filled.
///
/// Returns `None` when the user declined.
pub(super) async fn confirm_and_build<'a, N: Network, P: Provider<N>>(
    builder: CastTxBuilder<N, P, InputState>,
    sender: impl Into<SenderKind<'a>>,
    force: bool,
    lane: Option<&ResolvedLane>,
    rpc_signs: bool,
) -> Result<Option<N::TransactionRequest>>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let sender = sender.into();
    let discloses =
        if rpc_signs { builder.has_auth() } else { builder.will_disclose_auth_during_build() };
    if discloses && !confirm_auth_rpc_disclosure(&builder, &sender, force)? {
        return Ok(None);
    }
    let (tx, _) = builder.build(sender).await?;
    maybe_print_resolved_lane(lane, tx.nonce().unwrap_or_default())?;
    Ok(Some(tx))
}

/// [`confirm_and_build`] for a transaction signed by a Tempo access key; also returns the
/// prepared wallet.
pub(super) async fn confirm_and_build_with_tempo_wallet<N: Network, P: Provider<N>>(
    builder: CastTxBuilder<N, P, InputState>,
    wallet: &TempoAccountsWallet,
    force: bool,
    lane: Option<&ResolvedLane>,
) -> Result<Option<(N::TransactionRequest, TempoAccountsWallet)>>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if builder.will_disclose_auth_during_build()
        && !confirm_auth_rpc_disclosure(&builder, &wallet.account().into(), force)?
    {
        return Ok(None);
    }
    let (tx, _, prepared) = builder.build_with_tempo_wallet(wallet).await?;
    maybe_print_resolved_lane(lane, tx.nonce().unwrap_or_default())?;
    Ok(Some((tx, prepared)))
}
