//! Tests for provider configuration and RPC error handling.

use alloy_json_rpc::ErrorPayload;

use super::*;

#[test]
fn redacts_url_credentials_and_resource() {
    let url = "https://user:password@example.com:8545/private-key?token=secret#fragment";

    assert_eq!(redact_url(url), "https://example.com:8545/");
    assert_eq!(redact_url("not a URL with secret"), "<redacted>");
}

#[test]
fn invalid_provider_url_error_is_redacted() {
    let builder =
        ProviderBuilder::<AnyNetwork>::new("https://example.com:bad/private-api-key?token=secret");

    let error = builder.url.unwrap_err().to_string();
    assert!(error.contains("<redacted>"));
    assert!(!error.contains("private-api-key"));
    assert!(!error.contains("secret"));
}

#[test]
fn method_not_found_classification_is_exact() {
    let method_not_found = TransportError::ErrorResp(ErrorPayload::method_not_found());
    let internal_error = TransportError::ErrorResp(ErrorPayload::internal_error());
    let http_method_not_found = alloy_transport::TransportErrorKind::http_error(
        403,
        r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not allowed"}}"#.to_string(),
    );
    let http_internal_error = alloy_transport::TransportErrorKind::http_error(
        500,
        r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"internal error"}}"#.to_string(),
    );
    let http_method_not_found_with_diagnostics = alloy_transport::TransportErrorKind::http_error(
        403,
        concat!(
            r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"method not allowed"}}"#,
            "\n\nHTTP diagnostics:\nstatus: 403 Forbidden"
        )
        .to_string(),
    );
    let transport_error = alloy_transport::TransportErrorKind::backend_gone();

    assert!(is_rpc_method_not_found(&method_not_found));
    assert!(is_rpc_method_not_found(&http_method_not_found));
    assert!(is_rpc_method_not_found(&http_method_not_found_with_diagnostics));
    assert!(!is_rpc_method_not_found(&internal_error));
    assert!(!is_rpc_method_not_found(&http_internal_error));
    assert!(!is_rpc_method_not_found(&transport_error));
}

#[test]
fn can_auto_correct_missing_prefix() {
    let builder = ProviderBuilder::<AnyNetwork>::new("localhost:8545");
    assert!(builder.url.is_ok());

    let url = builder.url.unwrap();
    assert_eq!(url, Url::parse("http://localhost:8545").unwrap());
}

#[test]
fn from_config_applies_rpc_transport_options() {
    let config = Config {
        eth_rpc_url: Some("http://example.com".to_string()),
        chain: Some(NamedChain::Polygon.into()),
        eth_rpc_accept_invalid_certs: true,
        eth_rpc_no_proxy: true,
        eth_rpc_timeout: Some(7),
        ..Default::default()
    };

    let builder = ProviderBuilder::<AnyNetwork>::from_config(&config).unwrap();

    assert!(builder.accept_invalid_certs);
    assert!(builder.no_proxy);
    assert_eq!(builder.timeout, Duration::from_secs(7));
    assert_eq!(builder.chain, NamedChain::Polygon);
}

#[test]
fn from_config_with_url_overrides_rpc_url() {
    let config = Config {
        eth_rpc_url: Some("http://configured.example".to_string()),
        chain: Some(NamedChain::Polygon.into()),
        eth_rpc_timeout: Some(7),
        ..Default::default()
    };

    let builder =
        ProviderBuilder::<AnyNetwork>::from_config_with_url(&config, "http://sequence.example")
            .unwrap();

    assert_eq!(builder.url.unwrap().as_str(), "http://sequence.example/");
    assert_eq!(builder.timeout, Duration::from_secs(7));
    assert_eq!(builder.chain, NamedChain::Mainnet);
}
