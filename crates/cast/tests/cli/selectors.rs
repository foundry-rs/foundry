use foundry_test_utils::util::OutputExt;
use std::path::Path;

casttest!(flaky_error_decode_with_openchain, |prj, cmd| {
    prj.clear_cache();
    cmd.args(["decode-error", "0x7a0e198500000000000000000000000000000000000000000000000000000000000000650000000000000000000000000000000000000000000000000000000000000064"]).assert_success().stdout_eq(str![[r#"
ValueTooHigh(uint256,uint256)
101
100

"#]]);
});

casttest!(flaky_fourbyte, |_prj, cmd| {
    cmd.args(["4byte", "0xa9059cbb"]).assert_success().stdout_eq(str![[r#"
transfer(address,uint256)

"#]]);
});

casttest!(fourbyte_invalid, |_prj, cmd| {
    cmd.args(["4byte", "0xa9059c"]).assert_failure().stderr_eq(str![[r#"
error: invalid value '0xa9059c' for '[SELECTOR]': invalid string length

For more information, try '--help'.

"#]]);
});

casttest!(flaky_fourbyte_calldata, |_prj, cmd| {
    cmd.args(["4byte-calldata", "0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79"]).assert_success().stdout_eq(str![[r#"
1) "transfer(address,uint256)"
0x0A2AC0c368Dc8eC680a0c98C907656BD97067595
31802608249 [3.18e10]

"#]]);
});

casttest!(flaky_fourbyte_calldata_only_selector, |_prj, cmd| {
    cmd.args(["4byte-calldata", "0xa9059cbb"]).assert_success().stdout_eq(str![[r#"
transfer(address,uint256)

"#]]);
});

casttest!(flaky_fourbyte_calldata_alias, |_prj, cmd| {
    cmd.args(["4byte-decode", "0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79"]).assert_success().stdout_eq(str![[r#"
1) "transfer(address,uint256)"
0x0A2AC0c368Dc8eC680a0c98C907656BD97067595
31802608249 [3.18e10]

"#]]);
});

casttest!(flaky_fourbyte_event, |_prj, cmd| {
    cmd.args(["4byte-event", "0x7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6"])
        .assert_success()
        .stdout_eq(str![[r#"
updateAuthority(address,uint8)

"#]]);
});

casttest!(flaky_fourbyte_event_2, |_prj, cmd| {
    cmd.args(["4byte-event", "0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd"])
        .assert_success()
        .stdout_eq(str![[r#"
canCall(address,address,bytes4)

"#]]);
});

casttest!(flaky_upload_signatures, |_prj, cmd| {
    // test no prefix is accepted as function
    let output = cmd
        .args(["upload-signature", "transfer(address,uint256)"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);

    // test event prefix
    cmd.args(["upload-signature", "event Transfer(address,uint256)"]);
    let output = cmd.assert_success().get_output().stdout_lossy();
    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);

    // test error prefix
    cmd.args(["upload-signature", "error ERC20InsufficientBalance(address,uint256,uint256)"]);
    let output = cmd.assert_success().get_output().stdout_lossy();
    assert!(
        output.contains("Function ERC20InsufficientBalance(address,uint256,uint256): 0xe450d38c"),
        "{}",
        output
    ); // Custom error is interpreted as function

    // test multiple sigs
    cmd.args([
        "upload-signature",
        "event Transfer(address,uint256)",
        "transfer(address,uint256)",
        "approve(address,uint256)",
    ]);
    let output = cmd.assert_success().get_output().stdout_lossy();
    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);
    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);
    assert!(output.contains("Function approve(address,uint256): 0x095ea7b3"), "{}", output);

    // test abi
    cmd.args([
        "upload-signature",
        "event Transfer(address,uint256)",
        "transfer(address,uint256)",
        "error ERC20InsufficientBalance(address,uint256,uint256)",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ERC20Artifact.json")
            .as_os_str()
            .to_str()
            .unwrap(),
    ]);
    let output = cmd.assert_success().get_output().stdout_lossy();
    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);
    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);
    assert!(output.contains("Function approve(address,uint256): 0x095ea7b3"), "{}", output);
    assert!(output.contains("Function decimals(): 0x313ce567"), "{}", output);
    assert!(output.contains("Function allowance(address,address): 0xdd62ed3e"), "{}", output);
    assert!(
        output.contains("Function ERC20InsufficientBalance(address,uint256,uint256): 0xe450d38c"),
        "{}",
        output
    );
});

casttest!(selectors_json_envelope, |_prj, cmd| {
    // bytecode with one function: 0x2125b65b / uint32,address,uint224 / pure
    let bytecode = "6080604052348015600e575f80fd5b50600436106026575f3560e01c80632125b65b14602a575b5f80fd5b603a6035366004603c565b505050565b005b5f805f60608486031215604d575f80fd5b833563ffffffff81168114605f575f80fd5b925060208401356001600160a01b03811681146079575f80fd5b915060408401356001600160e01b03811681146093575f80fd5b80915050925092509256";

    cmd.args(["selectors", bytecode]).assert_success().stdout_eq(str![[r#"
0x2125b65b	uint32,address,uint224	pure

"#]]);

    cmd.args(["--json"]).assert_success().stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":[{"selector":"0x2125b65b","arguments":"uint32,address,uint224","state_mutability":"pure"}],"errors":[],"warnings":[]}

"#]]);
});

casttest!(abi_encode_event_json_envelope, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Transfer(address indexed,address indexed,uint256)",
        "0x0000000000000000000000000000000000000001",
        "0x0000000000000000000000000000000000000002",
        "1000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
[topic1]: 0x0000000000000000000000000000000000000000000000000000000000000001
[topic2]: 0x0000000000000000000000000000000000000000000000000000000000000002
[data]: 0x00000000000000000000000000000000000000000000000000000000000003e8

"#]]);

    // --json must precede the subcommand because `args` uses allow_hyphen_values
    cmd.cast_fuse()
        .args(["--json", "abi-encode-event", "Transfer(address indexed,address indexed,uint256)", "0x0000000000000000000000000000000000000001", "0x0000000000000000000000000000000000000002", "1000"])
        .assert_success()
        .stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":{"topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef","0x0000000000000000000000000000000000000000000000000000000000000001","0x0000000000000000000000000000000000000000000000000000000000000002"],"data":"0x00000000000000000000000000000000000000000000000000000000000003e8"},"errors":[],"warnings":[]}

"#]]);
});

// tests cast can decode event with provided signature
casttest!(event_decode_with_sig, |_prj, cmd| {
    cmd.args(["decode-event", "--sig", "MyEvent(uint256,address)", "0x000000000000000000000000000000000000000000000000000000000000004e0000000000000000000000000000000000000000000000000000000000d0004f"]).assert_success().stdout_eq(str![[r#"
78
0x0000000000000000000000000000000000D0004F

"#]]);

    cmd.args(["--json"]).assert_success().stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":[78,"0x0000000000000000000000000000000000D0004F"],"errors":[],"warnings":[]}

"#]]);
});

// tests cast can decode event with Openchain API
casttest!(flaky_event_decode_with_openchain, |prj, cmd| {
    prj.clear_cache();
    cmd.args(["decode-event", "0xe27c4c1372396a3d15a9922f74f9dfc7c72b1ad6d63868470787249c356454c1000000000000000000000000000000000000000000000000000000000000004e00000000000000000000000000000000000000000000000000000dd00000004e"]).assert_success().stdout_eq(str![[r#"
BaseCurrencySet(address,uint256)
0x000000000000000000000000000000000000004e
15187004358734 [1.518e13]

"#]]);
});

// tests cast can decode error with provided signature
casttest!(error_decode_with_sig, |_prj, cmd| {
    cmd.args(["decode-error", "--sig", "AnotherValueTooHigh(uint256,address)", "0x7191bc6200000000000000000000000000000000000000000000000000000000000000650000000000000000000000000000000000000000000000000000000000D0004F"]).assert_success().stdout_eq(str![[r#"
101
0x0000000000000000000000000000000000D0004F

"#]]);

    cmd.args(["--json"]).assert_success().stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":[101,"0x0000000000000000000000000000000000D0004F"],"errors":[],"warnings":[]}

"#]]);
});

// tests cast can decode error and event when using local sig identifiers cache
forgetest_init!(flaky_error_event_decode_with_cache, |prj, cmd| {
    prj.add_source(
        "LocalProjectContract",
        r#"
contract ContractWithCustomError {
    error AnotherValueTooHigh(uint256, address);
    event MyUniqueEventWithinLocalProject(uint256 a, address b);
}
   "#,
    );
    // Build and cache project selectors.
    cmd.forge_fuse().args(["build"]).assert_success();
    cmd.forge_fuse().args(["selectors", "cache"]).assert_success();

    // Assert cast can decode custom error with local cache.
    cmd.cast_fuse()
        .args(["decode-error", "0x7191bc6200000000000000000000000000000000000000000000000000000000000000650000000000000000000000000000000000000000000000000000000000D0004F"])
        .assert_success()
        .stdout_eq(str![[r#"
AnotherValueTooHigh(uint256,address)
101
0x0000000000000000000000000000000000D0004F

"#]]);
    // Assert cast can decode event with local cache.
    cmd.cast_fuse()
        .args(["decode-event", "0xbd3699995dcc867b64dbb607be2c33be38df9134bef1178df13bfb9446e73104000000000000000000000000000000000000000000000000000000000000004e00000000000000000000000000000000000000000000000000000dd00000004e"])
        .assert_success()
        .stdout_eq(str![[r#"
MyUniqueEventWithinLocalProject(uint256,address)
78
0x00000000000000000000000000000DD00000004e

"#]]);
});

forgetest!(flaky_cache_selectors_from_extra_abis, |prj, cmd| {
    // Create folder with ABI JSON files containing a unique error
    let abis_dir = prj.root().join("external_abis");
    std::fs::create_dir(&abis_dir).unwrap();
    std::fs::write(
        abis_dir.join("test.json"),
        r#"[{
          "type": "error",
          "name": "MyUniqueExtraAbiError",
          "inputs": [
              {"name": "value", "type": "uint256"},
              {"name": "flag", "type": "bool"}
          ]
      }]"#,
    )
    .unwrap();

    cmd.forge_fuse()
        .args(["selectors", "cache", "--extra-abis-path", abis_dir.to_str().unwrap()])
        .assert_success();

    // Verify with cast decode-error (uses local cache via SignaturesIdentifier)
    // Selector for MyUniqueExtraAbiError(uint256,bool) is 0x7819b107
    // Encoded: selector + uint256(42) + bool(true)
    cmd.cast_fuse()
        .args(["decode-error", "0x7819b107000000000000000000000000000000000000000000000000000000000000002a0000000000000000000000000000000000000000000000000000000000000001"])
        .assert_success()
        .stdout_eq(str![[r#"
MyUniqueExtraAbiError(uint256,bool)
42
true

"#]]);
});
