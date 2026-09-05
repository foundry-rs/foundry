//! CLI tests for conversions commands.

use super::*;

// tests that the `cast to-rlp` and `cast from-rlp` commands work correctly
casttest!(rlp, |_prj, cmd| {
    cmd.args(["--to-rlp", "[\"0xaa\", [[\"bb\"]], \"0xcc\"]"]).assert_success().stdout_eq(str![[
        r#"
0xc881aac3c281bb81cc

"#
    ]]);

    cmd.cast_fuse();
    cmd.args(["--from-rlp", "0xcbc58455556666c0c0c2c1c0"]).assert_success().stdout_eq(str![[r#"
[["0x55556666"],[],[],[[[]]]]

"#]]);

    // Build the RLP encoding of 10,000 nested single-item lists without recursively encoding it.
    const NESTING_DEPTH: usize = 10_000;
    let mut encoded_len = 1;
    let mut headers = Vec::with_capacity(NESTING_DEPTH);
    for _ in 0..NESTING_DEPTH {
        let mut header = Vec::new();
        Header { list: true, payload_length: encoded_len }.encode(&mut header);
        encoded_len += header.len();
        headers.push(header);
    }
    let mut deeply_nested = Vec::with_capacity(encoded_len);
    for header in headers.iter().rev() {
        deeply_nested.extend_from_slice(header);
    }
    deeply_nested.push(0x80);

    cmd.cast_fuse().arg("--from-rlp").stdin(hex::encode_prefixed(deeply_nested)).assert_success();
});

casttest!(to_bytes_memory, |_prj, cmd| {
    cmd.args(["to-bytes-memory", "0x1234"]).assert_success().stdout_eq(str![[r#"
0x00000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000

"#]]);
});

casttest!(to_bytes_memory_alias_from_stdin, |_prj, cmd| {
    cmd.arg("tbm").stdin("0x1234\n").assert_success().stdout_eq(str![[r#"
0x00000000000000000000000000000000000000000000000000000000000000021234000000000000000000000000000000000000000000000000000000000000

"#]]);
});

// tests that `cast --to-base` commands are working correctly.
casttest!(to_base, |_prj, cmd| {
    // One value per distinct code path (small positive, u256 max in decimal and
    // hex form, small negative, i256 min) to keep the number of spawned `cast`
    // processes low and avoid timing out on slow Windows/macOS/ARM CI runners.
    let values = [
        "1",
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "-1",
        "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
    ];
    for value in values {
        for subcmd in ["--to-base", "--to-hex", "--to-dec"] {
            if subcmd == "--to-base" {
                for base in ["bin", "oct", "dec", "hex"] {
                    cmd.cast_fuse().args([subcmd, value, base]);
                    assert!(!cmd.assert_success().get_output().stdout_lossy().trim().is_empty());
                }
            } else {
                cmd.cast_fuse().args([subcmd, value]);
                assert!(!cmd.assert_success().get_output().stdout_lossy().trim().is_empty());
            }
        }
    }
});

// tests that `cast --parse-bytes32-address` command is working correctly.
casttest!(parse_bytes32_address, |_prj, cmd| {
    cmd.args([
        "--parse-bytes32-address",
        "0x000000000000000000000000d8da6bf26964af9d7eed9e03e53415d37aa96045",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045

"#]]);
});

casttest!(index7201, |_prj, cmd| {
    cmd.args(["index-erc7201", "example.main"]).assert_success().stdout_eq(str![[r#"
0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500

"#]]);
});

casttest!(index7201_unknown_formula_id, |_prj, cmd| {
    cmd.args(["index-erc7201", "test", "--formula-id", "unknown"]).assert_failure().stderr_eq(
        str![[r#"
Error: unsupported formula ID: unknown

"#]],
    );
});

casttest!(hash_message, |_prj, cmd| {
    cmd.args(["hash-message", "hello"]).assert_success().stdout_eq(str![[r#"
0x50b2c43fd39106bafbba0da34fc430e1f91e3c96ea2acee2bc34119f92b37750

"#]]);

    cmd.cast_fuse().args(["hash-message", "0x68656c6c6f"]).assert_success().stdout_eq(str![[r#"
0x83a0870b6c63a71efdd3b2749ef700653d97454152c4b53fa9b102dc430c7c32

"#]]);
});

casttest!(parse_units, |_prj, cmd| {
    cmd.args(["parse-units", "1.5", "6"]).assert_success().stdout_eq(str![[r#"
1500000

"#]]);

    cmd.cast_fuse().args(["pun", "1.23", "18"]).assert_success().stdout_eq(str![[r#"
1230000000000000000

"#]]);

    cmd.cast_fuse().args(["--parse-units", "1.23", "3"]).assert_success().stdout_eq(str![[r#"
1230

"#]]);
});

casttest!(format_units, |_prj, cmd| {
    cmd.args(["format-units", "1000000", "6"]).assert_success().stdout_eq(str![[r#"
1

"#]]);

    cmd.cast_fuse().args(["--format-units", "2500000", "6"]).assert_success().stdout_eq(str![[
        r#"
2.500000

"#
    ]]);

    cmd.cast_fuse().args(["fun", "1230", "3"]).assert_success().stdout_eq(str![[r#"
1.230

"#]]);

    // Negative values must round-trip correctly instead of wrapping to a huge unsigned garbage
    // value (regression test).
    cmd.cast_fuse().args(["format-units", "--", "-1000000", "6"]).assert_success().stdout_eq(str![
        [r#"
-1

"#]
    ]);
});

// <https://github.com/foundry-rs/foundry/issues> negative wei/unit values must round-trip
// through from-wei/format-units instead of silently wrapping to U256::MAX-derived garbage.
casttest!(from_wei_negative, |_prj, cmd| {
    cmd.args(["from-wei", "--", "-1000000000000000000"]).assert_success().stdout_eq(str![[r#"
-1.000000000000000000

"#]]);

    // Round-trip against the documented inverse command.
    cmd.cast_fuse().args(["to-wei", "-1", "ether"]).assert_success().stdout_eq(str![[r#"
-1000000000000000000

"#]]);

    cmd.cast_fuse().args(["from-wei", "--", "-1000000000000000000"]).assert_success().stdout_eq(
        str![[r#"
-1.000000000000000000

"#]],
    );

    // In-binary oracle: to-fixed-point performs the same underlying arithmetic and must agree.
    cmd.cast_fuse()
        .args(["to-fixed-point", "18", "--", "-1000000000000000000"])
        .assert_success()
        .stdout_eq(str![[r#"
-1.000000000000000000

"#]]);
});

// A negative magnitude whose absolute value exceeds I256::MIN (2^255) cannot be represented as a
// signed 256-bit integer -- must error cleanly, not silently reinterpret as a small positive
// value (regression test for a review finding on the negative-value fix above).
casttest!(from_wei_rejects_magnitude_beyond_i256_range, |_prj, cmd| {
    cmd.args([
        "from-wei",
        "--",
        "-57896044618658097711785492504343953926634992332820282019728792003956564819969",
        "wei",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: value out of range for a signed 256-bit integer

"#]]);
});

casttest!(keccak_stdin_bytes, |_prj, cmd| {
    cmd.args(["keccak"]).stdin("0x12").assert_success().stdout_eq(str![[r#"
0x5fa2358263196dbbf23d1ca7a509451f7a2f64c15837bfbb81298b1e3e24e4fa

"#]]);
});

casttest!(keccak_stdin_bytes_with_newline, |_prj, cmd| {
    cmd.args(["keccak"]).stdin("0x12\n").assert_success().stdout_eq(str![[r#"
0x5fa2358263196dbbf23d1ca7a509451f7a2f64c15837bfbb81298b1e3e24e4fa

"#]]);
});

casttest!(max_int, |_prj, cmd| {
    cmd.cast_fuse().args(["max-int", "int32"]).assert_success().stdout_eq("2147483647\n");
    cmd.cast_fuse().args(["max-uint", "uint256"]).assert_success().stdout_eq(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935\n",
    );
});

casttest!(min_int, |_prj, cmd| {
    cmd.cast_fuse().args(["min-int", "int32"]).assert_success().stdout_eq("-2147483648\n");
    cmd.cast_fuse().args(["min-int", "uint256"]).assert_success().stdout_eq("0\n");
});

casttest!(from_utf8, |_prj, cmd| {
    cmd.cast_fuse().args(["from-utf8", "你好"]).assert_success().stdout_eq("0xe4bda0e5a5bd\n");
});

casttest!(to_utf8, |_prj, cmd| {
    cmd.cast_fuse().args(["to-utf8", "0xe4bda0e5a5bd"]).assert_success().stdout_eq("你好\n");
    cmd.cast_fuse().args(["to-utf8", "0xff"]).assert_success().stdout_eq("�\n");
});

casttest!(to_ascii, |_prj, cmd| {
    cmd.cast_fuse().args(["to-ascii", "0x796f"]).assert_success().stdout_eq("yo\n");
});

casttest!(from_fixed_point, |_prj, cmd| {
    cmd.cast_fuse().args(["from-fixed-point", "0.010", "3"]).assert_success().stdout_eq("10\n");
});

casttest!(concat_hex, |_prj, cmd| {
    cmd.cast_fuse().args(["concat-hex", "0x00", "0x01"]).assert_success().stdout_eq("0x0001\n");
});

casttest!(to_uint256, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["to-uint256", "100"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000064\n");
});

casttest!(to_int256, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["to-int256", "100"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000064\n");
});

casttest!(to_fixed_point, |_prj, cmd| {
    cmd.cast_fuse().args(["to-fixed-point", "10", "2"]).assert_success().stdout_eq("0.10\n");
    cmd.cast_fuse().args(["to-fixed-point", "-10", "3"]).assert_success().stdout_eq("-0.010\n");
    cmd.cast_fuse().args(["to-fixed-point", "10", "0"]).assert_success().stdout_eq("10.\n");
});

casttest!(to_fixed_point_overflow_18446744073709551616, |_prj, cmd| {
    cmd.args(["to-fixed-point", "10", "18446744073709551616"])
        .assert_failure()
        .stderr_eq("Error: decimals out of range: 18446744073709551616\n");
});

casttest!(to_fixed_point_overflow_70000, |_prj, cmd| {
    cmd.args(["to-fixed-point", "10", "70000"])
        .assert_failure()
        .stderr_eq("Error: decimals out of range: 70000\n");
});

casttest!(to_fixed_point_overflow_65536, |_prj, cmd| {
    cmd.args(["to-fixed-point", "10", "65536"])
        .assert_failure()
        .stderr_eq("Error: decimals out of range: 65536\n");
});

casttest!(to_unit, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["to-unit", "1ether", "wei"])
        .assert_success()
        .stdout_eq("1000000000000000000\n");
});

casttest!(to_wei, |_prj, cmd| {
    cmd.cast_fuse().args(["to-wei", "100", "gwei"]).assert_success().stdout_eq("100000000000\n");
});

casttest!(from_rlp_long, |_prj, cmd| {
    cmd.cast_fuse().args(["from-rlp", "0xf8b1a02b5df5f0757397573e8ff34a8b987b21680357de1f6c8d10273aa528a851eaca8080a02838ac1d2d2721ba883169179b48480b2ba4f43d70fcf806956746bd9e83f90380a0e46fff283b0ab96a32a7cc375cecc3ed7b6303a43d64e0a12eceb0bc6bd8754980a01d818c1c414c665a9c9a0e0c0ef1ef87cacb380b8c1f6223cb2a68a4b2d023f5808080a0236e8f61ecde6abfebc6c529441f782f62469d8a2cc47b7aace2c136bd3b1ff08080808080"]).assert_success().stdout_eq("[\"0x2b5df5f0757397573e8ff34a8b987b21680357de1f6c8d10273aa528a851eaca\",\"0x\",\"0x\",\"0x2838ac1d2d2721ba883169179b48480b2ba4f43d70fcf806956746bd9e83f903\",\"0x\",\"0xe46fff283b0ab96a32a7cc375cecc3ed7b6303a43d64e0a12eceb0bc6bd87549\",\"0x\",\"0x1d818c1c414c665a9c9a0e0c0ef1ef87cacb380b8c1f6223cb2a68a4b2d023f5\",\"0x\",\"0x\",\"0x\",\"0x236e8f61ecde6abfebc6c529441f782f62469d8a2cc47b7aace2c136bd3b1ff0\",\"0x\",\"0x\",\"0x\",\"0x\",\"0x\"]\n");
});

casttest!(to_base_uppercase, |_prj, cmd| {
    cmd.cast_fuse().args(["to-dec", "0B10"]).assert_success().stdout_eq("2\n");
    cmd.cast_fuse().args(["to-dec", "0O10"]).assert_success().stdout_eq("8\n");
    cmd.cast_fuse().args(["to-dec", "0X10"]).assert_success().stdout_eq("16\n");
    cmd.cast_fuse().args(["to-dec", "-0X10"]).assert_success().stdout_eq("-16\n");
});

casttest!(to_bytes32, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["to-bytes32", "0x1234"])
        .assert_success()
        .stdout_eq("0x1234000000000000000000000000000000000000000000000000000000000000\n");
});

casttest!(to_bytes32_too_long, |_prj, cmd| {
    cmd.args(["to-bytes32", "000000000000000000000000000000000000000000000000000000000000000000"])
        .assert_failure()
        .stderr_eq("Error: string >32 bytes\n");
});

casttest!(to_bytes_memory_boundaries, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["to-bytes-memory", "0x"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000000\n");
    cmd.cast_fuse().args(["to-bytes-memory", "0xababababababababababababababababababababababababababababababab"]).assert_success().stdout_eq("0x000000000000000000000000000000000000000000000000000000000000001fababababababababababababababababababababababababababababababab00\n");
    cmd.cast_fuse().args(["to-bytes-memory", "0xabababababababababababababababababababababababababababababababab"]).assert_success().stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000020abababababababababababababababababababababababababababababababab\n");
    cmd.cast_fuse().args(["to-bytes-memory", "0xababababababababababababababababababababababababababababababababab"]).assert_success().stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000021ababababababababababababababababababababababababababababababababab00000000000000000000000000000000000000000000000000000000000000\n");
});

casttest!(format_bytes32_string, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["format-bytes32-string", "hello"])
        .assert_success()
        .stdout_eq("0x68656c6c6f000000000000000000000000000000000000000000000000000000\n");
});

casttest!(pad, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["pad", "abcd", "--len", "20"])
        .assert_success()
        .stdout_eq("0x000000000000000000000000000000000000abcd\n");
    cmd.cast_fuse()
        .args(["pad", "abcd", "--len", "20", "--right"])
        .assert_success()
        .stdout_eq("0xabcd000000000000000000000000000000000000\n");
});

casttest!(pad_overflow, |_prj, cmd| {
    cmd.args(["pad", "abcd", "--len", "32768"])
        .assert_failure()
        .stderr_eq("Error: len out of range: 32768\n");
});

casttest!(parse_bytes32_string, |_prj, cmd| {
    cmd.cast_fuse()
        .args([
            "parse-bytes32-string",
            "0x68656c6c6f000000000000000000000000000000000000000000000000000000",
        ])
        .assert_success()
        .stdout_eq("hello\n");
});

casttest!(left_shift, |_prj, cmd| {
    cmd.cast_fuse().args(["left-shift", "16", "1"]).assert_success().stdout_eq("0x20\n");
});

casttest!(right_shift, |_prj, cmd| {
    cmd.cast_fuse().args(["right-shift", "16", "1"]).assert_success().stdout_eq("0x8\n");
});

// `--debug-trace-call` must render a multi-node trace (a call that emits a log AND makes a
// sub-call), exercising the log/sub-call interleaving and nesting through the real pipeline, not
// just in the unit tests. The overridden runtime emits a LOG0 then STATICCALLs the identity
// precompile, so the trace has a child call ordered after the log.
casttest!(cast_call_debug_trace_call_renders_nested_call_and_log, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000cc",
            "run()",
            "--debug-trace-call",
            // runtime: LOG0(0,0); STATICCALL(gas, 0x4, 0,0,0,0); POP; STOP
            "--override-code",
            "0x00000000000000000000000000000000000000cc:0x60006000a060006000600060007300000000000000000000000000000000000000045afa5000",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [21579] 0x00000000000000000000000000000000000000cc::run()
    ├─           data: 0x
    ├─ [15] PRECOMPILES::identity(0x) [staticcall]
    │   └─ ← [Return] 0x
    └─ ← [Return]


Transaction successfully executed.
[GAS]

"#]]);
});
