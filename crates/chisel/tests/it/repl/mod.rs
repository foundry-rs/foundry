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
        #[test]
        #[allow(unused_mut)]
        fn $name() {
            let mut $cmd = ChiselSession::new(stringify!($name), $flags, $init);
            $test;
            return (); // Fix "go to definition" due to `tokio::test`.
        }
    };
}

repl_test!(test_repl_help, |repl| {
    repl.sendln_raw("!h");
    repl.expect("Chisel help");
    repl.expect_prompt();
});

// Test abi encode/decode.
repl_test!(test_abi_encode_decode, |repl| {
    repl.sendln("bytes memory encoded = abi.encode(42, \"hello\")");
    repl.sendln("(uint num, string memory str) = abi.decode(encoded, (uint, string))");
    repl.sendln("num");
    repl.expect("42");
    repl.sendln("str");
    repl.expect("hello");
});

// Test 0x prefixed strings.
repl_test!(test_hex_string_interpretation, |repl| {
    repl.sendln("string memory s = \"0x1234\"");
    repl.sendln("s");
    // Should be treated as string, not hex literal.
    repl.expect("0x1234");
});

// Test cheatcodes availability.
repl_test!(test_cheatcodes_available, "", init = true, |repl| {
    repl.sendln("address alice = address(0x1)");

    repl.sendln("alice.balance");
    repl.expect("Decimal: 0");

    repl.sendln("vm.deal();");
    repl.expect("Wrong argument count for function call");

    repl.sendln("vm.deal(alice, 1 ether);");

    repl.sendln("alice.balance");
    repl.expect("Decimal: 1000000000000000000");
});

// Test empty inputs.
repl_test!(test_empty_input, |repl| {
    repl.sendln("   \n \n\n    \t \t \n \n\t\t\t\t \n \n");
});

// Issue #4130: Test type(intN).min correctness.
repl_test!(test_int_min_values, |repl| {
    repl.sendln("type(int8).min");
    repl.expect("-128");
    repl.sendln("type(int256).min");
    repl.expect("-57896044618658097711785492504343953926634992332820282019728792003956564819968");
});

// Issue #4393: Test edit command with traces.
// TODO: test `!edit`
// repl_test!(test_edit_with_traces, |repl| {
//     repl.sendln("!traces");
//     repl.sendln("uint x = 42");
//     repl.sendln("!edit");
//     // Should open editor without errors.
//     repl.expect("Running");
// });

// Test tuple support.
repl_test!(test_tuples, |repl| {
    repl.sendln("(uint a, uint b) = (1, 2)");
    repl.sendln("a");
    repl.expect("Decimal: 1");
    repl.sendln("b");
    repl.expect("Decimal: 2");
});

// Issue #4467: Test import.
repl_test!(test_import, "", init = true, |repl| {
    repl.sendln("import {Counter} from \"src/Counter.sol\"");
    repl.sendln("Counter c = new Counter()");
    // TODO: pre-existing inspection failure.
    // repl.sendln("c.number()");
    repl.sendln("uint x = c.number();\nx");
    repl.expect("Decimal: 0");
    repl.sendln("c.increment();");
    // repl.sendln("c.number()");
    repl.sendln("x = c.number();\nx");
    repl.expect("Decimal: 1");
});

// Issue #4617: Test code after assembly return.
repl_test!(test_assembly_return, |repl| {
    repl.sendln("uint x = 1;");
    repl.sendln("assembly { mstore(0x0, 0x1337) return(0x0, 0x20) }");
    repl.sendln("x = 2;");
    repl.sendln("!md");
    // Should work without errors.
    repl.expect("[0x00:0x20]: 0x0000000000000000000000000000000000000000000000000000000000001337");
});

// Issue #4652: Test commands with trailing whitespace.
repl_test!(test_trailing_whitespace, |repl| {
    repl.sendln("uint x = 42   ");
    repl.sendln("x");
    repl.expect("Decimal: 42");
});

// Issue #4652: Test that solc flags are respected.
repl_test!(test_solc_flags, "--use 0.8.23", |repl| {
    repl.sendln("pragma solidity 0.8.24;");
    repl.expect("invalid solc version");
});

// Issue #4915: `chisel eval`
repl_test!(test_eval_subcommand, "eval type(uint8).max", |repl| {
    repl.expect("Decimal: 255");
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
    repl.sendln_raw(input.trim());
    repl.expect_prompts(3);
    repl.sendln("value");
    repl.expect("Decimal: 12345");
    repl.sendln("!md");
    repl.expect("[0x00:0x20]");
});

// Issue #5051, #8978: Test EVM version normalization.
repl_test!(test_evm_version_normalization, "--use 0.7.6 --evm-version london", |repl| {
    repl.sendln("uint x;\nx");
    repl.expect("Decimal: 0");
});

// Issue #5481: Test function return values are displayed.
repl_test!(test_function_return_display, |repl| {
    repl.sendln("function add(uint a, uint b) public pure returns (uint) { return a + b; }");
    repl.sendln("add(2, 3)");
    repl.expect("Decimal: 5");
});

// Issue #5737: Test bytesN return types.
repl_test!(test_bytes_length_type, |repl| {
    repl.sendln("bytes10 b = bytes10(0)");
    repl.sendln("b.length");
    repl.expect("Decimal: 10");
});

// Issue #5737: Test bytesN indexing return type.
repl_test!(test_bytes_index_type, |repl| {
    repl.sendln("bytes32 b = bytes32(uint256(0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20))");
    repl.sendln("b[3]");
    repl.expect("Data: 0x0400000000000000000000000000000000000000000000000000000000000000");
});

// Issue #6618: Test fetching interface with structs.
repl_test!(test_fetch_interface_with_structs, |repl| {
    repl.sendln_raw("!fe 0x5ff137d4b0fdcd49dca30c7cf57e578a026d2789 IEntryPoint");
    repl.expect(
        "Added 0x5FF137D4b0FDCD49DcA30c7CF57E578a026d2789's interface to source as `IEntryPoint`",
    );
    repl.expect_prompt();
    repl.sendln("uint256 x = 1;\nx");
    repl.expect("Decimal: 1");
});

// Issue #7035: Test that hex strings aren't checksummed as addresses.
repl_test!(test_hex_string_no_checksum, |repl| {
    repl.sendln("function test(string memory s) public pure returns (string memory) { return s; }");
    repl.sendln("test(\"0xe5f3af50fe5d0bf402a3c6f55ccc47d4307922d4\")");
    // Should return the exact string, not checksummed.
    repl.expect("0xe5f3af50fe5d0bf402a3c6f55ccc47d4307922d4");
});

// Issue #7050: Test enum min/max operations.
repl_test!(test_enum_min_max, |repl| {
    repl.sendln("enum Color { Red, Green, Blue }");
    repl.sendln("type(Color).min");
    repl.expect("Decimal: 0");
    repl.sendln("type(Color).max");
    repl.expect("Decimal: 2");
});

// Issue #9377: Test correct hex formatting for uint256.
repl_test!(test_uint256_hex_formatting, |repl| {
    repl.sendln("uint256 x = 42");
    // Full word hex should be 64 chars (256 bits).
    repl.sendln("x");
    repl.expect("0x000000000000000000000000000000000000000000000000000000000000002a");
});

// Issue #9377: Test that full words are printed correctly.
repl_test!(test_full_word_hex_formatting, |repl| {
    repl.sendln(r#"keccak256(abi.encode(uint256(keccak256("AgoraStableSwapStorage.OracleStorage")) - 1)) & ~bytes32(uint256(0xff))"#);
    repl.expect(
        "Hex (full word): 0x0a6b316b47a0cd26c1b582ae3dcffbd175283c221c3cb3d1c614e3e47f62a700",
    );
});

// Test that uint is printed properly with any size.
repl_test!(test_uint_formatting, |repl| {
    for size in (8..=256).step_by(8) {
        repl.sendln(&format!("type(uint{size}).max"));
        repl.expect(&format!("Hex: 0x{}", "f".repeat(size / 4)));

        repl.sendln(&format!("uint{size}(2)"));
        repl.expect("Hex: 0x2");
    }
});

// Test that int is printed properly with any size.
repl_test!(test_int_formatting, |repl| {
    for size in (8..=256).step_by(8) {
        let size_minus_1: usize = size / 4 - 1;
        repl.sendln(&format!("type(int{size}).max"));
        repl.expect(&format!("Hex: 0x7{}", "f".repeat(size_minus_1)));

        repl.sendln(&format!("int{size}(2)"));
        repl.expect("Hex: 0x2");

        repl.sendln(&format!("type(int{size}).min"));
        repl.expect(&format!("Hex: 0x8{}", "0".repeat(size_minus_1)));

        repl.sendln(&format!("int{size}(-2)"));
        repl.expect(&format!("Hex: 0x{}e", "f".repeat(size_minus_1)));
    }
});
