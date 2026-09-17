//! Tests for transaction replay arguments and tracing configuration.

use super::*;
use alloy_primitives::address;

#[test]
fn parses_legacy_short_label_alias() {
    let address = address!("0x0000000000000000000000000000000000000001");
    let label = format!("{address}:alice");
    let args = RunArgs::parse_from(["cast run", "0x00", "-l", &label]);

    assert_eq!(args.legacy_labels, vec![label]);
}

#[test]
fn debug_trace_transaction_accepts_label_and_render_flags() {
    let args = RunArgs::try_parse_from([
        "foundry-cli",
        "--debug-trace-transaction",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
        "--label",
        "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth",
        "--disable-labels",
        "--trace-depth",
        "2",
        "--with-local-artifacts",
    ]);
    assert!(args.is_ok(), "--debug-trace-transaction must accept label/rendering flags");
}

#[test]
fn parent_beacon_block_root_is_applied_only_when_the_header_has_one() {
    let networks = NetworkConfigs::default();
    let root = Some(B256::repeat_byte(0x42));
    // Polygon and Scroll run a Cancun or later EVM without populating the header field.
    for (networks, spec_id, root, expected) in [
        (networks, SpecId::CANCUN, None, None),
        (networks, SpecId::CANCUN, root, root),
        (networks, SpecId::SHANGHAI, root, None),
        (networks, SpecId::SHANGHAI, None, None),
        #[cfg(feature = "monad")]
        (NetworkConfigs::with_monad(), SpecId::PRAGUE, root, None),
        #[cfg(feature = "monad")]
        (NetworkConfigs::with_monad(), SpecId::OSAKA, root, None),
        #[cfg(feature = "monad")]
        (NetworkConfigs::with_monad(), SpecId::PRAGUE, None, None),
        #[cfg(feature = "monad")]
        (NetworkConfigs::with_monad(), SpecId::OSAKA, None, None),
    ] {
        assert_eq!(parent_beacon_block_root_for_network(networks, spec_id, root), expected);
    }
}

#[test]
fn debug_trace_transaction_ignores_configured_internal_decoding() {
    let args = RunArgs::parse_from(["cast run", "0x00", "--debug-trace-transaction"]);
    let config = TracingConfig { decode_internal: true, ..Default::default() };

    assert!(!args.resolve_tracing(&config, 0).decode_internal);
}
