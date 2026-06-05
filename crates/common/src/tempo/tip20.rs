//! Shared TIP-20 helpers.

use std::fmt;

pub const TIP20_MAX_LOGO_URI_BYTES: usize = 256;
pub const TIP20_ALLOWED_LOGO_URI_SCHEMES: &[&str] = &["https", "http", "ipfs", "data"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tip20LogoUriValidationError {
    LogoURITooLong,
    InvalidLogoURI,
}

impl fmt::Display for Tip20LogoUriValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LogoURITooLong => f.write_str("LogoURITooLong"),
            Self::InvalidLogoURI => f.write_str("InvalidLogoURI"),
        }
    }
}

impl std::error::Error for Tip20LogoUriValidationError {}

pub fn validate_tip20_logo_uri(uri: &str) -> Result<(), Tip20LogoUriValidationError> {
    if uri.len() > TIP20_MAX_LOGO_URI_BYTES {
        return Err(Tip20LogoUriValidationError::LogoURITooLong);
    }

    if uri.is_empty() {
        return Ok(());
    }

    let Some((scheme, _)) = uri.split_once(':') else {
        return Err(Tip20LogoUriValidationError::InvalidLogoURI);
    };

    let mut bytes = scheme.bytes();
    if !bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        || !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        || !TIP20_ALLOWED_LOGO_URI_SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    {
        return Err(Tip20LogoUriValidationError::InvalidLogoURI);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_empty_and_allowed_schemes_case_insensitively() {
        for uri in [
            "",
            "https://example.com/logo.png",
            "HTTP://example.com/logo.png",
            "ipfs://bafybeigdyrzt",
            "DATA:image/png;base64,abcd",
        ] {
            validate_tip20_logo_uri(uri).unwrap();
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
            assert_eq!(
                validate_tip20_logo_uri(uri).unwrap_err(),
                Tip20LogoUriValidationError::InvalidLogoURI
            );
        }

        assert_eq!(
            validate_tip20_logo_uri(&format!("https://{}", "a".repeat(249))).unwrap_err(),
            Tip20LogoUriValidationError::LogoURITooLong
        );
    }

    #[test]
    fn validates_logo_uri_byte_length_boundaries() {
        validate_tip20_logo_uri(&format!("https://{}", "a".repeat(248))).unwrap();

        let multibyte = format!("https://{}", "é".repeat(124));
        assert_eq!(multibyte.len(), TIP20_MAX_LOGO_URI_BYTES);
        validate_tip20_logo_uri(&multibyte).unwrap();

        let too_long = format!("https://{}é", "a".repeat(247));
        assert_eq!(too_long.len(), TIP20_MAX_LOGO_URI_BYTES + 1);
        assert_eq!(
            validate_tip20_logo_uri(&too_long).unwrap_err(),
            Tip20LogoUriValidationError::LogoURITooLong
        );
    }
}
