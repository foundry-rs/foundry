//! CLI tests for Base execution.

use super::*;

#[cfg(feature = "base")]
casttest!(cast_call_trace_selects_base_network, async |prj, cmd| {
    prj.update_config(|config| {
        config.networks = foundry_evm_networks::NetworkConfigs::with_base();
        config.hardfork = Some("base:Beryl".parse().unwrap());
        config.chain = Some(foundry_config::Chain::from_id(8453));
    });
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    let output = cmd
        .args([
            "call",
            "0x8453000000000000000000000000000000000001",
            "admin()(address)",
            "--rpc-url",
            &rpc,
            "--trace",
            "--chain",
            "8453",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // The registered precompile ABI names the call and decodes the return as a typed address,
    // which renders checksummed rather than as raw hex.
    assert!(output.contains("ActivationRegistry::admin()"), "{output}");
    assert!(output.contains("0xcE3a3bEE7E72E2A24079f3c0Cb3b97740ED425A9"), "{output}");
});

#[cfg(feature = "base")]
casttest!(cast_decode_tx_network_base_short_and_long_equivalent, |_prj, cmd| {
    let tx = "0x7ef90207a0cbde10ec697aff886f95d2514bab434e455620627b9bb8ba33baaaa4d537d62794d45955f4de64f1840e5686e64278da901e263031944200000000000000000000000000000000000007872386f26fc10000872386f26fc1000083096c4980b901a4d764ad0b0001000000000000000000000000000000000000000000000000000000065132000000000000000000000000fd0bf71f60660e2f608ed56e1659c450eb1131200000000000000000000000004200000000000000000000000000000000000010000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000493e000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000a41635f5fd000000000000000000000000ca11bde05977b3631167028862be2a173976ca110000000000000000000000005703b26fe5a7be820db1bf34c901a79da1a46ba4000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

    let via_long = cmd
        .args(["decode-tx", "--network", "base", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let via_short = cmd
        .cast_fuse()
        .args(["decode-tx", "-n", "base", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(via_long, via_short, "--network base and -n base should produce same output");
});
