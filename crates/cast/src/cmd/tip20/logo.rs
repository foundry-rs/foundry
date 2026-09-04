use crate::{
    tempo::tempo_provider,
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_sol_types::SolCall;
use eyre::Result;
use foundry_common::tempo::{Tip20LogoUriValidationError, validate_tip20_logo_uri};
use tempo_contracts::precompiles::ITIP20;

pub(super) fn check(logo_uri: &str) -> Result<()> {
    validate_logo_uri(logo_uri)?;
    sh_println!("Valid TIP-20 logo URI")
}

pub(super) async fn set(
    token: NameOrAddress,
    logo_uri: String,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    validate_logo_uri(&logo_uri)?;
    let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;
    let token = token.resolve(&provider).await?;
    let data = ITIP20::setLogoURICall { newLogoURI: logo_uri }.abi_encode();
    super::send_tip20_transaction(token, data, send_tx, tx_opts).await
}

pub(super) fn validate_logo_uri(logo_uri: &str) -> Result<()> {
    validate_tip20_logo_uri(logo_uri).map_err(|err| match err {
        Tip20LogoUriValidationError::LogoURITooLong => {
            eyre::eyre!("client-side validation failed: LogoURITooLong: logo URI exceeds 256 bytes")
        }
        Tip20LogoUriValidationError::InvalidLogoURI => {
            eyre::eyre!(
                "client-side validation failed: InvalidLogoURI: logo URI must use one of: https, http, ipfs, data"
            )
        }
    })
}
