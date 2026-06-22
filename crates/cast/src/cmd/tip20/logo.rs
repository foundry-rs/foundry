use crate::tx::{SendTxOpts, TxParams};
use alloy_ens::NameOrAddress;
use foundry_cli::utils::LoadConfig;
use foundry_common::{
    provider::ProviderBuilder,
    tempo::{Tip20LogoUriValidationError, validate_tip20_logo_uri},
};
use tempo_alloy::TempoNetwork;

pub(super) fn check(logo_uri: String) -> eyre::Result<()> {
    validate_logo_uri(&logo_uri)?;
    sh_println!("Valid TIP-20 logo URI")?;
    Ok(())
}

pub(super) async fn set(
    token: NameOrAddress,
    logo_uri: String,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> eyre::Result<()> {
    validate_logo_uri(&logo_uri)?;

    let (signer, tempo_access_key) = super::resolve_tip20_signer(&send_tx, &tx_opts).await?;

    let config = send_tx.eth.rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let token_addr = token.resolve(&provider).await?;

    super::send_tip20_transaction(
        NameOrAddress::Address(token_addr),
        "setLogoURI(string)",
        vec![logo_uri],
        send_tx,
        tx_opts,
        signer,
        tempo_access_key,
    )
    .await?;

    Ok(())
}

pub(super) fn validate_logo_uri(logo_uri: &str) -> eyre::Result<()> {
    validate_tip20_logo_uri(logo_uri).map_err(|err| match err {
        Tip20LogoUriValidationError::LogoURITooLong => {
            eyre::eyre!(
                "client-side validation failed: LogoURITooLong: logo URI exceeds 256 bytes"
            )
        }
        Tip20LogoUriValidationError::InvalidLogoURI => {
            eyre::eyre!(
                "client-side validation failed: InvalidLogoURI: logo URI must use one of: https, http, ipfs, data"
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::bytes;
    use alloy_sol_types::SolCall;
    use foundry_common::tempo::TIP20_MAX_LOGO_URI_BYTES;
    use tempo_contracts::precompiles::ITIP20;

    #[test]
    fn validates_empty_and_allowed_schemes_case_insensitively() {
        for uri in [
            "",
            "https://example.com/logo.png",
            "HTTP://example.com/logo.png",
            "ipfs://bafybeigdyrzt",
            "DATA:image/png;base64,abcd",
        ] {
            validate_logo_uri(uri).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_schemes_and_overlong_values() {
        for uri in [
            "ftp://example.com/logo.png",
            "1https://example.com/logo.png",
            "https+foo://example.com/logo.png",
            "example.com/logo.png",
        ] {
            let invalid = validate_logo_uri(uri).unwrap_err().to_string();
            assert!(invalid.contains("client-side validation failed: InvalidLogoURI"));
        }

        let too_long =
            validate_logo_uri(&format!("https://{}", "a".repeat(249))).unwrap_err().to_string();
        assert!(too_long.contains("client-side validation failed: LogoURITooLong"));
    }

    #[test]
    fn validates_logo_uri_byte_length_boundaries() {
        validate_logo_uri(&format!("https://{}", "a".repeat(248))).unwrap();

        let multibyte = format!("https://{}", "é".repeat(124));
        assert_eq!(multibyte.len(), TIP20_MAX_LOGO_URI_BYTES);
        validate_logo_uri(&multibyte).unwrap();

        let too_long = format!("https://{}é", "a".repeat(247));
        assert_eq!(too_long.len(), TIP20_MAX_LOGO_URI_BYTES + 1);
        assert!(validate_logo_uri(&too_long).unwrap_err().to_string().contains("LogoURITooLong"));
    }

    #[test]
    fn set_logo_uri_selector_matches_tip20_t5() {
        let calldata =
            ITIP20::setLogoURICall { newLogoURI: "https://example.com/logo.png".to_string() }
                .abi_encode();

        assert_eq!(&calldata[..4], bytes!("c30ff6df").as_ref());
    }
}
