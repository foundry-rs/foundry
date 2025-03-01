use base64::{engine::general_purpose, Engine};
use std::{fmt, net::SocketAddr};

/// Basic, bearer or raw authentication in http or websocket transport.
///
/// Use to inject username and password or an auth token into requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authorization {
    /// [RFC7617](https://datatracker.ietf.org/doc/html/rfc7617) HTTP Basic Auth.
    Basic(String),
    /// [RFC6750](https://datatracker.ietf.org/doc/html/rfc6750) Bearer Auth.
    Bearer(String),
    /// Raw auth string.
    Raw(String),
}

impl Authorization {
    /// Extract the auth info from a URL.
    pub fn extract_from_url(url: &url::Url) -> Option<Self> {
        let username = url.username();
        let password = url.password().unwrap_or_default();

        // eliminates false positives on the authority
        if username.contains("localhost") || username.parse::<SocketAddr>().is_ok() {
            return None;
        }

        (!username.is_empty() || !password.is_empty()).then(|| Self::basic(username, password))
    }

    /// Instantiate a new basic auth from an authority string.
    pub fn authority(auth: impl AsRef<str>) -> Self {
        let auth_secret = general_purpose::STANDARD.encode(auth.as_ref());
        Self::Basic(auth_secret)
    }

    /// Instantiate a new basic auth from a username and password.
    pub fn basic(username: impl AsRef<str>, password: impl AsRef<str>) -> Self {
        let username = username.as_ref();
        let password = password.as_ref();
        Self::authority(format!("{username}:{password}"))
    }

    /// Instantiate a new bearer auth from the given token.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer(token.into())
    }

    /// Instantiate a new raw auth from the given token.
    pub fn raw(token: impl Into<String>) -> Self {
        Self::Raw(token.into())
    }
}

impl fmt::Display for Authorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic(auth) => write!(f, "Basic {auth}"),
            Self::Bearer(auth) => write!(f, "Bearer {auth}"),
            Self::Raw(auth) => write!(f, "{auth}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn test_extract_from_url_with_basic_auth() {
        let url = Url::parse("http://username:password@domain.com").unwrap();
        let auth = Authorization::extract_from_url(&url).unwrap();

        // Expected Basic auth encoded in base64
        assert_eq!(
            auth,
            Authorization::Basic(general_purpose::STANDARD.encode("username:password"))
        );
    }

    #[test]
    fn test_extract_from_url_no_auth() {
        let url = Url::parse("http://domain.com").unwrap();
        assert!(Authorization::extract_from_url(&url).is_none());
    }

    #[test]
    fn test_extract_from_url_with_localhost() {
        let url = Url::parse("http://localhost:password@domain.com").unwrap();
        assert!(Authorization::extract_from_url(&url).is_none());
    }

    #[test]
    fn test_extract_from_url_with_socket_address() {
        let url = Url::parse("http://127.0.0.1:8080").unwrap();
        assert!(Authorization::extract_from_url(&url).is_none());
    }

    #[test]
    fn test_authority() {
        let auth = Authorization::authority("user:pass");
        assert_eq!(auth, Authorization::Basic(general_purpose::STANDARD.encode("user:pass")));
    }

    #[test]
    fn test_basic() {
        let auth = Authorization::basic("user", "pass");
        assert_eq!(auth, Authorization::Basic(general_purpose::STANDARD.encode("user:pass")));
    }

    #[test]
    fn test_raw() {
        let auth = Authorization::raw("raw_token");
        assert_eq!(auth, Authorization::Raw("raw_token".to_string()));
    }
}
