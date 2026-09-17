//! CLI tests for transaction commands.

use super::*;

// tests that the raw encoded transaction is returned
casttest!(tx_raw, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();

    // <https://etherscan.io/getRawTx?tx=0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.args([
        "tx",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "raw",
        "--rpc-url",
        rpc.as_str(),
    ]).assert_success().stdout_eq(str![[r#"
0xf86d824c548502743b65088275309491da5bf3f8eb72724e6f50ec6c3d199c6355c59c87a0a73f33e9e4cc8025a0428518b1748a08bbeb2392ea055b418538944d30adfc2accbbfa8362a401d3a4a07d6093ab2580efd17c11b277de7664fce56e6953cae8e925bec3313399860470

"#]]);

    // <https://etherscan.io/getRawTx?tx=0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.cast_fuse().args([
        "tx",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "--raw",
        "--rpc-url",
        rpc.as_str(),
    ]).assert_success().stdout_eq(str![[r#"
0xf86d824c548502743b65088275309491da5bf3f8eb72724e6f50ec6c3d199c6355c59c87a0a73f33e9e4cc8025a0428518b1748a08bbeb2392ea055b418538944d30adfc2accbbfa8362a401d3a4a07d6093ab2580efd17c11b277de7664fce56e6953cae8e925bec3313399860470

"#]]);
});

casttest!(tx_to_request_json, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();

    // <https://etherscan.io/getRawTx?tx=0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.args([
        "tx",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "--to-request",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{
  "from": "0x199d5ed7f45f4ee35960cf22eade2076e95b253f",
  "to": "0x91da5bf3f8eb72724e6f50ec6c3d199c6355c59c",
  "gasPrice": "0x2743b6508",
  "gas": "0x7530",
  "value": "0xa0a73f33e9e4cc",
  "input": "0x",
  "nonce": "0x4c54",
  "chainId": "0x1",
  "type": "0x0"
}

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10740>
#[cfg(feature = "optimism")]
casttest!(tx_raw_opstack_deposit, |_prj, cmd| {
    cmd.args([
        "tx",
        "0xf403cba612d1c01c027455c0d97427ccd5f7f99aac30017e065f81d1e30244ea",
        "--raw",
        "-n",
        "optimism",
        "--rpc-url",
        "https://sepolia.base.org",
    ]).assert_success()
            .stdout_eq(str![[r#"
0x7ef90207a0cbde10ec697aff886f95d2514bab434e455620627b9bb8ba33baaaa4d537d62794d45955f4de64f1840e5686e64278da901e263031944200000000000000000000000000000000000007872386f26fc10000872386f26fc1000083096c4980b901a4d764ad0b0001000000000000000000000000000000000000000000000000000000065132000000000000000000000000fd0bf71f60660e2f608ed56e1659c450eb1131200000000000000000000000004200000000000000000000000000000000000010000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000493e000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000a41635f5fd000000000000000000000000ca11bde05977b3631167028862be2a173976ca110000000000000000000000005703b26fe5a7be820db1bf34c901a79da1a46ba4000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000

"#]]);
});

casttest!(tx_raw_tempo, |_prj, cmd| {
    cmd.args([
        "tx",
        "0xa24c6bbeea629a80be79e970a9749d0cbc6ee31625a0b75f585c173ab15a18ec",
        "--raw",
        "-n",
        "tempo",
        "--rpc-url",
        "https://rpc.moderato.tempo.xyz",
    ]).assert_success()
            .stdout_eq(str![[r#"
0x76f8cf82a5bf1485059682f018830494e5f85ef85c9420c0000000000000000000007d9cc57068833ea780b84440c10f190000000000000000000000008a871f4189067637cfc4cc1500abd6244bf1df740000000000000000000000000000000000000000000000000000000005f5e100c08082057e80809420c000000000000000000000000000000000000080c0b841eb100c4cbd96903bf9e97968c0982670bb90fc191ee4544c7ff32d44e901dbea3f6fbdd58255051135c2fe1aa81583a270d96009cbe375f4605ef15971273a4f1b

"#]]);
});

// Test decode-tx with a valid EIP-1559 Ethereum transaction
casttest!(cast_decode_tx_ethereum, |_prj, cmd| {
    // Ethereum mainnet 0x02d2ae7454273bcc02405276b208c03b83ea979ec06aa6f9bc48f81ca343dc1d
    let tx = "0x02f8b1018223e48374667184147d0df48301388094dac17f958d2ee523a2206206994597c13d831ec780b844a9059cbb000000000000000000000000594bd0e0c83d619e375459f0f9b85a17cb8391b400000000000000000000000000000000000000000000000000000000295d9980c080a0704a930876b48fc99cbee17597dc6660c82cd4de5d6f4ace58fe0fcf3bbcb942a07c94560ed0850c9b8e9ab5f3c50902113decea95db3a6bac7c76edd11b7138aa";
    let output = cmd.args(["decode-tx", tx]).assert_success().get_output().stdout.clone();
    let output: String = serde_json::from_slice(&output).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(decoded["type"], "0x2");
    assert_eq!(decoded["nonce"], String::from("0x23e4"));
});

// Test decode-tx with --network tempo accepts the flag and decodes correctly
casttest!(cast_decode_tx_tempo, |_prj, cmd| {
    // Tempo mainnet 0xa26f2dc8ed22d65ad5e5b3acc40295d89c331fd1e79d34b13baa3f6f47b136dc
    let tx = "0x76f9033a821079843b9aca0085098bca5a0083241bc4f9011cf85c9420c000000000000000000000b9537d11c60e8b5080b844095ea7b30000000000000000000000000901aed692c755b870f9605e56baa66c35beff6900000000000000000000000000000000000000000000000000000000000f4240f8bc940901aed692c755b870f9605e56baa66c35beff6980b8a4c79ea485000000000000000000000000b48141c3da5030def992bdc686f0e9a8729206b600000000000000000000000020c000000000000000000000b9537d11c60e8b5000000000000000000000000000000000000000000000000000000000000f424055d3e824159a36fa0d16bbe5c91f497568124441cc6731b8638263d82bfeea6f0000000000000000000000007cfdf901fba309a4a9189a56bede35701aea96dac0a0ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff808469b2c5ef809420c000000000000000000000b9537d11c60e8b5080c0f9016ef83a82107980947cfdf901fba309a4a9189a56bede35701aea96da8469da52c0dbda9420c000000000000000000000b9537d11c60e8b508405f5e100b9012f02a7cb28053c8ee4e5394fc67a0018dc1c622dad5ce3591b8ca13094ae86d11ba61d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a22355068657374624a6a6d4c4c514331622d696f6b5334755f496748307a4d4a6b5a3677385f4e667532596b222c226f726967696e223a2268747470733a2f2f77616c6c65742e74656d706f2e78797a222c2263726f73734f726967696e223a66616c73657d763ed1d6d008091ef06390b2d3150e326795daeba580f0bebc84242d503f13e71328ee2af9426777d4fa5ee148753262ea41b5503522967a6b877c04e5c0c2a7e1af1c624e48eba171e5f521d8f4c89f80b04ecb3f5ba6060109ccb56d344ed129f2a97fb8757cb3cbdcbc636b949fedad4b74490af444a49f5b83d6e0bb0750b8560339e87712af0f3c9c3c1f7c9c57190bb8c8db125d721b2cbf2ba3ac52332b11e0f5d397406da076668a40840135e950e5967bc5f032e8502e33ea91899b90cd8f6106d27efb934a4dac5dcfd0ca5c491733faf91c1c";
    let output = cmd
        .args(["decode-tx", "--network", "tempo", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let output: String = serde_json::from_slice(&output).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(decoded["type"], "0x76");
    assert_eq!(decoded["feeToken"], "0x20c000000000000000000000b9537d11c60e8b50");
});

// Test decode-tx with invalid hex input
casttest!(cast_decode_tx_invalid, |_prj, cmd| {
    cmd.args(["decode-tx", "0xinvalid"]).assert_failure();
});

// Test decode-tx auto-detects the Tempo network from the `0x76` type byte without `--network`,
// producing the same output as passing `--network tempo` explicitly.
casttest!(cast_decode_tx_tempo_autodetect, |_prj, cmd| {
    let tx = "0x76f8cf82a5bf1485059682f018830494e5f85ef85c9420c0000000000000000000007d9cc57068833ea780b84440c10f190000000000000000000000008a871f4189067637cfc4cc1500abd6244bf1df740000000000000000000000000000000000000000000000000000000005f5e100c08082057e80809420c000000000000000000000000000000000000080c0b841eb100c4cbd96903bf9e97968c0982670bb90fc191ee4544c7ff32d44e901dbea3f6fbdd58255051135c2fe1aa81583a270d96009cbe375f4605ef15971273a4f1b";

    let auto = cmd.args(["decode-tx", tx]).assert_success().get_output().stdout.clone();

    let with_flag = cmd
        .cast_fuse()
        .args(["decode-tx", "--network", "tempo", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(auto, with_flag, "auto-detected and --network tempo output should match");

    let output: String = serde_json::from_slice(&auto).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(decoded["type"], "0x76");
});

// Test that `--network ethereum` forces Ethereum decoding and rejects a Tempo tx (type `0x76`),
// so the flag remains a meaningful override rather than falling back to auto-detection.
casttest!(cast_decode_tx_network_ethereum_rejects_tempo, |_prj, cmd| {
    let tx = "0x76f8cf82a5bf1485059682f018830494e5f85ef85c9420c0000000000000000000007d9cc57068833ea780b84440c10f190000000000000000000000008a871f4189067637cfc4cc1500abd6244bf1df740000000000000000000000000000000000000000000000000000000005f5e100c08082057e80809420c000000000000000000000000000000000000080c0b841eb100c4cbd96903bf9e97968c0982670bb90fc191ee4544c7ff32d44e901dbea3f6fbdd58255051135c2fe1aa81583a270d96009cbe375f4605ef15971273a4f1b";

    cmd.args(["decode-tx", "--network", "ethereum", tx]).assert_failure();
});

// Test that `--network tempo` and `-n tempo` (short form) produce identical output for decode-tx.
// Uses a known Tempo mainnet transaction.
casttest!(cast_decode_tx_network_flag_short_and_long_equivalent, |_prj, cmd| {
    let tx = "0x76f8cf82a5bf1485059682f018830494e5f85ef85c9420c0000000000000000000007d9cc57068833ea780b84440c10f190000000000000000000000008a871f4189067637cfc4cc1500abd6244bf1df740000000000000000000000000000000000000000000000000000000005f5e100c08082057e80809420c000000000000000000000000000000000000080c0b841eb100c4cbd96903bf9e97968c0982670bb90fc191ee4544c7ff32d44e901dbea3f6fbdd58255051135c2fe1aa81583a270d96009cbe375f4605ef15971273a4f1b";

    let via_long = cmd
        .args(["decode-tx", "--network", "tempo", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    let via_short = cmd
        .cast_fuse()
        .args(["decode-tx", "-n", "tempo", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(via_long, via_short, "--network tempo and -n tempo should produce same output");
});

// Test that `--network optimism` and `-n optimism` produce identical output for decode-tx.
// Uses a known OP-stack deposit transaction (same tx as tx_raw_opstack_deposit test).
#[cfg(feature = "optimism")]
casttest!(cast_decode_tx_network_optimism_short_and_long_equivalent, |_prj, cmd| {
    let tx = "0x7ef90207a0cbde10ec697aff886f95d2514bab434e455620627b9bb8ba33baaaa4d537d62794d45955f4de64f1840e5686e64278da901e263031944200000000000000000000000000000000000007872386f26fc10000872386f26fc1000083096c4980b901a4d764ad0b0001000000000000000000000000000000000000000000000000000000065132000000000000000000000000fd0bf71f60660e2f608ed56e1659c450eb1131200000000000000000000000004200000000000000000000000000000000000010000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000493e000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000a41635f5fd000000000000000000000000ca11bde05977b3631167028862be2a173976ca110000000000000000000000005703b26fe5a7be820db1bf34c901a79da1a46ba4000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

    let via_long = cmd
        .args(["decode-tx", "--network", "optimism", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    let via_short = cmd
        .cast_fuse()
        .args(["decode-tx", "-n", "optimism", tx])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(
        via_long, via_short,
        "--network optimism and -n optimism should produce same output"
    );
});

casttest!(tx_using_sender_and_nonce, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    // <https://etherscan.io/tx/0x5bcd22734cca2385dc25b2d38a3d33a640c5961bd46d390dff184c894204b594>
    let args = vec![
        "tx",
        "--from",
        "0x4648451b5F87FF8F0F7D622bD40574bb97E25980",
        "--nonce",
        "113642",
        "--rpc-url",
        rpc.as_str(),
    ];
    cmd.args(args).assert_success().stdout_eq(str![[r#"

blockHash            0x29518c1cea251b1bda5949a9b039722604ec1fb99bf9d8124cfe001c95a50bdc
blockNumber          22287055
from                 0x4648451b5F87FF8F0F7D622bD40574bb97E25980
transactionIndex     230
effectiveGasPrice    363392048
hash                 0x5bcd22734cca2385dc25b2d38a3d33a640c5961bd46d390dff184c894204b594
type                 2
chainId              1
nonce                113642
gasLimit             350000
maxFeePerGas         675979146
maxPriorityFeePerGas 1337
to                   0xdAC17F958D2ee523a2206206994597C13D831ec7
value                0
accessList           []
input                0xa9059cbb000000000000000000000000568766d218d82333dd4dae933ddfcda5da26625000000000000000000000000000000000000000000000000000000000cc3ed109
r                    0x1e92d3e1ca69109a1743fc4b3cf9dff58630bc9f429cea3c3fe311506264e36c
s                    0x793947d4bbdce56a1a5b2b3525c46f01569414a22355f4883b5429668ab0f51a
yParity              1
...
"#]]);
});

casttest!(recover_authority, |_prj, cmd| {
    let auth = r#"{
        "chainId": "0x1",
        "address": "0xb684710e6d5914ad6e64493de2a3c424cc43e970",
        "nonce": "0x3dc1",
        "yParity": "0x1",
        "r": "0x2f15ba55009fcd3682cd0f9c9645dd94e616f9a969ba3f1a5a2d871f9fe0f2b4",
        "s": "0x53c332a83312d0b17dd4c16eeb15b1ff5223398b14e0a55c70762e8f3972b7a5"
    }"#;
    cmd.args(["recover-authority", auth]).assert_success().stdout_eq(str![[r#"
0x17816E9A858b161c3E37016D139cf618056CaCD4

"#]]);
});
