use eyre::Result;
use foundry_common::shell;

/// Confirms that the user intends to disclose an EIP-7702 authorization to an RPC endpoint.
///
/// Returns `false` when the user declines and the command should exit without sending the
/// authorization.
pub(super) fn confirm_auth_rpc_disclosure(force: bool) -> Result<bool> {
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
