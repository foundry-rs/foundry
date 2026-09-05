//! CLI tests for abi commands.

use super::*;

// checks `cast calldata` can handle arrays
casttest!(calldata_array, |_prj, cmd| {
    cmd.args(["calldata", "propose(string[])", "[\"\"]"]).assert_success().stdout_eq(str![[r#"
0xcde2baba0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000

"#]]);
});

casttest!(string_decode, |_prj, cmd| {
    cmd.args(["string-decode", "0x88c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000054753303235000000000000000000000000000000000000000000000000000000"]).assert_success().stdout_eq(str![[r#"
"GS025"

"#]]);
});

// Test cast abi-encode-event with indexed parameters
casttest!(abi_encode_event_indexed, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "0x1234567890123456789012345678901234567890",
        "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
        "1000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
[topic1]: 0x0000000000000000000000001234567890123456789012345678901234567890
[topic2]: 0x000000000000000000000000abcdefabcdefabcdefabcdefabcdefabcdefabcd
[data]: 0x00000000000000000000000000000000000000000000000000000000000003e8

"#]]);
});

// Test cast abi-encode-event with no indexed parameters
casttest!(abi_encode_event_no_indexed, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Approval(address owner, address spender, uint256 value)",
        "0x1234567890123456789012345678901234567890",
        "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
        "2000"
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925
[data]: 0x0000000000000000000000001234567890123456789012345678901234567890000000000000000000000000abcdefabcdefabcdefabcdefabcdefabcdefabcd00000000000000000000000000000000000000000000000000000000000007d0

"#]]);
});

// Test cast abi-encode-event with dynamic indexed parameter (string)
casttest!(abi_encode_event_dynamic_indexed, |_prj, cmd| {
    // topic1 is keccak256("hello"), matching Solidity's hashing of indexed strings.
    cmd.args(["abi-encode-event", "Log(string indexed message, uint256 data)", "hello", "42"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0xdd970dd9b5bfe707922155b058a407655cb18288b807e2216442bca8ad83d6b5
[topic1]: 0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8
[data]: 0x000000000000000000000000000000000000000000000000000000000000002a

"#]]);

    // topic1 is keccak256(0xdeadbeef): raw contents, no padding or length prefix.
    cmd.cast_fuse()
        .args(["abi-encode-event", "Raw(bytes indexed payload)", "0xdeadbeef"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0xada02cd4e7d8ed80ea02ba8f2b0c44295aecd704d4d25ffb82af584a09f59997
[topic1]: 0xd4fd4e189132273036449fc9e11198c739161b4c0116a9a2dccdfa1c492006f1

"#]]);
});

casttest!(abi_encode_event_indexed_arrays, |_prj, cmd| {
    // Array topics hash the concatenated padded elements without any length prefix:
    // topic1 is keccak256(word(1) ++ word(2)).
    cmd.args(["abi-encode-event", "Numbers(uint256[] indexed values)", "[1,2]"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x998e4b4864cb035323945d614c5aecbe49a6045c844d1e046fee480db062cc97
[topic1]: 0xe90b7bceb6e7df5418fb78d8ee546e97c83a08bbccc01a0644d599ccd2a7c2e0

"#]]);

    cmd.cast_fuse()
        .args(["abi-encode-event", "Fixed(uint256[2] indexed values)", "[7,9]"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x369112dff39fc5861d470b610421e77e5cd444efdbe9948df8de38824dffc6be
[topic1]: 0xae6299332bcd708cd60e3a8defa55de28078a50a4cf2b3de3a546253240ff9e1

"#]]);

    // Nested arrays flatten into one preimage: `[[1],[2,3]]` hashes like `[1,2,3]`.
    cmd.cast_fuse()
        .args(["abi-encode-event", "Matrix(uint256[][] indexed rows)", "[[1],[2,3]]"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x8b54144c0baafc3c0a1b743082b3378dc1f6c787fe1b6ed5067469704b5b47b9
[topic1]: 0x6e0c627900b24bd432fe7b1f713f1b0744091a646a9fe4a65a18dfed21f2949c

"#]]);

    // Strings nested in arrays are right-padded to 32 bytes each.
    cmd.cast_fuse()
        .args(["abi-encode-event", "Names(string[] indexed names)", "[alpha,beta]"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x54612034f490f8c9efbbf618b99e0dd23834387135bf603e7f77f36ab5a0dc59
[topic1]: 0xec503acd2f5b8395d778ead6068e4dff0b21beb8f55fc559e660857d57f0c112

"#]]);
});

casttest!(abi_encode_event_indexed_tuples, |_prj, cmd| {
    // Tuple topics hash member preimages without offsets: keccak256(word(7) ++ pad32("hello")).
    cmd.args(["abi-encode-event", "Pair((uint256,string) indexed pair)", "(7,hello)"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x9238dd7c0dba6500736bb8e584ccce3ba50e1d827893b0a66469369afa1b1ac8
[topic1]: 0x2919104b27111a00427dc5719d616be7497d87822aa864c58817a818e17032b4

"#]]);

    // Nested strings longer than one word are padded to a multiple of 32 bytes: the 40-byte
    // string occupies 64 bytes of the preimage.
    cmd.cast_fuse()
        .args([
            "abi-encode-event",
            "Entries((string,uint256) indexed entry)",
            "(abcdefghijklmnopqrstuvwxyz0123456789abcd,1)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0x55493f7db049561b9d0b263da2632368fcd86567dae4e8b4d8767f6789647dee
[topic1]: 0x5e20f5ecd52f29dc801132b4fba26d512378bf6c068eff0f4c6e797d69ac1e2a

"#]]);
});

casttest!(abi_encode_event_indexed_function, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Callback(function indexed callback)",
        "0x29088eeb3082c897bebd16bbafc162322cbb1bf47cfdab90",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0xc5656f42bc03a54463abf4aa177d76f9e12a2b4c0307bd71f23e713c47e8ed2d
[topic1]: 0x29088eeb3082c897bebd16bbafc162322cbb1bf47cfdab900000000000000000

"#]]);
});

casttest!(abi_encode_event_dynamic_tuple, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Details((uint256,string) details,uint256 nonce)",
        "(7,hello)",
        "9",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0xaabc576555b53190a80e9ddc865e2c1a772416783d18d8e5dc1d3f80ccbbbf3e
[data]: 0x0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000900000000000000000000000000000000000000000000000000000000000000070000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000568656c6c6f000000000000000000000000000000000000000000000000000000

"#]]);
});

casttest!(abi_encode_event_dynamic_strings, |_prj, cmd| {
    let signature = "Strings(string,string)";
    let data = "0x000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000005616c70686100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000046265746100000000000000000000000000000000000000000000000000000000";

    cmd.args(["abi-encode-event", signature, "alpha", "beta"])
        .assert_success()
        .stdout_eq(str![[r#"
[topic0]: 0xb032a34ae8575c904b71dc599fde31122ca66ed21144188304825ec5ab48652a
[data]: 0x000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000005616c70686100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000046265746100000000000000000000000000000000000000000000000000000000

"#]]);

    cmd.cast_fuse().args(["decode-event", "--sig", signature, data]).assert_success().stdout_eq(
        str![[r#"
"alpha"
"beta"

"#]],
    );
});

casttest!(abi_encode_event_argument_count_mismatch, |_prj, cmd| {
    cmd.args(["abi-encode-event", "Pair(uint256,uint256)", "1"]).assert_failure().stderr_eq(str![
        [r#"
Error: encode length mismatch: expected 2 types, got 1

"#]
    ]);

    cmd.cast_fuse()
        .args(["abi-encode-event", "Pair(uint256,uint256)", "1", "2", "3"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: encode length mismatch: expected 2 types, got 3

"#]]);
});

casttest!(abi_encode_event_anonymous, |_prj, cmd| {
    cmd.args([
        "abi-encode-event",
        "Log(uint256 indexed id,string message) anonymous",
        "7",
        "hello",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[topic0]: 0x0000000000000000000000000000000000000000000000000000000000000007
[data]: 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000568656c6c6f000000000000000000000000000000000000000000000000000000

"#]]);
});

casttest!(abi_decode_output, |_prj, cmd| {
    cmd.cast_fuse()
        .args([
            "abi-decode",
            "f()(uint256)",
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        ])
        .assert_success()
        .stdout_eq("1\n");
    cmd.cast_fuse()
        .args([
            "abi-decode",
            "balanceOf(address, uint256)(uint256)",
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        ])
        .assert_success()
        .stdout_eq("1\n");
});

casttest!(abi_and_calldata_decode_mixed_values, |_prj, cmd| {
    let data = "0000000000000000000000008dbd1b711dc621e1404633da156fcc779e1c6f3e000000000000000000000000d9f3c9cc99548bf3b44a43e0a2d07399eb918adc000000000000000000000000000000000000000000000000000000000000002a000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000";
    let sig = "safeTransferFrom(address, address, uint256, uint256, bytes)";
    let expected = "0x8DbD1b711DC621e1404633da156FcC779e1c6f3E\n0xD9f3c9CC99548bF3b44a43E0A2D07399EB918ADc\n42\n1\n0x\n";
    cmd.cast_fuse()
        .args(["abi-decode", "--input", sig, &format!("0x{data}")])
        .assert_success()
        .stdout_eq(expected);
    cmd.cast_fuse()
        .args(["calldata-decode", sig, &format!("0xf242432a{data}")])
        .assert_success()
        .stdout_eq(expected);
});

casttest!(calldata_decode_nested_json, |_prj, cmd| {
    let calldata = "0xdb5b0ed700000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000006772bf190000000000000000000000000000000000000000000000000000000000020716000000000000000000000000af9d27ffe4d51ed54ac8eec78f2785d7e11e5ab100000000000000000000000000000000000000000000000000000000000002c0000000000000000000000000000000000000000000000000000000000000000404366a6dc4b2f348a85e0066e46f0cc206fca6512e0ed7f17ca7afb88e9a4c27000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000093922dee6e380c28a50c008ab167b7800bb24c2026cd1b22f1c6fb884ceed7400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060f85e59ecad6c1a6be343a945abedb7d5b5bfad7817c4d8cc668da7d391faf700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000093dfbf04395fbec1f1aed4ad0f9d3ba880ff58a60485df5d33f8f5e0fb73188600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000aa334a426ea9e21d5f84eb2d4723ca56b92382b9260ab2b6769b7c23d437b6b512322a25cecc954127e60cf91ef056ac1da25f90b73be81c3ff1872fa48d10c7ef1ccb4087bbeedb54b1417a24abbb76f6cd57010a65bb03c7b6602b1eaf0e32c67c54168232d4edc0bfa1b815b2af2a2d0a5c109d675a4f2de684e51df9abb324ab1b19a81bac80f9ce3a45095f3df3a7cf69ef18fc08e94ac3cbc1c7effeacca68e3bfe5d81e26a659b5";
    let sig =
        "sequenceBatchesValidium((bytes32,bytes32,uint64,bytes32)[],uint64,uint64,address,bytes)";
    let expected = serde_json::json!([
        [
            [
                "0x04366a6dc4b2f348a85e0066e46f0cc206fca6512e0ed7f17ca7afb88e9a4c27",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
                0,
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            [
                "0x093922dee6e380c28a50c008ab167b7800bb24c2026cd1b22f1c6fb884ceed74",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
                0,
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            [
                "0x60f85e59ecad6c1a6be343a945abedb7d5b5bfad7817c4d8cc668da7d391faf7",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
                0,
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            [
                "0x93dfbf04395fbec1f1aed4ad0f9d3ba880ff58a60485df5d33f8f5e0fb731886",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
                0,
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ]
        ],
        1735573273,
        132886,
        "0xAF9d27ffe4d51eD54AC8eEc78f2785D7E11E5ab1",
        "0x334a426ea9e21d5f84eb2d4723ca56b92382b9260ab2b6769b7c23d437b6b512322a25cecc954127e60cf91ef056ac1da25f90b73be81c3ff1872fa48d10c7ef1ccb4087bbeedb54b1417a24abbb76f6cd57010a65bb03c7b6602b1eaf0e32c67c54168232d4edc0bfa1b815b2af2a2d0a5c109d675a4f2de684e51df9abb324ab1b19a81bac80f9ce3a45095f3df3a7cf69ef18fc08e94ac3cbc1c7effeacca68e3bfe5d81e26a659b5"
    ]);
    cmd.args(["calldata-decode", sig, calldata, "--json"]).assert_json_stdout(json!({"schema_version": 1, "success": true, "data": expected, "errors": [], "warnings": []}).to_string());
});

casttest!(abi_encode, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["abi-encode", "f(uint256)", "1"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000001\n");
});

casttest!(abi_encode_constructor, |_prj, cmd| {
    cmd.args(["abi-encode", "constructor(uint a)", "1"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000001\n");
});

casttest!(abi_encode_packed, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["abi-encode", "--packed", "(uint128[] a, uint64 b)", "[100, 300]", "200"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000012c00000000000000c8\n");
    cmd.cast_fuse()
        .args([
            "abi-encode",
            "--packed",
            "foo(address a, string b)",
            "0x8dbd1b711dc621e1404633da156fcc779e1c6f3e",
            "hello world",
        ])
        .assert_success()
        .stdout_eq("0x8dbd1b711dc621e1404633da156fcc779e1c6f3e68656c6c6f20776f726c64\n");
    cmd.cast_fuse()
        .args(["abi-encode", "--packed", "f(uint256)", "1"])
        .assert_success()
        .stdout_eq("0x0000000000000000000000000000000000000000000000000000000000000001\n");
});

casttest!(calldata_bool, |_prj, cmd| {
    cmd.cast_fuse()
        .args(["calldata", "bar(bool)", "false"])
        .assert_success()
        .stdout_eq("0x6fae94120000000000000000000000000000000000000000000000000000000000000000\n");
});
