casttest!(error_decode_with_openchain, |prj, cmd| {
    prj.clear_cache();
    cmd.args(["decode-error", "0x7a0e198500000000000000000000000000000000000000000000000000000000000000650000000000000000000000000000000000000000000000000000000000000064"]).assert_success().stdout_eq(str![[r#"
ValueTooHigh(uint256,uint256)
101
100

"#]]);
});

casttest!(fourbyte, |_prj, cmd| {
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

casttest!(fourbyte_calldata, |_prj, cmd| {
    cmd.args(["4byte-calldata", "0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79"]).assert_success().stdout_eq(str![[r#"
1) "transfer(address,uint256)"
0x0A2AC0c368Dc8eC680a0c98C907656BD97067595
31802608249 [3.18e10]

"#]]);
});

casttest!(fourbyte_calldata_only_selector, |_prj, cmd| {
    cmd.args(["4byte-calldata", "0xa9059cbb"]).assert_success().stdout_eq(str![[r#"
transfer(address,uint256)

"#]]);
});

casttest!(fourbyte_calldata_alias, |_prj, cmd| {
    cmd.args(["4byte-decode", "0xa9059cbb0000000000000000000000000a2ac0c368dc8ec680a0c98c907656bd970675950000000000000000000000000000000000000000000000000000000767954a79"]).assert_success().stdout_eq(str![[r#"
1) "transfer(address,uint256)"
0x0A2AC0c368Dc8eC680a0c98C907656BD97067595
31802608249 [3.18e10]

"#]]);
});

casttest!(fourbyte_event, |_prj, cmd| {
    cmd.args(["4byte-event", "0x7e1db2a1cd12f0506ecd806dba508035b290666b84b096a87af2fd2a1516ede6"])
        .assert_success()
        .stdout_eq(str![[r#"
updateAuthority(address,uint8)

"#]]);
});

casttest!(fourbyte_event_2, |_prj, cmd| {
    cmd.args(["4byte-event", "0xb7009613e63fb13fd59a2fa4c206a992c1f090a44e5d530be255aa17fed0b3dd"])
        .assert_success()
        .stdout_eq(str![[r#"
canCall(address,address,bytes4)

"#]]);
});

// tests cast can decode event with provided signature
casttest!(event_decode_with_sig, |_prj, cmd| {
    cmd.args(["decode-event", "--sig", "MyEvent(uint256,address)", "0x000000000000000000000000000000000000000000000000000000000000004e0000000000000000000000000000000000000000000000000000000000d0004f"]).assert_success().stdout_eq(str![[r#"
78
0x0000000000000000000000000000000000D0004F

"#]]);

    cmd.args(["--json"]).assert_success().stdout_eq(str![[r#"
[
  78,
  "0x0000000000000000000000000000000000D0004F"
]

"#]]);
});

// tests cast can decode event with Sourcify API
casttest!(event_decode_with_sourcify, |prj, cmd| {
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
[
  101,
  "0x0000000000000000000000000000000000D0004F"
]

"#]]);
});

// tests cast can decode error and event when using local sig identifiers cache
forgetest_init!(error_event_decode_with_cache, |prj, cmd| {
    prj.add_source(
        "LocalProjectContract",
        r#"
contract ContractWithCustomError {
    error AnotherValueTooHigh(uint256, address);
    event MyUniqueEventWithinLocalProject(uint256 a, address b);
}
   "#,
    );
    // Store selectors in local cache.
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
