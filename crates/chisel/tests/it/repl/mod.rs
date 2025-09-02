mod session;
use session::ChiselSession;

macro_rules! repl_test {
    ($name:ident, | $cmd:ident | $test:expr) => {
        repl_test!($name, "", |$cmd| $test);
    };
    ($name:ident, $flags:expr, | $cmd:ident | $test:expr) => {
        repl_test!($name, $flags, init = false, |$cmd| $test);
    };
    ($name:ident, $flags:expr,init = $init:expr, | $cmd:ident | $test:expr) => {
        #[tokio::test]
        #[allow(unused_mut)]
        async fn $name() {
            let mut $cmd = ChiselSession::new(stringify!($name), $flags, $init).await;
            $test;
            return (); // Fix "go to definition" due to `tokio::test`.
        }
    };
}

repl_test!(test_repl_help, |repl| {
    repl.sendln("!h").await;
    repl.expect("Chisel help").await;
});

// Test abi encode/decode.
repl_test!(test_abi_encode_decode, |repl| {
    repl.sendln("bytes memory encoded = abi.encode(42, \"hello\")").await;
    repl.sendln("(uint num, string memory str) = abi.decode(encoded, (uint, string))").await;
    repl.sendln("num").await;
    repl.expect("42").await;
    repl.sendln("str").await;
    repl.expect("hello").await;
});

// Test 0x prefixed strings.
repl_test!(test_hex_string_interpretation, |repl| {
    repl.sendln("string memory s = \"0x1234\"").await;
    repl.sendln("s").await;
    // Should be treated as string, not hex literal.
    repl.expect("0x1234").await;
});

// Test cheatcodes availability.
repl_test!(test_cheatcodes_available, "", init = true, |repl| {
    repl.sendln("address alice = address(0x1)").await;

    repl.sendln("alice.balance").await;
    repl.expect("Decimal: 0").await;

    repl.sendln("vm.deal();").await;
    repl.expect("Wrong argument count for function call").await;

    repl.sendln("vm.deal(alice, 1 ether);").await;

    repl.sendln("alice.balance").await;
    repl.expect("Decimal: 1000000000000000000").await;
});

// Test empty inputs.
repl_test!(test_empty_input, |repl| {
    repl.sendln("   \n \n\n    \t \t \n \n\t\t\t\t \n \n").await;
});

// Issue #4130: Test type(intN).min correctness.
repl_test!(test_int_min_values, |repl| {
    repl.sendln("type(int8).min").await;
    repl.expect("-128").await;
    repl.sendln("type(int256).min").await;
    repl.expect("-57896044618658097711785492504343953926634992332820282019728792003956564819968")
        .await;
});

// Issue #4393: Test edit command with traces.
// TODO: test `!edit`
// repl_test!(test_edit_with_traces, |repl| {
//     repl.sendln("!traces").await;
//     repl.sendln("uint x = 42").await;
//     repl.sendln("!edit").await;
//     // Should open editor without errors.
//     repl.expect("Running").await;
// });

// Test tuple support.
repl_test!(test_tuples, |repl| {
    repl.sendln("(uint a, uint b) = (1, 2)").await;
    repl.sendln("a").await;
    repl.expect("Decimal: 1").await;
    repl.sendln("b").await;
    repl.expect("Decimal: 2").await;
});

// Issue #4467: Test import.
repl_test!(test_import, "", init = true, |repl| {
    repl.sendln("import {Counter} from \"src/Counter.sol\"").await;
    repl.sendln("Counter c = new Counter()").await;
    // TODO: pre-existing inspection failure.
    // repl.sendln("c.number()").await;
    repl.sendln("uint x = c.number();\nx").await;
    repl.expect("Decimal: 0").await;
    repl.sendln("c.increment();").await;
    // repl.sendln("c.number()").await;
    repl.sendln("x = c.number();\nx").await;
    repl.expect("Decimal: 1").await;
});

// Issue #4617: Test code after assembly return.
repl_test!(test_assembly_return, |repl| {
    repl.sendln("uint x = 1;").await;
    repl.sendln("assembly { mstore(0x0, 0x1337) return(0x0, 0x20) }").await;
    repl.sendln("x = 2;").await;
    repl.sendln("!md").await;
    // Should work without errors.
    repl.expect("[0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337")
        .await;
});

// Issue #4652: Test commands with trailing whitespace.
repl_test!(test_trailing_whitespace, |repl| {
    repl.sendln("uint x = 42   ").await;
    repl.sendln("x").await;
    repl.expect("Decimal: 42").await;
});

// Issue #4652: Test that solc flags are respected.
repl_test!(test_solc_flags, "--use 0.8.23", |repl| {
    repl.sendln("pragma solidity 0.8.24;").await;
    repl.expect("invalid solc version").await;
});

// Issue #4915: `chisel eval`
repl_test!(test_eval_subcommand, "eval type(uint8).max", |repl| {
    repl.expect("Decimal: 255").await;
});

// Issue #4938: Test memory/stack dumps with assembly.
repl_test!(test_assembly_memory_dump, |repl| {
    let input = r#"
uint256 value = 12345;
string memory str;
assembly {
    str := add(mload(0x40), 0x80)
    mstore(0x40, add(str, 0x20))
    mstore(str, 0)
    let end := str
}
"#;
    repl.sendln_raw(input.trim()).await;
    repl.expect_prompts(3).await;
    repl.sendln("value").await;
    repl.expect("Decimal: 12345").await;
    repl.sendln("!md").await;
    repl.expect("[0x00:0x20]").await;
});

// Issue #5051, #8978: Test EVM version normalization.
repl_test!(test_evm_version_normalization, "--use 0.7.6 --evm-version london", |repl| {
    repl.sendln("uint x;\nx").await;
    repl.expect("Decimal: 0").await;
});

// Issue #5481: Test function return values are displayed.
repl_test!(test_function_return_display, |repl| {
    repl.sendln("function add(uint a, uint b) public pure returns (uint) { return a + b; }").await;
    repl.sendln("add(2, 3)").await;
    repl.expect("Decimal: 5").await;
});

// Issue #5737: Test bytesN return types.
repl_test!(test_bytes_length_type, |repl| {
    repl.sendln("bytes10 b = bytes10(0)").await;
    repl.sendln("b.length").await;
    repl.expect("Decimal: 10").await;
});

// Issue #5737: Test bytesN indexing return type.
repl_test!(test_bytes_index_type, |repl| {
    repl.sendln("bytes32 b = bytes32(uint256(0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20))").await;
    repl.sendln("b[3]").await;
    repl.expect("Data: 0x0400000000000000000000000000000000000000000000000000000000000000").await;
});

// Issue #6618: Test fetching interface with structs.
repl_test!(test_fetch_interface_with_structs, |repl| {
    repl.sendln_raw("!fe 0x5ff137d4b0fdcd49dca30c7cf57e578a026d2789 IEntryPoint").await;
    repl.expect(
        "Added 0x5FF137D4b0FDCD49DcA30c7CF57E578a026d2789's interface to source as `IEntryPoint`",
    )
    .await;
    repl.expect_prompt().await;
    repl.sendln("uint256 x = 1;\nx").await;
    repl.expect("Decimal: 1").await;
});

// Issue #7035: Test that hex strings aren't checksummed as addresses.
repl_test!(test_hex_string_no_checksum, |repl| {
    repl.sendln("function test(string memory s) public pure returns (string memory) { return s; }")
        .await;
    repl.sendln("test(\"0xe5f3af50fe5d0bf402a3c6f55ccc47d4307922d4\")").await;
    // Should return the exact string, not checksummed.
    repl.expect("0xe5f3af50fe5d0bf402a3c6f55ccc47d4307922d4").await;
});

// Issue #7050: Test enum min/max operations.
repl_test!(test_enum_min_max, |repl| {
    repl.sendln("enum Color { Red, Green, Blue }").await;
    repl.sendln("type(Color).min").await;
    repl.expect("Decimal: 0").await;
    repl.sendln("type(Color).max").await;
    repl.expect("Decimal: 2").await;
});

// Issue #9377: Test correct hex formatting for uint256.
repl_test!(test_uint256_hex_formatting, |repl| {
    repl.sendln("uint256 x = 42").await;
    // Full word hex should be 64 chars (256 bits).
    repl.sendln("x").await;
    repl.expect("0x000000000000000000000000000000000000000000000000000000000000002a").await;
});

// Issue #9377: Test that full words are printed correctly.
repl_test!(test_full_word_hex_formatting, |repl| {
    repl.sendln(r#"keccak256(abi.encode(uint256(keccak256("AgoraStableSwapStorage.OracleStorage")) - 1)) & ~bytes32(uint256(0xff))"#).await;
    repl.expect(
        "Hex (full word): 0x0a6b316b47a0cd26c1b582ae3dcffbd175283c221c3cb3d1c614e3e47f62a700",
    )
    .await;
});
