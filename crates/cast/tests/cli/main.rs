//! Contains various tests for checking cast commands

use alloy_chains::NamedChain;
use alloy_hardforks::EthereumHardfork;
use alloy_network::{TransactionBuilder, TransactionResponse};
use alloy_primitives::{B256, Bytes, address, b256, hex};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::{BlockNumberOrTag, Index, TransactionRequest};
use anvil::NodeConfig;
use foundry_test_utils::{
    rpc::{
        next_etherscan_api_key, next_http_archive_rpc_url, next_http_rpc_endpoint,
        next_rpc_endpoint, next_ws_rpc_endpoint,
    },
    snapbox::IntoData as _,
    str,
    util::OutputExt,
};
use std::{fs, io::Write, path::Path, str::FromStr};

#[macro_use]
extern crate foundry_test_utils;

mod selectors;

casttest!(print_short_version, |_prj, cmd| {
    cmd.arg("-V").assert_success().stdout_eq(str![[r#"
cast [..]-[..] ([..] [..])

"#]]);
});

casttest!(print_long_version, |_prj, cmd| {
    cmd.arg("--version").assert_success().stdout_eq(str![[r#"
cast Version: [..]
Commit SHA: [..]
Build Timestamp: [..]
Build Profile: [..]

"#]]);
});

// tests `--help` is printed to std out
casttest!(print_help, |_prj, cmd| {
    cmd.arg("--help").assert_success().stdout_eq(str![[r#"
A Swiss Army knife for interacting with Ethereum applications from the command line

Usage: cast[..] <COMMAND>

Commands:
...

Options:
  -h, --help
          Print help (see a summary with '-h')

  -j, --threads <THREADS>
          Number of threads to use. Specifying 0 defaults to the number of logical cores
          
          [aliases: --jobs]

  -V, --version
          Print version

Display options:
      --color <COLOR>
          The color of the log messages

          Possible values:
          - auto:   Intelligently guess whether to use color output (default)
          - always: Force color output
          - never:  Force disable color output

      --json
          Format log messages as JSON

  -q, --quiet
          Do not print log messages

  -v, --verbosity...
          Verbosity level of the log messages.
          
          Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
          
          Depending on the context the verbosity levels have different meanings.
          
          For example, the verbosity levels of the EVM are:
          - 2 (-vv): Print logs for all tests.
          - 3 (-vvv): Print execution traces for failing tests.
          - 4 (-vvvv): Print execution traces for all tests, and setup traces for failing tests.
          - 5 (-vvvvv): Print execution and setup traces for all tests, including storage changes.

Find more information in the book: https://getfoundry.sh/cast/overview

"#]]);
});

// tests that the `cast block` command works correctly
casttest!(latest_block, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    cmd.args(["block", "latest", "--rpc-url", eth_rpc_url.as_str()]);
    cmd.assert_success().stdout_eq(str![[r#"


baseFeePerGas        [..]
difficulty           [..]
extraData            [..]
gasLimit             [..]
gasUsed              [..]
hash                 [..]
logsBloom            [..]
miner                [..]
mixHash              [..]
nonce                [..]
number               [..]
parentHash           [..]
parentBeaconRoot     [..]
transactionsRoot     [..]
receiptsRoot         [..]
sha3Uncles           [..]
size                 [..]
stateRoot            [..]
timestamp            [..]
withdrawalsRoot      [..]
totalDifficulty      [..]
blobGasUsed          [..]
excessBlobGas        [..]
requestsHash         [..]
transactions:        [
...
]

"#]]);

    // <https://etherscan.io/block/15007840>
    cmd.cast_fuse().args(["block", "15007840", "-f", "hash", "--rpc-url", eth_rpc_url.as_str()]);
    cmd.assert_success().stdout_eq(str![[r#"
0x950091817a57e22b6c1f3b951a15f52d41ac89b299cc8f9c89bb6d185f80c415

"#]]);
});

casttest!(block_raw, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    let output = cmd
        .args(["block", "22934900", "--rpc-url", eth_rpc_url.as_str(), "--raw"])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Hash the output with keccak256
    let hash = alloy_primitives::keccak256(hex::decode(output).unwrap());

    // Verify the Mainnet's block #22934900 header hash equals the expected value
    // obtained with go-ethereum's `block.Header().Hash()` method
    assert_eq!(
        hash.to_string(),
        "0x49fd7f3b9ba5d67fa60197027f09454d4cac945e8f271edcc84c3fd5872446d3"
    );
});

// tests that the `cast find-block` command works correctly
casttest!(finds_block, |_prj, cmd| {
    // Construct args
    let timestamp = "1647843609".to_string();
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    // <https://etherscan.io/block/14428082>
    cmd.args(["find-block", "--rpc-url", eth_rpc_url.as_str(), &timestamp])
        .assert_success()
        .stdout_eq(str![[r#"
14428082

"#]]);
});

// tests that we can create a new wallet
casttest!(new_wallet, |_prj, cmd| {
    cmd.args(["wallet", "new"]).assert_success().stdout_eq(str![[r#"
Successfully created new keypair.
[ADDRESS]
[PRIVATE_KEY]

"#]]);
});

// tests that we can create a new wallet (verbose variant)
casttest!(new_wallet_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", "-v"]).assert_success().stdout_eq(str![[r#"
Successfully created new keypair.
[ADDRESS]
[PUBLIC_KEY]
[PRIVATE_KEY]

"#]]);
});

// tests that we can create a new wallet with json output
casttest!(new_wallet_json, |_prj, cmd| {
    cmd.args(["wallet", "new", "--json"]).assert_success().stdout_eq(
        str![[r#"
[
  {
    "address": "{...}",
    "private_key": "{...}"
  }
]

"#]]
        .is_json(),
    );
});

// tests that we can create a new wallet with json output (verbose variant)
casttest!(new_wallet_json_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", "--json", "-v"]).assert_success().stdout_eq(
        str![[r#"
[
  {
    "address": "{...}",
    "public_key": "{...}",
    "private_key": "{...}"
  }
]

"#]]
        .is_json(),
    );
});

// tests that we can create a new wallet with keystore
casttest!(new_wallet_keystore_with_password, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test"])
        .assert_success()
        .stdout_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);
});

// tests that we can create a new wallet with keystore (verbose variant)
casttest!(new_wallet_keystore_with_password_verbose, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "test-account", "--unsafe-password", "test", "-v"])
        .assert_success()
        .stdout_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]
[PUBLIC_KEY]

"#]]);
});

// tests that we can get the address of a keystore file
casttest!(wallet_address_keystore_with_password_file, |_prj, cmd| {
    let keystore_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystore");

    cmd.args([
        "wallet",
        "address",
        "--keystore",
        keystore_dir
            .join("UTC--2022-12-20T10-30-43.591916000Z--ec554aeafe75601aaab43bd4621a22284db566c2")
            .to_str()
            .unwrap(),
        "--password-file",
        keystore_dir.join("password-ec554").to_str().unwrap(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xeC554aeAFE75601AaAb43Bd4621A22284dB566C2

"#]]);
});

// tests that `cast wallet remove` can successfully remove a keystore file and validates password
casttest!(wallet_remove_keystore_with_unsafe_password, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);

    assert!(keystore_file.exists());
    // Remove the wallet
    cmd.cast_fuse()
        .args([
            "wallet",
            "remove",
            "--name",
            account_name,
            "--dir",
            keystore_path.to_str().unwrap(),
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was removed successfully.

"#]]);

    assert!(!keystore_file.exists());
});

// tests that `cast wallet sign message` outputs the expected signature
casttest!(wallet_sign_message_utf8_data, |_prj, cmd| {
    let pk = "0x0000000000000000000000000000000000000000000000000000000000000001";
    let address = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";
    let msg = "test";
    let expected = "0xfe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c";

    cmd.args(["wallet", "sign", "--private-key", pk, msg]).assert_success().stdout_eq(str![[r#"
0xfe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c

"#]]);

    // Success.
    cmd.cast_fuse()
        .args(["wallet", "verify", "-a", address, msg, expected])
        .assert_success()
        .stdout_eq(str![[r#"
Validation succeeded. Address 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf signed this message.

"#]]);

    // Fail.
    cmd.cast_fuse()
        .args(["wallet", "verify", "-a", address, "other msg", expected])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Validation failed. Address 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf did not sign this message.

"#]]);
});

// tests that `cast wallet sign message` outputs the expected signature, given a 0x-prefixed data
casttest!(wallet_sign_message_hex_data, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    ]).assert_success().stdout_eq(str![[r#"
0x23a42ca5616ee730ff3735890c32fc7b9491a9f633faca9434797f2c845f5abf4d9ba23bd7edb8577acebaa3644dc5a4995296db420522bb40060f1693c33c9b1c

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10613>
// tests that `cast wallet sign` and `cast wallet verify` work with the same message as input
casttest!(wallet_sign_and_verify_message_hex_data, |_prj, cmd| {
    //     message="$1"
    //     mnemonic="test test test test test test test test test test test junk"
    //     key=$(cast wallet private-key --mnemonic "$mnemonic")
    //     address=$(cast wallet address --mnemonic "$mnemonic")
    //     signature=$(cast wallet sign --private-key "$key" "$message")
    //     cast wallet verify --address "$address" "$message" "$signature"
    let mnemonic = "test test test test test test test test test test test junk";
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    cmd.args(["wallet", "private-key", "--mnemonic", mnemonic]).assert_success().stdout_eq(str![[
        r#"
0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

"#
    ]]);
    cmd.cast_fuse().args(["wallet", "address", "--mnemonic", mnemonic]).assert_success().stdout_eq(
        str![[r#"
0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

"#]],
    );

    let msg_hex = "0x0000000000000000000000000000000000000000000000000000000000000001";
    let signature_hex = "0xed769da87f78d0166b30aebf2767ceed5a3867da21b2fba8c6527af256bbcebe24a1e758ec8ad1ffc29cfefa540ea7ba7966c0edf6907af82348f894ba4f40fa1b";
    cmd.cast_fuse().args([
        "wallet", "sign", "--private-key",key, msg_hex
    ]).assert_success().stdout_eq(str![[r#"
0xed769da87f78d0166b30aebf2767ceed5a3867da21b2fba8c6527af256bbcebe24a1e758ec8ad1ffc29cfefa540ea7ba7966c0edf6907af82348f894ba4f40fa1b

"#]]);

    cmd.cast_fuse()
        .args(["wallet", "verify", "--address", address, msg_hex, signature_hex])
        .assert_success()
        .stdout_eq(str![[r#"
Validation succeeded. Address 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 signed this message.

"#]]);

    let msg_raw = "0000000000000000000000000000000000000000000000000000000000000001";
    let signature_raw = "0x27a97b378477d9d004bd19cbd838d59bbb9847074ae4cc5b5975cc5566065eea76ee5b752fcdd483073e1baba548d82d9accc8603b3781bcc9abf195614cd3411c";
    cmd.cast_fuse().args([
        "wallet", "sign", "--private-key",key, msg_raw
    ]).assert_success().stdout_eq(str![[r#"
0x27a97b378477d9d004bd19cbd838d59bbb9847074ae4cc5b5975cc5566065eea76ee5b752fcdd483073e1baba548d82d9accc8603b3781bcc9abf195614cd3411c

"#]]);

    cmd.cast_fuse()
        .args(["wallet", "verify", "--address", address, msg_raw, signature_raw])
        .assert_success()
        .stdout_eq(str![[r#"
Validation succeeded. Address 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 signed this message.

"#]]);
});

// tests that `cast wallet sign typed-data` outputs the expected signature, given a JSON string
casttest!(wallet_sign_typed_data_string, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--data",
        "{\"types\": {\"EIP712Domain\": [{\"name\": \"name\",\"type\": \"string\"},{\"name\": \"version\",\"type\": \"string\"},{\"name\": \"chainId\",\"type\": \"uint256\"},{\"name\": \"verifyingContract\",\"type\": \"address\"}],\"Message\": [{\"name\": \"data\",\"type\": \"string\"}]},\"primaryType\": \"Message\",\"domain\": {\"name\": \"example.metamask.io\",\"version\": \"1\",\"chainId\": \"1\",\"verifyingContract\": \"0x0000000000000000000000000000000000000000\"},\"message\": {\"data\": \"Hello!\"}}",
    ]).assert_success().stdout_eq(str![[r#"
0x06c18bdc8163219fddc9afaf5a0550e381326474bb757c86dc32317040cf384e07a2c72ce66c1a0626b6750ca9b6c035bf6f03e7ed67ae2d1134171e9085c0b51b

"#]]);
});

// tests that `cast wallet sign typed-data` outputs the expected signature, given a JSON file
casttest!(wallet_sign_typed_data_file, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--data",
        "--from-file",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sign_typed_data.json")
            .into_os_string()
            .into_string()
            .unwrap()
            .as_str(),
    ]).assert_success().stdout_eq(str![[r#"
0x06c18bdc8163219fddc9afaf5a0550e381326474bb757c86dc32317040cf384e07a2c72ce66c1a0626b6750ca9b6c035bf6f03e7ed67ae2d1134171e9085c0b51b

"#]]);
});

// tests that `cast wallet sign typed-data` passes with type names containing colons
//  <https://github.com/foundry-rs/foundry/issues/10765>
casttest!(wallet_sign_typed_data_with_colon_succeeds, |_prj, cmd| {
    let typed_data_with_colon = r#"{
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
            ],
            "Test:Message": [
                {"name": "content", "type": "string"}
            ]
        },
        "primaryType": "Test:Message",
        "domain": {
            "name": "TestDomain",
            "version": "1",
            "chainId": 1,
            "verifyingContract": "0x0000000000000000000000000000000000000000"
        },
        "message": {
            "content": "Hello"
        }
    }"#;

    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--data",
        typed_data_with_colon,
    ]).assert_success().stdout_eq(str![[r#"
0xf91c67e845a4d468d1f876f457ffa01e65468641fc121453705242d21de39b266c278592b085814ab1e9adc938cc26b1d64bb61f80b437df077777c4283612291b

"#]]);
});

// tests that the same data without colon works correctly
// <https://github.com/foundry-rs/foundry/issues/10765>
casttest!(wallet_sign_typed_data_without_colon_works, |_prj, cmd| {
    let typed_data_without_colon = r#"{
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
            ],
            "TestMessage": [
                {"name": "content", "type": "string"}
            ]
        },
        "primaryType": "TestMessage",
        "domain": {
            "name": "TestDomain",
            "version": "1",
            "chainId": 1,
            "verifyingContract": "0x0000000000000000000000000000000000000000"
        },
        "message": {
            "content": "Hello"
        }
    }"#;

    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--data",
        typed_data_without_colon,
    ])
    .assert_success();
});

// tests that `cast wallet sign-auth message` outputs the expected signature
casttest!(wallet_sign_auth, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign-auth",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--nonce",
        "100",
        "--chain",
        "1",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"]).assert_success().stdout_eq(str![[r#"
0xf85a01947e5f4552091a69125d5dfcb7b8c2659029395bdf6401a0ad489ee0314497c3f06567f3080a46a63908edc1c7cdf2ac2d609ca911212086a065a6ba951c8748dd8634740fe498efb61770097d99ff5fdcb9a863b62ea899f6

"#]]);
});

// tests that `cast wallet list` outputs the local accounts
casttest!(wallet_list_local_accounts, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");
    fs::create_dir_all(keystore_path).unwrap();
    cmd.set_current_dir(prj.root());

    // empty results
    cmd.cast_fuse()
        .args(["wallet", "list", "--dir", "keystore"])
        .assert_success()
        .stdout_eq(str![""]);

    // create 10 wallets
    cmd.cast_fuse()
        .args(["wallet", "new", "keystore", "-n", "10", "--unsafe-password", "test"])
        .assert_success()
        .stdout_eq(str![[r#"
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]
Created new encrypted keystore file: [..]
[ADDRESS]

"#]]);

    // test list new wallet
    cmd.cast_fuse().args(["wallet", "list", "--dir", "keystore"]).assert_success().stdout_eq(str![
        [r#"
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)
[..] (Local)

"#]
    ]);
});

// tests that `cast wallet new-mnemonic --entropy` outputs the expected mnemonic
casttest!(wallet_mnemonic_from_entropy, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
Generating mnemonic from provided entropy...
Successfully generated a new mnemonic.
Phrase:
test test test test test test test test test test test junk

Accounts:
- Account 0:
Address:     0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

- Account 1:
Address:     0x70997970C51812dc3A010C7d01b50e0d17dc79C8
Private key: 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

- Account 2:
Address:     0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
Private key: 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a


"#]]
        .raw(),
    );
});

// tests that `cast wallet new-mnemonic --entropy` outputs the expected mnemonic (verbose variant)
casttest!(wallet_mnemonic_from_entropy_verbose, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "-v",
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
Generating mnemonic from provided entropy...
Successfully generated a new mnemonic.
Phrase:
test test test test test test test test test test test junk

Accounts:
- Account 0:
Address:     0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Public key:  0x8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5
Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

- Account 1:
Address:     0x70997970C51812dc3A010C7d01b50e0d17dc79C8
Public key:  0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4
Private key: 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

- Account 2:
Address:     0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
Public key:  0x9d9031e97dd78ff8c15aa86939de9b1e791066a0224e331bc962a2099a7b1f0464b8bbafe1535f2301c72c2cb3535b172da30b02686ab0393d348614f157fbdb
Private key: 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a


"#]]
        .raw(),
    );
});

// tests that `cast wallet new-mnemonic --json` outputs the expected mnemonic
casttest!(wallet_mnemonic_from_entropy_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "--json",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{
  "mnemonic": "test test test test test test test test test test test junk",
  "accounts": [
    {
      "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      "private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    },
    {
      "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
      "private_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    },
    {
      "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
      "private_key": "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    }
  ]
}

"#]]);
});

// tests that `cast wallet new-mnemonic --json` outputs the expected mnemonic (verbose variant)
casttest!(wallet_mnemonic_from_entropy_json_verbose, |_prj, cmd| {
    cmd.args([
        "wallet",
        "new-mnemonic",
        "--accounts",
        "3",
        "--entropy",
        "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c",
        "--json",
        "-v",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{
  "mnemonic": "test test test test test test test test test test test junk",
  "accounts": [
    {
      "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      "public_key": "0x8318535b54105d4a7aae60c08fc45f9687181b4fdfc625bd1a753fa7397fed753547f11ca8696646f2f3acb08e31016afac23e630c5d11f59f61fef57b0d2aa5",
      "private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    },
    {
      "address": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
      "public_key": "0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4",
      "private_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    },
    {
      "address": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
      "public_key": "0x9d9031e97dd78ff8c15aa86939de9b1e791066a0224e331bc962a2099a7b1f0464b8bbafe1535f2301c72c2cb3535b172da30b02686ab0393d348614f157fbdb",
      "private_key": "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    }
  ]
}

"#]]);
});

// tests that `cast wallet private-key` with arguments outputs the private key
casttest!(wallet_private_key_from_mnemonic_arg, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "test test test test test test test test test test test junk",
        "1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});

// tests that `cast wallet private-key` with options outputs the private key
casttest!(wallet_private_key_from_mnemonic_option, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "--mnemonic",
        "test test test test test test test test test test test junk",
        "--mnemonic-index",
        "1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});
// tests that `cast wallet public-key` correctly derives and outputs the public key
casttest!(wallet_public_key_with_private_key, |_prj, cmd| {
    cmd.args([
        "wallet",
        "public-key",
        "--raw-private-key",
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xba5734d8f7091719471e7f7ed6b9df170dc70cc661ca05e688601ad984f068b0d67351e5f06073092499336ab0839ef8a521afd334e53807205fa2f08eec74f4

"#]]);
});
// tests that `cast wallet private-key` with derivation path outputs the private key
casttest!(wallet_private_key_with_derivation_path, |_prj, cmd| {
    cmd.args([
        "wallet",
        "private-key",
        "--mnemonic",
        "test test test test test test test test test test test junk",
        "--mnemonic-derivation-path",
        "m/44'/60'/0'/0/1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

"#]]);
});

// tests that `cast wallet import` creates a keystore for a private key and that `cast wallet
// decrypt-keystore` can access it
casttest!(wallet_import_and_decrypt, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "test",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);

    assert!(keystore_file.exists());

    // decrypt the keystore file
    let decrypt_output = cmd.cast_fuse().args([
        "wallet",
        "decrypt-keystore",
        account_name,
        "-k",
        "keystore",
        "--unsafe-password",
        "test",
    ]);

    // get the PK out of the output (last word in the output)
    let decrypt_output = decrypt_output.assert_success().get_output().stdout_lossy();
    let private_key_string = decrypt_output.split_whitespace().last().unwrap();
    // check that the decrypted private key matches the imported private key
    let decrypted_private_key = B256::from_str(private_key_string).unwrap();
    // the form
    assert_eq!(decrypted_private_key, test_private_key);
});

// tests that `cast wallet change-password` can successfully change the password of a keystore file
casttest!(wallet_change_password, |prj, cmd| {
    let keystore_path = prj.root().join("keystore");

    cmd.set_current_dir(prj.root());

    let account_name = "testAccount";

    // Default Anvil private key
    let test_private_key =
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

    // import private key with initial password
    cmd.cast_fuse()
        .args([
            "wallet",
            "import",
            account_name,
            "--private-key",
            &test_private_key.to_string(),
            "-k",
            "keystore",
            "--unsafe-password",
            "old_password",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
`testAccount` keystore was saved successfully. [ADDRESS]

"#]]);

    // check that the keystore file was created
    let keystore_file = keystore_path.join(account_name);
    assert!(keystore_file.exists());

    // change the password
    cmd.cast_fuse()
        .args([
            "wallet",
            "change-password",
            account_name,
            "--keystore-dir",
            "keystore",
            "--unsafe-password",
            "old_password",
            "--unsafe-new-password",
            "new_password",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Password for keystore `testAccount` was changed successfully. [ADDRESS]

"#]]);

    // verify the old password no longer works
    cmd.cast_fuse()
        .args([
            "wallet",
            "decrypt-keystore",
            account_name,
            "-k",
            "keystore",
            "--unsafe-password",
            "old_password",
        ])
        .assert_failure();

    // verify the new password works
    let decrypt_output = cmd.cast_fuse().args([
        "wallet",
        "decrypt-keystore",
        account_name,
        "-k",
        "keystore",
        "--unsafe-password",
        "new_password",
    ]);

    // get the PK out of the output (last word in the output)
    let decrypt_output = decrypt_output.assert_success().get_output().stdout_lossy();
    let private_key_string = decrypt_output.split_whitespace().last().unwrap();

    // check that the decrypted private key matches the imported private key
    let decrypted_private_key = B256::from_str(private_key_string).unwrap();
    assert_eq!(decrypted_private_key, test_private_key);
});

// tests that `cast estimate` is working correctly.
casttest!(estimate_function_gas, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // ensure we get a positive non-error value for gas estimate
    let output: u32 = cmd
        .args([
            "estimate",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", // vitalik.eth
            "--value",
            "100",
            "deposit()",
            "--rpc-url",
            eth_rpc_url.as_str(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .parse()
        .unwrap();
    assert!(output.ge(&0));
});

// tests that `cast estimate --cost` is working correctly.
casttest!(estimate_function_cost, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // ensure we get a positive non-error value for cost estimate
    let output: f64 = cmd
        .args([
            "estimate",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", // vitalik.eth
            "--value",
            "100",
            "deposit()",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--cost",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .parse()
        .unwrap();
    assert!(output > 0f64);
});

// tests that `cast estimate --create` is working correctly.
casttest!(estimate_contract_deploy_gas, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    // sample contract code bytecode. Wouldn't run but is valid bytecode that the estimate method
    // accepts and could be deployed.
    let output = cmd
        .args([
            "estimate",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--create",
            "0000",
            "ERC20(uint256,string,string)",
            "100",
            "Test",
            "TST",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // ensure we get a positive non-error value for gas estimate
    let output: u32 = output.trim().parse().unwrap();
    assert!(output > 0);
});

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
});

// test that `cast impl` works correctly for both the implementation slot and the beacon slot
casttest!(impl_slot, |_prj, cmd| {
    let eth_rpc_url = next_http_archive_rpc_url();

    // Call `cast impl` for the implementation slot (AAVE Proxy)
    cmd.args([
        "impl",
        "0x4965f6FA20fE9728deCf5165016fc338a5a85aBF",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--block",
        "21422087",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xb61306c8eb34a2104d9eb8d84f1bb1001067fa4b

"#]]);
});

casttest!(impl_slot_beacon, |_prj, cmd| {
    let eth_rpc_url = next_http_archive_rpc_url();

    // Call `cast impl` for the beacon slot
    cmd.args([
        "impl",
        "0xc63d9f0040d35f328274312fc8771a986fc4ba86",
        "--beacon",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--block",
        "21422087",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0xa748ae65ba11606492a9c57effa0d4b7be551ec2

"#]]);
});

// test for cast_rpc without arguments
casttest!(rpc_no_args, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_chainId`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_chainId"]).assert_success().stdout_eq(
        str![[r#"
"0x1"

"#]],
    );
});

// test for cast_rpc without arguments using websocket
casttest!(ws_rpc_no_args, |_prj, cmd| {
    let eth_rpc_url = next_ws_rpc_endpoint();

    // Call `cast rpc eth_chainId`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_chainId"]).assert_success().stdout_eq(
        str![[r#"
"0x1"

"#]],
    );
});

// test for cast_rpc with arguments
casttest!(rpc_with_args, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber 0x123 false`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "0x123", "false"])
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// test for cast_rpc with arguments
casttest!(rpc_format_as_json, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber 0x123 false`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "0x123", "false", "--json"])
    .assert_json_stdout(str![[r#"
{
  "hash": "0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524",
  "parentHash": "0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017",
  "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
  "miner": "0xbb7b8287f3f0a933474a79eae42cbca977791171",
  "stateRoot": "0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4",
  "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
  "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
  "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
  "difficulty": "0x494433b31",
  "number": "0x123",
  "gasLimit": "0x1388",
  "gasUsed": "0x0",
  "timestamp": "0x55ba4564",
  "extraData": "0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32",
  "mixHash": "0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0",
  "nonce": "0x29d6547c196e00e0",
  "size": "0x220",
  "uncles": [],
  "transactions": []
}

"#]]);
});

// test for cast_rpc with raw params
casttest!(rpc_raw_params, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber --raw '["0x123", false]'`
    cmd.args([
        "rpc",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "eth_getBlockByNumber",
        "--raw",
        r#"["0x123", false]"#,
    ])
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// test for cast_rpc with direct params
casttest!(rpc_raw_params_stdin, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `echo "\n[\n\"0x123\",\nfalse\n]\n" | cast rpc  eth_getBlockByNumber --raw
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "--raw"]).stdin(
        |mut stdin| {
            stdin.write_all(b"\n[\n\"0x123\",\nfalse\n]\n").unwrap();
        },
    )
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// checks `cast calldata` can handle arrays
casttest!(calldata_array, |_prj, cmd| {
    cmd.args(["calldata", "propose(string[])", "[\"\"]"]).assert_success().stdout_eq(str![[r#"
0xcde2baba0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/2705>
casttest!(run_succeeds, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "run",
        "-v",
        "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
        "--quick",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);
});

// tests that `cast --to-base` commands are working correctly.
casttest!(to_base, |_prj, cmd| {
    let values = [
        "1",
        "100",
        "100000",
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "-1",
        "-100",
        "-100000",
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

// tests that revert reason is only present if transaction has reverted.
casttest!(receipt_revert_reason, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();

    // <https://etherscan.io/tx/0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.args([
        "receipt",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"

blockHash            0x2cfe65be49863676b6dbc04d58176a14f39b123f1e2f4fea0383a2d82c2c50d0
blockNumber          16239315
contractAddress      
cumulativeGasUsed    10743428
effectiveGasPrice    10539984136
from                 0x199D5ED7F45F4eE35960cF22EAde2076e95B253F
gasUsed              21000
logs                 []
logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                 
status               1 (success)
transactionHash      0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e
transactionIndex     116
type                 0
blobGasPrice         
blobGasUsed          
to                   0x91da5bf3F8Eb72724E6f50Ec6C3D199C6355c59c

"#]]);

    let rpc = next_http_archive_rpc_url();

    // <https://etherscan.io/tx/0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c>
    cmd.cast_fuse()
        .args([
            "receipt",
            "0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c",
            "--rpc-url",
            rpc.as_str(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"

blockHash            0x883f974b17ca7b28cb970798d1c80f4d4bb427473dc6d39b2a7fe24edc02902d
blockNumber          14839405
contractAddress      
cumulativeGasUsed    20273649
effectiveGasPrice    21491736378
from                 0x3cF412d970474804623bb4e3a42dE13F9bCa5436
gasUsed              24952
logs                 []
logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                 
status               0 (failed)
transactionHash      0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c
transactionIndex     173
type                 2
blobGasPrice         
blobGasUsed          
to                   0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45
revertReason         [..]Transaction too old, data: "0x08c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000135472616e73616374696f6e20746f6f206f6c6400000000000000000000000000"

"#]]);
});

// tests that the revert reason is loaded using the correct `from` address.
casttest!(revert_reason_from, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    // https://sepolia.etherscan.io/tx/0x10ee70cf9f5ced5c515e8d53bfab5ea9f5c72cd61b25fba455c8355ee286c4e4
    cmd.args([
        "receipt",
        "0x10ee70cf9f5ced5c515e8d53bfab5ea9f5c72cd61b25fba455c8355ee286c4e4",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"

blockHash            0x32663d7730c9ea8e1de6d99854483e25fcc05bb56c91c0cc82f9f04944fbffc1
blockNumber          7823353
contractAddress      
cumulativeGasUsed    7500797
effectiveGasPrice    14296851013
from                 0x3583fF95f96b356d716881C871aF7Eb55ea34a93
gasUsed              25815
logs                 []
logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                 
status               0 (failed)
transactionHash      0x10ee70cf9f5ced5c515e8d53bfab5ea9f5c72cd61b25fba455c8355ee286c4e4
transactionIndex     96
type                 0
blobGasPrice         
blobGasUsed          
to                   0x91b5d4111a4C038153b24e31F75ccdC47123595d
revertReason         Counter is too large, data: "0x08c379a000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000014436f756e74657220697320746f6f206c61726765000000000000000000000000"

"#]]);
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

casttest!(access_list, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "access-list",
        "0xbb2b8038a1640196fbe3e38816f3e67cba72d940",
        "skim(address)",
        "0xbb2b8038a1640196fbe3e38816f3e67cba72d940",
        "--rpc-url",
        rpc.as_str(),
        "--gas-limit", // need to set this for alchemy.io to avoid "intrinsic gas too low" error
        "100000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[GAS]
access list:
- address: [..]
  keys:
...
- address: [..]
  keys:
...
- address: [..]
  keys:
...

"#]]);
});

casttest!(logs_topics, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "0x000000000000000000000000ab5801a7d398351b8be11c439e05c5b3259aec9b",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_topic_2, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "",
        "0x00000000000000000000000068a99f89e475a078645f4bac491360afe255dff1", /* Filter on the
                                                                               * `to` address */
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_sig, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_sig_2, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "",
        "0x68A99f89E475a078645f4BAC491360aFe255Dff1",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(mktx, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--value",
        "100",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ]).assert_success().stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a

"#]]);
});

// ensure recipient or code is required
casttest!(mktx_requires_to, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Must specify a recipient address or contract code to deploy

"#]]);
});

casttest!(mktx_signer_from_mismatch, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--from",
        "0x0000000000000000000000000000000000000001",
        "--chain",
        "1",
        "0x0000000000000000000000000000000000000001",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address.

"#]]);
});

casttest!(mktx_signer_from_match, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ]).assert_success().stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0cce9a61187b5d18a89ecd27ec675e3b3f10d37f165627ef89a15a7fe76395ce8a07537f5bffb358ffbef22cda84b1c92f7211723f9e09ae037e81686805d3e5505

"#]]);
});

casttest!(mktx_raw_unsigned, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e80180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_no_from_missing_chain, async |_prj, cmd| {
    // As chain is not provided, a query is made to the provider to get the chain id, before the tx
    // is built. Anvil is configured to use chain id 1 so that the produced tx will be the same
    // as in the `mktx_raw_unsigned` test.
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    cmd.args([
        "mktx",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e80180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_no_from_missing_gas_pricing, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "mktx",
        "--nonce",
        "0",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e5827a69800184773594018252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_no_from_missing_nonce, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--chain",
        "1",
        "--gas-limit",
        "21000", 
        "--gas-price",
        "20000000000",
        "0x742d35Cc6634C0532925a3b8D6Ac6F67C9c2b7FD",
        "--raw-unsigned",
    ])
    .assert_failure()
    .stderr_eq(str![[
        r#"Error: Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce

"#
    ]]);
});

casttest!(mktx_ethsign, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    cmd.args([
        "mktx",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--chain",
        "31337",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--ethsign",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"
0x02f86d827a6980843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0b8eeb1ded87b085859c510c5692bed231e3ee8b068ccf71142bbf28da0e95987a07813b676a248ae8055f28495021d78dee6695479d339a6ad9d260d9eaf20674c

"#
    ]]);
});

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

casttest!(tx_using_sender_and_nonce, |_prj, cmd| {
    let rpc = "https://reth-ethereum.ithaca.xyz/rpc";
    // <https://etherscan.io/tx/0x5bcd22734cca2385dc25b2d38a3d33a640c5961bd46d390dff184c894204b594>
    let args = vec![
        "tx",
        "--from",
        "0x4648451b5F87FF8F0F7D622bD40574bb97E25980",
        "--nonce",
        "113642",
        "--rpc-url",
        rpc,
    ];
    cmd.args(args).assert_success().stdout_eq(str![[r#"

blockHash            0x29518c1cea251b1bda5949a9b039722604ec1fb99bf9d8124cfe001c95a50bdc
blockNumber          22287055
from                 0x4648451b5F87FF8F0F7D622bD40574bb97E25980
transactionIndex     230
effectiveGasPrice    363392048

accessList           []
chainId              1
gasLimit             350000
hash                 0x5bcd22734cca2385dc25b2d38a3d33a640c5961bd46d390dff184c894204b594
input                0xa9059cbb000000000000000000000000568766d218d82333dd4dae933ddfcda5da26625000000000000000000000000000000000000000000000000000000000cc3ed109
maxFeePerGas         675979146
maxPriorityFeePerGas 1337
nonce                113642
r                    0x1e92d3e1ca69109a1743fc4b3cf9dff58630bc9f429cea3c3fe311506264e36c
s                    0x793947d4bbdce56a1a5b2b3525c46f01569414a22355f4883b5429668ab0f51a
to                   0xdAC17F958D2ee523a2206206994597C13D831ec7
type                 2
value                0
yParity              1
...
"#]]);
});

// ensure receipt or code is required
casttest!(send_requires_to, |_prj, cmd| {
    cmd.args([
        "send",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Must specify a recipient address or contract code to deploy

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/9918>
casttest!(send_7702_conflicts_with_create, |_prj, cmd| {
    cmd.args([
        "send", "--private-key", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" ,"--auth", "0xf85c827a6994f39fd6e51aad88f6f4ce6ab8827279cfffb922668001a03e1a66234e71242afcc7bc46c8950c3b2997b102db257774865f1232d2e7bf48a045e252dad189b27b2306792047745eba86bff0dd18aca813dbf3fba8c4e94576", "--create",  "0x60806040523373ffffffffffffffffffffffffffffffffffffffff163273ffffffffffffffffffffffffffffffffffffffff1614610072576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610069906100e5565b60405180910390fd5b3373ffffffffffffffffffffffffffffffffffffffff16ff5b5f82825260208201905092915050565b7f74782e6f726967696e203d3d206d73672e73656e6465720000000000000000005f82015250565b5f6100cf60178361008b565b91506100da8261009b565b602082019050919050565b5f6020820190508181035f8301526100fc816100c3565b905091905056fe"
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: EIP-7702 transactions can't be CREATE transactions and require a destination address

"#]]);
});

casttest!(storage, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args(["storage", "vitalik.eth", "1", "--rpc-url", &rpc]).assert_success().stdout_eq(str![
        [r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]
    ]);

    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args(["storage", "vitalik.eth", "0x01", "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_archive_rpc_url();
    let usdt = "0xdac17f958d2ee523a2206206994597c13d831ec7";
    let decimals_slot = "0x09";
    cmd.cast_fuse()
        .args(["storage", usdt, decimals_slot, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000006

"#]]);

    let rpc = next_http_archive_rpc_url();
    let total_supply_slot = "0x01";
    let block_before = "4634747";
    let block_after = "4634748";
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_before])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_archive_rpc_url();
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_after])
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000174876e800

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/6319>
casttest!(storage_layout_simple, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------╮
| Name    | Type    | Slot | Offset | Bytes | Value | Hex Value                                                          | Contract                                      |
+========================================================================================================================================================================+
| _owner  | address | 0    | 0      | 20    | 0     | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/Create2Deployer.sol:Create2Deployer |
|---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------|
| _paused | bool    | 0    | 20     | 1     | 0     | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/Create2Deployer.sol:Create2Deployer |
╰---------+---------+------+--------+-------+-------+--------------------------------------------------------------------+-----------------------------------------------╯


"#]]);
});

// <https://github.com/foundry-rs/foundry/pull/9332>
casttest!(storage_layout_simple_json, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2",
        "--json",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/storage_layout_simple.json": Json]);
});

// <https://github.com/foundry-rs/foundry/issues/6319>
casttest!(storage_layout_complex, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xBA12222222228d8Ba445958a75a0704d566BF2C8",
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------╮
| Name                          | Type                                                               | Slot | Offset | Bytes | Value                                            | Hex Value                                                          | Contract                        |
+======================================================================================================================================================================================================================================================================================+
| _status                       | uint256                                                            | 0    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _generalPoolsBalances         | mapping(bytes32 => struct EnumerableMap.IERC20ToBytes32Map)        | 1    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _nextNonce                    | mapping(address => uint256)                                        | 2    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _paused                       | bool                                                               | 3    | 0      | 1     | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _authorizer                   | contract IAuthorizer                                               | 3    | 1      | 20    | 549683469959765988649777481110995959958745616871 | 0x0000000000000000000000006048a8c631fb7e77eca533cf9c29784e482391e7 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _approvedRelayers             | mapping(address => mapping(address => bool))                       | 4    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _isPoolRegistered             | mapping(bytes32 => bool)                                           | 5    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _nextPoolNonce                | uint256                                                            | 6    | 0      | 32    | 1760                                             | 0x00000000000000000000000000000000000000000000000000000000000006e0 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _minimalSwapInfoPoolsBalances | mapping(bytes32 => mapping(contract IERC20 => bytes32))            | 7    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _minimalSwapInfoPoolsTokens   | mapping(bytes32 => struct EnumerableSet.AddressSet)                | 8    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _twoTokenPoolTokens           | mapping(bytes32 => struct TwoTokenPoolsBalance.TwoTokenPoolTokens) | 9    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _poolAssetManagers            | mapping(bytes32 => mapping(contract IERC20 => address))            | 10   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
|-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------|
| _internalTokenBalance         | mapping(address => mapping(contract IERC20 => uint256))            | 11   | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/vault/Vault.sol:Vault |
╰-------------------------------+--------------------------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+---------------------------------╯


"#]]);
});

casttest!(storage_layout_complex_proxy, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_rpc_endpoint(NamedChain::Sepolia).as_str(),
        "--block",
        "7857852",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xE2588A9CAb7Ea877206E35f615a39f84a64A7A3b",
        "--proxy",
        "0x29fcb43b46531bca003ddc8fcb67ffe91900c762"
    ])
    .assert_success()
    .stdout_eq(str![[r#"

╭----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------╮
| Name                       | Type                                            | Slot | Offset | Bytes | Value                                            | Hex Value                                                          | Contract                    |
+============================================================================================================================================================================================================================================================+
| singleton                  | address                                         | 0    | 0      | 20    | 239704109775411986678417050956533140837380441954 | 0x00000000000000000000000029fcb43b46531bca003ddc8fcb67ffe91900c762 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| modules                    | mapping(address => address)                     | 1    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| owners                     | mapping(address => address)                     | 2    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| ownerCount                 | uint256                                         | 3    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| threshold                  | uint256                                         | 4    | 0      | 32    | 1                                                | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| nonce                      | uint256                                         | 5    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| _deprecatedDomainSeparator | bytes32                                         | 6    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| signedMessages             | mapping(bytes32 => uint256)                     | 7    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
|----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------|
| approvedHashes             | mapping(address => mapping(bytes32 => uint256)) | 8    | 0      | 32    | 0                                                | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/SafeL2.sol:SafeL2 |
╰----------------------------+-------------------------------------------------+------+--------+-------+--------------------------------------------------+--------------------------------------------------------------------+-----------------------------╯


"#]]);
});

casttest!(storage_layout_complex_json, |_prj, cmd| {
    cmd.args([
        "storage",
        "--rpc-url",
        next_http_archive_rpc_url().as_str(),
        "--block",
        "21034138",
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
        "0xBA12222222228d8Ba445958a75a0704d566BF2C8",
        "--json",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/storage_layout_complex.json": Json]);
});

casttest!(balance, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();
    let usdt = "0xdac17f958d2ee523a2206206994597c13d831ec7";

    let usdt_result = cmd
        .args([
            "balance",
            "0x0000000000000000000000000000000000000000",
            "--erc20",
            usdt,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    let alias_result = cmd
        .cast_fuse()
        .args([
            "balance",
            "0x0000000000000000000000000000000000000000",
            "--erc721",
            usdt,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    assert_ne!(usdt_result, "0");
    assert_eq!(alias_result, usdt_result);
});

// tests that `cast interface` excludes the constructor
// <https://github.com/alloy-rs/core/issues/555>
casttest!(interface_no_constructor, |prj, cmd| {
    let interface = include_str!("../fixtures/interface.json");

    let path = prj.root().join("interface.json");
    fs::write(&path, interface).unwrap();
    // Call `cast find-block`
    cmd.arg("interface").arg(&path).assert_success().stdout_eq(str![[
        r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

library IIntegrationManager {
    type SpendAssetsHandleType is uint8;
}

interface Interface {
    function getIntegrationManager() external view returns (address integrationManager_);
    function lend(address _vaultProxy, bytes memory, bytes memory _assetData) external;
    function parseAssetsForAction(address, bytes4 _selector, bytes memory _actionData)
        external
        view
        returns (
            IIntegrationManager.SpendAssetsHandleType spendAssetsHandleType_,
            address[] memory spendAssets_,
            uint256[] memory spendAssetAmounts_,
            address[] memory incomingAssets_,
            uint256[] memory minIncomingAssetAmounts_
        );
    function redeem(address _vaultProxy, bytes memory, bytes memory _assetData) external;
}

"#
    ]]);
});

// tests that fetches WETH interface from etherscan
// <https://etherscan.io/token/0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2>
casttest!(fetch_weth_interface_from_etherscan, |_prj, cmd| {
    cmd.args([
        "interface",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.4;

interface WETH9 {
    event Approval(address indexed src, address indexed guy, uint256 wad);
    event Deposit(address indexed dst, uint256 wad);
    event Transfer(address indexed src, address indexed dst, uint256 wad);
    event Withdrawal(address indexed src, uint256 wad);

    fallback() external payable;

    function allowance(address, address) external view returns (uint256);
    function approve(address guy, uint256 wad) external returns (bool);
    function balanceOf(address) external view returns (uint256);
    function decimals() external view returns (uint8);
    function deposit() external payable;
    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
    function totalSupply() external view returns (uint256);
    function transfer(address dst, uint256 wad) external returns (bool);
    function transferFrom(address src, address dst, uint256 wad) external returns (bool);
    function withdraw(uint256 wad) external;
}

"#]]);
});

casttest!(ens_namehash, |_prj, cmd| {
    cmd.args(["namehash", "emo.eth"]).assert_success().stdout_eq(str![[r#"
0x0a21aaf2f6414aa664deb341d1114351fdb023cad07bf53b28e57c26db681910

"#]]);
});

casttest!(ens_lookup, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "lookup-address",
        "0x28679A1a632125fbBf7A68d850E50623194A709E",
        "--rpc-url",
        &eth_rpc_url,
        "--verify",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
emo.eth

"#]]);
});

casttest!(ens_resolve, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo.eth", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_success()
        .stdout_eq(str![[r#"
0x28679A1a632125fbBf7A68d850E50623194A709E

"#]]);
});

casttest!(ens_resolve_no_dot_eth, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: ENS resolver not found for name "emo"

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

casttest!(block_number, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str()])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
});

casttest!(block_number_latest, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str(), "latest"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
});

casttest!(block_number_hash, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args([
            "block-number",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "0x88e96d4537bea4d9c05d12549907b32561d3bf31f45aae734cdc119f13406cb6",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(s.trim().parse::<u64>().unwrap(), 1, "{s}")
});

// Tests that `cast --disable-block-gas-limit` commands are working correctly for BSC
// <https://github.com/foundry-rs/foundry/pull/9996>
// Equivalent transaction on Binance Smart Chain Testnet:
// <https://testnet.bscscan.com/tx/0x0db4f279fc4d47dca1e6ace180f45f50c5bf12e2b968f210c217f57031e02744>
casttest!(run_disable_block_gas_limit_check, |_prj, cmd| {
    let bsc_testnet_rpc_url = next_rpc_endpoint(NamedChain::BinanceSmartChainTestnet);

    let latest_block_json: serde_json::Value = serde_json::from_str(
        &cmd.args(["block", "--rpc-url", bsc_testnet_rpc_url.as_str(), "--json"])
            .assert_success()
            .get_output()
            .stdout_lossy(),
    )
    .expect("Failed to parse latest block");

    let latest_excessive_gas_limit_tx =
        latest_block_json["transactions"].as_array().and_then(|txs| {
            txs.iter()
                .find(|tx| tx.get("gas").and_then(|gas| gas.as_str()) == Some("0x7fffffffffffffff"))
        });

    match latest_excessive_gas_limit_tx {
        Some(tx) => {
            let tx_hash =
                tx.get("hash").and_then(|h| h.as_str()).expect("Transaction missing hash");

            // If --disable-block-gas-limit is not provided, the transaction should fail as the gas
            // limit exceeds the block gas limit.
            cmd.cast_fuse()
                .args(["run", "-v", tx_hash, "--quick", "--rpc-url", bsc_testnet_rpc_url.as_str()])
                .assert_failure()
                .stderr_eq(str![[r#"
Error: EVM error; transaction validation error: caller gas limit exceeds the block gas limit

"#]]);

            // If --disable-block-gas-limit is provided, the transaction should succeed
            // despite the gas limit exceeding the block gas limit.
            cmd.cast_fuse()
                .args([
                    "run",
                    "-v",
                    tx_hash,
                    "--quick",
                    "--rpc-url",
                    bsc_testnet_rpc_url.as_str(),
                    "--disable-block-gas-limit",
                ])
                .assert_success()
                .stdout_eq(str![[r#"
...
Transaction successfully executed.
[GAS]

"#]]);
        }
        None => {
            eprintln!(
                "Skipping test: No transaction with gas = 0x7fffffffffffffff found in the latest block."
            );
        }
    }
});

casttest!(send_eip7702, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();

    cmd.args([
        "send",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ])
    .assert_success();

    cmd.cast_fuse()
        .args(["code", "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
0xef010070997970c51812dc3a010c7d01b50e0d17dc79c8

"#]]);
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

casttest!(string_decode, |_prj, cmd| {
    cmd.args(["string-decode", "0x88c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000054753303235000000000000000000000000000000000000000000000000000000"]).assert_success().stdout_eq(str![[r#"
"GS025"

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
});

// tests that fetches a sample contract creation code
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(fetch_creation_code_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "creation-code",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x0923cad07f06b2d0e5e49e63b8b35738d4156b95",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x60566050600b82828239805160001a6073146043577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600080fdfea264697066735822122074c61e8e4eefd410ca92eec26e8112ec6e831d0a4bf35718fdd78b45d68220d064736f6c63430008070033

"#]]);
});

// tests that fetches a sample contract creation args bytes
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(fetch_creation_code_only_args_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "creation-code",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x6982508145454ce325ddbe47a25d4ec3d2311933",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--only-args",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x00000000000000000000000000000000000014bddab3e51a57cff87a50000000

"#]]);
});

// tests that displays a sample contract creation args
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(fetch_constructor_args_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "constructor-args",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x6982508145454ce325ddbe47a25d4ec3d2311933",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x00000000000000000000000000000000000014bddab3e51a57cff87a50000000 → Uint(420690000000000000000000000000000, 256)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/3473>
casttest!(test_non_mainnet_traces, |prj, cmd| {
    prj.clear();
    cmd.args([
        "run",
        "0xa003e419e2d7502269eb5eda56947b580120e00abfd5b5460d08f8af44a0c24f",
        "--rpc-url",
        next_rpc_endpoint(NamedChain::Optimism).as_str(),
        "--etherscan-api-key",
        next_etherscan_api_key().as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Executing previous transactions from the block.
Traces:
  [33841] FiatTokenProxy::fallback(0x111111125421cA6dc452d289314280a0f8842A65, 164054805 [1.64e8])
    ├─ [26673] FiatTokenV2_2::approve(0x111111125421cA6dc452d289314280a0f8842A65, 164054805 [1.64e8]) [delegatecall]
    │   ├─ emit Approval(owner: 0x9a95Af47C51562acfb2107F44d7967DF253197df, spender: 0x111111125421cA6dc452d289314280a0f8842A65, value: 164054805 [1.64e8])
    │   └─ ← [Return] true
    └─ ← [Return] true
...

"#]]);
});

// tests that displays a sample contract artifact
// <https://etherscan.io/address/0x0923cad07f06b2d0e5e49e63b8b35738d4156b95>
casttest!(fetch_artifact_from_etherscan, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "artifact",
        "--etherscan-api-key",
        &next_etherscan_api_key(),
        "0x0923cad07f06b2d0e5e49e63b8b35738d4156b95",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"{
  "abi": [],
  "bytecode": {
    "object": "0x60566050600b82828239805160001a6073146043577f4e487b7100000000000000000000000000000000000000000000000000000000600052600060045260246000fd5b30600052607381538281f3fe73000000000000000000000000000000000000000030146080604052600080fdfea264697066735822122074c61e8e4eefd410ca92eec26e8112ec6e831d0a4bf35718fdd78b45d68220d064736f6c63430008070033"
  }
}

"#]]);
});

// tests cast can decode traces when using project artifacts
forgetest_async!(decode_traces_with_project_artifacts, |prj, cmd| {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "LocalProjectContract",
        r#"
contract LocalProjectContract {
    event LocalProjectContractCreated(address owner);

    constructor() {
        emit LocalProjectContractCreated(msg.sender);
    }
}
   "#,
    )
    .unwrap();
    prj.add_script(
        "LocalProjectScript",
        r#"
import "forge-std/Script.sol";
import {LocalProjectContract} from "../src/LocalProjectContract.sol";

contract LocalProjectScript is Script {
    function run() public {
        vm.startBroadcast();
        new LocalProjectContract();
        vm.stopBroadcast();
    }
}
   "#,
    )
    .unwrap();

    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "LocalProjectScript",
    ]);

    cmd.assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Assert cast with local artifacts from outside the project.
    cmd.cast_fuse()
        .args(["run", "--la", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
Executing previous transactions from the block.
Compiling project to generate artifacts
Nothing to compile

"#]]);

    // Run cast from project dir.
    cmd.cast_fuse().set_current_dir(prj.root());

    // Assert cast without local artifacts cannot decode traces.
    cmd.cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
Executing previous transactions from the block.
Traces:
  [..] → new <unknown>@0x5FbDB2315678afecb367f032d93F642f64180aa3
    ├─  emit topic 0: 0xa7263295d3a687d750d1fd377b5df47de69d7db8decc745aaa4bbee44dc1688d
    │           data: 0x000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266
    └─ ← [Return] 62 bytes of code


Transaction successfully executed.
[GAS]

"#]]);

    // Assert cast with local artifacts can decode traces.
    cmd.cast_fuse()
        .args(["run", "--la", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
Executing previous transactions from the block.
Compiling project to generate artifacts
No files changed, compilation skipped
Traces:
  [..] → new LocalProjectContract@0x5FbDB2315678afecb367f032d93F642f64180aa3
    ├─ emit LocalProjectContractCreated(owner: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
    └─ ← [Return] 62 bytes of code


Transaction successfully executed.
[GAS]

"#]]);
});

// tests cast can decode traces when running with verbosity level > 4
forgetest_async!(show_state_changes_in_traces, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // Send tx to change counter storage value.
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "setNumber(uint256)",
            "111",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Assert cast with verbosity displays storage changes.
    cmd.cast_fuse()
        .args([
            "run",
            format!("{tx_hash}").as_str(),
            "-vvvvv",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Executing previous transactions from the block.
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::setNumber(111)
    ├─  storage changes:
    │   @ 0: 0 → 111
    └─ ← [Stop]


Transaction successfully executed.
[GAS]

"#]]);
});

// tests cast can decode external libraries traces with project cached selectors
forgetest_async!(decode_external_libraries_with_cached_selectors, |prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "ExternalLib",
        r#"
import "./CounterInExternalLib.sol";
library ExternalLib {
    function updateCounterInExternalLib(CounterInExternalLib.Info storage counterInfo, uint256 counter) public {
        counterInfo.counter = counter + 1;
    }
}
   "#,
    )
    .unwrap();
    prj.add_source(
        "CounterInExternalLib",
        r#"
import "./ExternalLib.sol";
contract CounterInExternalLib {
    struct Info {
        uint256 counter;
    }
    Info info;
    constructor() {
        ExternalLib.updateCounterInExternalLib(info, 100);
    }
}
   "#,
    )
    .unwrap();
    prj.add_script(
        "CounterInExternalLibScript",
        r#"
import "forge-std/Script.sol";
import {CounterInExternalLib} from "../src/CounterInExternalLib.sol";
contract CounterInExternalLibScript is Script {
    function run() public {
        vm.startBroadcast();
        new CounterInExternalLib();
        vm.stopBroadcast();
    }
}
   "#,
    )
    .unwrap();

    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterInExternalLibScript",
    ])
    .assert_success();

    let tx_hash = api
        .transaction_by_block_number_and_index(BlockNumberOrTag::Latest, Index::from(0))
        .await
        .unwrap()
        .unwrap()
        .tx_hash();

    // Cache project selectors.
    cmd.forge_fuse().args(["selectors", "cache"]).assert_success();

    // Assert cast with local artifacts can decode external lib signature.
    cmd.cast_fuse()
        .args(["run", format!("{tx_hash}").as_str(), "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
...
Traces:
  [..] → new <unknown>@0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
    ├─ [..] 0x6fD8bf6770F4bEe578348D24028000cE9c4D2bB9::updateCounterInExternalLib(0, 100) [delegatecall]
    │   └─ ← [Stop]
    └─ ← [Return] 62 bytes of code


Transaction successfully executed.
[GAS]

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/9476
forgetest_async!(cast_call_custom_chain_id, |_prj, cmd| {
    let chain_id = 55555u64;
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(chain_id))).await;

    let http_endpoint = handle.http_endpoint();

    cmd.cast_fuse()
        .args([
            "call",
            "5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &http_endpoint,
            "--chain",
            &chain_id.to_string(),
        ])
        .assert_success();
});

// https://github.com/foundry-rs/foundry/issues/10848
forgetest_async!(cast_call_disable_labels, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "Counter",
        r#"
contract Counter {
    uint256 public number;

    function getBalance(address target) public returns (uint256) {
        return target.balance;
    }
}
   "#,
    )
    .unwrap();

    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--labels",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:WETH",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] WETH::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);

    // Override state, `number()` with `disable_labels`.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--labels",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:WETH",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
            "--disable-labels",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/10189
forgetest_async!(cast_call_custom_override, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "Counter",
        r#"
contract Counter {
    uint256 public number;

    function getBalance(address target) public returns (uint256) {
        return target.balance;
    }
}
   "#,
    )
    .unwrap();

    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);

    // Override balance, `getBalance()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-balance",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x1111",
            "getBalance(address)(uint256)",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4369

"#]]);

    // Override balance, `getBalance()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-balance",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x1111",
            "getBalance(address)(uint256)",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [747] 0x5FbDB2315678afecb367f032d93F642f64180aa3::getBalance(0x5FbDB2315678afecb367f032d93F642f64180aa3)
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001111


Transaction successfully executed.
[GAS]

"#]]);

    // Override code with
    // contract Counter {
    //     uint256 public number1;
    // }
    // Calling `number()` should fail.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "number()(uint256)",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: server returned an error response: error code 3: execution reverted, data: "0x"

"#]]);

    // Override code with
    // contract Counter {
    //     uint256 public number1;
    // }
    // Calling `number()` should revert.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "number()(uint256)",
            "--trace"
        ])
        .assert_success()
        .stderr_eq(str![[r#"
Error: Transaction failed.

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
8738

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
            "--trace"
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number1()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000002222


Transaction successfully executed.
[GAS]

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state-diff",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
8738

"#]]);

    // Calling `number1()` with overridden state should return new value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063c223a39e14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212202a0acfb9083efed3e0e9f27177b090731d4392cf196d58e27e05088f59008d0964736f6c634300081d0033",
            "--override-state-diff",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x2222",
            "number1()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number1()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000002222


Transaction successfully executed.
[GAS]

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/9541
forgetest_async!(cast_run_impersonated_tx, |_prj, cmd| {
    let (_api, handle) = anvil::spawn(
        NodeConfig::test()
            .with_auto_impersonate(true)
            .with_eth_rpc_url(Some("https://sepolia.base.org")),
    )
    .await;

    let http_endpoint = handle.http_endpoint();

    let provider = ProviderBuilder::new().connect_http(http_endpoint.parse().unwrap());

    // send impersonated tx
    let tx = TransactionRequest::default()
        .with_from(address!("0x041563c07028Fc89106788185763Fc73028e8511"))
        .with_to(address!("0xF38aA5909D89F5d98fCeA857e708F6a6033f6CF8"))
        .with_input(
            Bytes::from_str(
                "0x60fe47b1000000000000000000000000000000000000000000000000000000000000000c",
            )
            .unwrap(),
        );

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    // run impersonated tx
    cmd.cast_fuse()
        .args(["run", &receipt.transaction_hash.to_string(), "--rpc-url", &http_endpoint])
        .assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/4776>
casttest!(fetch_src_blockscout, |_prj, cmd| {
    let url = "https://eth.blockscout.com/api";

    let weth = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

    cmd.args([
        "source",
        &weth.to_string(),
        "--chain-id",
        "1",
        "--explorer-api-url",
        url,
        "--flatten",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
contract WETH9 {
    string public name     = "Wrapped Ether";
    string public symbol   = "WETH";
    uint8  public decimals = 18;
..."#]]);
});

casttest!(fetch_src_default, |_prj, cmd| {
    let weth = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let etherscan_api_key = next_etherscan_api_key();

    cmd.args(["source", &weth.to_string(), "--flatten", "--etherscan-api-key", &etherscan_api_key])
        .assert_success()
        .stdout_eq(str![[r#"
...
contract WETH9 {
    string public name     = "Wrapped Ether";
    string public symbol   = "WETH";
    uint8  public decimals = 18;
..."#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10553>
// <https://basescan.org/tx/0x17b2de59ebd7dfd2452a3638a16737b6b65ae816c1c5571631dc0d80b63c41de>
casttest!(odyssey_can_run_p256_precompile, |_prj, cmd| {
    cmd.args([
        "run",
        "0x17b2de59ebd7dfd2452a3638a16737b6b65ae816c1c5571631dc0d80b63c41de",
        "--rpc-url",
        next_rpc_endpoint(NamedChain::Base).as_str(),
        "--quick",
        "--odyssey",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [88087] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::execute(0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    ├─ [2241] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::fallback(00) [staticcall]
    │   └─ ← [Return] 0x0000000000000000000000000b55b053230e4effb6609de652fca73fd1c29804
    ├─ [9750] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0xA12384c5E52fD646E7BC7F6B3b33A605651F566E) [staticcall]
    │   ├─ [2553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0xA12384c5E52fD646E7BC7F6B3b33A605651F566E) [delegatecall]
    │   │   └─ ← [Return] 0x000000000000000000000000000000000000000000000000000000000000f3b9
    │   └─ ← [Return] 0x000000000000000000000000000000000000000000000000000000000000f3b9
    ├─ [61992] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::00000000(00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    │   ├─ [21620] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::unwrapAndValidateSignature(0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00) [staticcall]
    │   │   ├─ [18617] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::unwrapAndValidateSignature(0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00) [delegatecall]
    │   │   │   ├─ [2369] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::pauseFlag() [staticcall]
    │   │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000
    │   │   │   ├─ [120] PRECOMPILES::sha256(0x7b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d) [staticcall]
    │   │   │   │   └─ ← [Return] 0xc13089327d3c20c0ce35f2f058c423de29977e6950e406c095e366a8fabd463f
    │   │   │   ├─ [96] PRECOMPILES::sha256(0x424242424242424242424242424242424242424242424242424242424242424201000000c13089327d3c20c0ce35f2f058c423de29977e6950e406c095e366a8fabd463f) [staticcall]
    │   │   │   │   └─ ← [Return] 0xc544bd9a4ea526dda3a008f43c21b6f0be3031b1ff71832b9876915dc91deea0
    │   │   │   ├─ [3450] 0x0000000000000000000000000000000000000100::c544bd9a(4ea526dda3a008f43c21b6f0be3031b1ff71832b9876915dc91deea0dd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f925bf54fa13f88658092efa36c51b1e3c4db31d3afb92812fb852dac7cf9614bc479bf5da7241d9c4ab1b431b57ec3369587b4c831d7a564438990da053708c3289) [staticcall]
    │   │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000001
    │   │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000011bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b
    │   │   └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000000011bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b
    │   ├─ [5994] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::checkAndIncrementNonce(23)
    │   │   ├─ [5608] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::checkAndIncrementNonce(23) [delegatecall]
    │   │   │   └─ ← [Stop]
    │   │   └─ ← [Return]
    │   ├─ [3250] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [staticcall]
    │   │   ├─ [2553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [delegatecall]
    │   │   │   └─ ← [Return] 0x000000000000000000000000000000000000000000000000000000000000968b
    │   │   └─ ← [Return] 0x000000000000000000000000000000000000000000000000000000000000968b
    │   ├─ [16411] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::pay(1551, 0x1bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b, 0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    │   │   ├─ [15711] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::pay(1551, 0x1bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b, 0x290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff, 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000) [delegatecall]
    │   │   │   ├─ [12963] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::transfer(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, 1551)
    │   │   │   │   ├─ [12263] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::transfer(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, 1551) [delegatecall]
    │   │   │   │   │   ├─ emit Transfer(param0: 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E, param1: 0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312, param2: 1551)
    │   │   │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000001
    │   │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000001
    │   │   │   └─ ← [Stop]
    │   │   └─ ← [Return]
    │   ├─ [1250] 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [staticcall]
    │   │   ├─ [553] 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779::balanceOf(0x327a25aD5Cfe5c4D4339C1A4267D4a83E8c93312) [delegatecall]
    │   │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000009c9a
    │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000009c9a
    │   ├─ [5675] 0xc2FF493F28e894742b968A7DB5D3F21F0aD80C6c::00000001(00000000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b290a4c4039f102eceba2147e1fcc46f994a46d1229faf43ffff26a058e7378ff0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f0000000000000000000000000000000000000000000000000000000000036cd000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000320000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000000000000000000000000000000000000000060f000000000000000000000000327a25ad5cfe5c4d4339c1a4267d4a83e8c93312000000000000000000000000000000000000000000000000000000000000034000000000000000000000000000000000000000000000000000000000000005a00000000000000000000000000b55b053230e4effb6609de652fca73fd1c2980400000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000221000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006cdd519280ec730727f07aa36550bde31a1d5f3097818f3425c2f083ed33a91f080fa2afac0071f6e1af9a0e9c09b851bf01e68bc8a1c1f89f686c48205762f92500000000000000000000000000000000000000000000000000000000000000244242424242424242424242424242424242424242424242424242424242424242010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000827b226368616c6c656e6765223a224b51704d51446e7841757a726f68522d483878472d5a536b625249702d76515f5f5f4a714259357a655038222c2263726f73734f726967696e223a66616c73652c226f726967696e223a2268747470732f2f6974686163612e78797a222c2274797065223a22776562617574686e2e676574227d0000000000000000000000000000000000000000000000000000000000001bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000)
    │   │   ├─ [4148] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::execute(0x0100000000007821000100000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000201bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b)
    │   │   │   ├─ [3693] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::execute(0x0100000000007821000100000000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000201bde17b8de18819c9eb86cefc3920ddb5d3d4254de276e3d6e18dd2b399f732b) [delegatecall]
    │   │   │   │   ├─ [435] 0xA12384c5E52fD646E7BC7F6B3b33A605651F566E::fallback()
    │   │   │   │   │   ├─ [55] 0x0B55b053230E4EFFb6609de652fCa73Fd1C29804::fallback() [delegatecall]
    │   │   │   │   │   │   └─ ← [Stop]
    │   │   │   │   │   └─ ← [Return]
    │   │   │   │   └─ ← [Stop]
    │   │   │   └─ ← [Return]
    │   │   └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000
    │   └─ ← [Stop]
    ├─  emit topic 0: 0x31e2fdd22f7eeca688d70008a7bee8e41aa5640885c2bc592419ae8d09d889f1
    │        topic 1: 0x000000000000000000000000a12384c5e52fd646e7bc7f6b3b33a605651f566e
    │        topic 2: 0x0000000000000000000000000000000000000000000000000000000000000017
    │           data: 0x00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000000000


Transaction successfully executed.
[GAS]

"#]]);
});

// tests cast send gas estimate execution failure message contains decoded custom error
// <https://github.com/foundry-rs/foundry/issues/9789>
forgetest_async!(cast_send_estimate_gas_error, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "SimpleStorage",
        r#"
contract SimpleStorage {
    uint256 private storedValue;
    error AddressInsufficientBalance(address account, uint256 newValue);
    function setValue(uint256 _newValue) public {
        if (_newValue > 100) {
            revert AddressInsufficientBalance(msg.sender, _newValue);
        }
        storedValue = _newValue;
    }
}
   "#,
    )
    .unwrap();
    prj.add_script(
        "SimpleStorageScript",
        r#"
import "forge-std/Script.sol";
import {SimpleStorage} from "../src/SimpleStorage.sol";
contract SimpleStorageScript is Script {
    function run() public {
        vm.startBroadcast();
        new SimpleStorage();
        vm.stopBroadcast();
    }
}
   "#,
    )
    .unwrap();

    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "SimpleStorageScript",
    ])
    .assert_success();

    // Cache project selectors.
    cmd.forge_fuse().set_current_dir(prj.root());
    cmd.forge_fuse().args(["selectors", "cache"]).assert_success();

    // Assert cast send can decode custom error on estimate gas execution failure.
    cmd.cast_fuse()
        .args([
            "send",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "setValue(uint256)",
            "1000",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure().stderr_eq(str![[r#"
Error: Failed to estimate gas: server returned an error response: error code 3: execution reverted: custom error 0x6786ad34: 000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb9226600000000000000000000000000000000000000000000000000000000000003e8, data: "0x6786ad34000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb9226600000000000000000000000000000000000000000000000000000000000003e8": AddressInsufficientBalance(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, 1000)

"#]]);
});

// <https://basescan.org/block/30558838>
casttest!(estimate_base_da, |_prj, cmd| {
    cmd.args(["da-estimate", "30558838", "-r", "https://mainnet.base.org/"])
        .assert_success()
        .stdout_eq(str![[r#"
Estimated data availability size for block 30558838 with 225 transactions: 52916546100

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10705>
casttest!(cast_call_return_array_of_tuples, |_prj, cmd| {
    cmd.args([
        "call",
        "0x198FC70Dfe05E755C81e54bd67Bff3F729344B9b",
        "facets() returns ((address,bytes4[])[])",
        "--rpc-url",
        "https://rpc.viction.xyz",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[[..]]

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/7541>
casttest!(cast_call_on_contract_with_no_code_prints_warning, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stderr_eq(str![[r#"
Warning: Contract code is empty

"#]])
    .stdout_eq(str![[r#"
0x

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10740>
casttest!(tx_raw_opstack_deposit, |_prj, cmd| {
    cmd.args([
        "tx",
        "0xf403cba612d1c01c027455c0d97427ccd5f7f99aac30017e065f81d1e30244ea",
        "--raw",
        "--rpc-url",
        "https://sepolia.base.org",
    ]).assert_success()
            .stdout_eq(str![[r#"
0x7ef90207a0cbde10ec697aff886f95d2514bab434e455620627b9bb8ba33baaaa4d537d62794d45955f4de64f1840e5686e64278da901e263031944200000000000000000000000000000000000007872386f26fc10000872386f26fc1000083096c4980b901a4d764ad0b0001000000000000000000000000000000000000000000000000000000065132000000000000000000000000fd0bf71f60660e2f608ed56e1659c450eb1131200000000000000000000000004200000000000000000000000000000000000010000000000000000000000000000000000000000000000000002386f26fc1000000000000000000000000000000000000000000000000000000000000000493e000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000a41635f5fd000000000000000000000000ca11bde05977b3631167028862be2a173976ca110000000000000000000000005703b26fe5a7be820db1bf34c901a79da1a46ba4000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000

"#]]);
});

// Test that cast send --create works correctly with constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_with_constructor_args, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Deploy a simple contract with constructor arguments
    // Contract source that takes constructor args
    prj.add_source(
        "ConstructorContract",
        r#"
contract ConstructorContract {
    uint256 public value;
    string public name;
    
    constructor(uint256 _value, string memory _name) {
        value = _value;
        name = _name;
    }
    
    function getValue() public view returns (uint256) {
        return value;
    }
}
"#,
    )
    .unwrap();

    // Compile to get bytecode
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get the compiled bytecode
    let bytecode_path = prj.root().join("out/ConstructorContract.sol/ConstructorContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Use cast send --create with constructor arguments
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor(uint256,string)",
            "42",
            "TestContract",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Extract the deployed contract address from output
    let lines: Vec<&str> = output.lines().collect();
    let mut address = None;
    for line in lines {
        if line.contains("contractAddress") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            address = Some(parts[1]);
            break;
        }
    }
    let address = address.expect("Contract address not found in output");

    // Verify the contract was deployed correctly by calling getValue()
    let value_output = cmd
        .cast_fuse()
        .args(["call", address, "getValue()", "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // The value should be 42 (0x2a in hex)
    assert!(
        value_output.contains("0x000000000000000000000000000000000000000000000000000000000000002a")
    );
});

// Test that cast estimate --create works correctly with constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
casttest!(cast_estimate_create_with_constructor_args, |prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Add a simple contract with constructor arguments
    prj.add_source(
        "EstimateContract",
        r#"
contract EstimateContract {
    uint256 public value;
    string public name;
    
    constructor(uint256 _value, string memory _name) {
        value = _value;
        name = _name;
    }
}
"#,
    )
    .unwrap();

    // Compile to get bytecode
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get the compiled bytecode
    let bytecode_path = prj.root().join("out/EstimateContract.sol/EstimateContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    let output = cmd
        .cast_fuse()
        .args([
            "estimate",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--create",
            bytecode,
            "constructor(uint256,string)",
            "100",
            "TestContract",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Parse the gas estimate
    let gas_estimate = output.trim().parse::<u64>().expect("Failed to parse gas estimate");

    // Gas estimate should be positive and reasonable for contract deployment
    assert!(gas_estimate > 50000, "Gas estimate too low for contract deployment");
    assert!(gas_estimate < 5000000, "Gas estimate unreasonably high");
});

// Test edge case: empty constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_empty_constructor, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Simple contract with no constructor arguments
    prj.add_source(
        "SimpleContract",
        r#"
contract SimpleContract {
    uint256 public constant VALUE = 42;
}
"#,
    )
    .unwrap();

    // Compile
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get bytecode
    let bytecode_path = prj.root().join("out/SimpleContract.sol/SimpleContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Deploy with empty constructor
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor()",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify deployment succeeded
    assert!(output.contains("contractAddress"));
});

// Test complex constructor arguments (multiple types)
// <https://github.com/foundry-rs/foundry/issues/10947>
forgetest_async!(cast_send_create_complex_constructor, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();

    // Contract with complex constructor
    prj.add_source(
        "ComplexContract",
        r#"
contract ComplexContract {
    address public owner;
    uint256[] public values;
    bool public active;
    
    constructor(address _owner, uint256[] memory _values, bool _active) {
        owner = _owner;
        values = _values;
        active = _active;
    }
    
    function getValuesLength() public view returns (uint256) {
        return values.length;
    }
}
"#,
    )
    .unwrap();

    // Compile
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get bytecode
    let bytecode_path = prj.root().join("out/ComplexContract.sol/ComplexContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    // Deploy with complex arguments
    let output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
            "constructor(address,uint256[],bool)",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "[1,2,3,4,5]",
            "true",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Extract deployed address
    let lines: Vec<&str> = output.lines().collect();
    let mut address = None;
    for line in lines {
        if line.contains("contractAddress") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                address = Some(parts[1]);
                break;
            }
        }
    }
    let address = address.expect("Contract address not found in output");

    // Verify the array length was set correctly
    let length_output = cmd
        .cast_fuse()
        .args(["call", address, "getValuesLength()", "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Should return 5 (0x5 in hex)
    assert!(
        length_output
            .contains("0x0000000000000000000000000000000000000000000000000000000000000005")
    );
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

// <https://github.com/foundry-rs/foundry/issues/10945>
// tests `cast code --disassemble`
casttest!(can_disassemble_contract_code, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Mainnet);
    cmd.args([
        "code",
        "--disassemble",
        "--rpc-url",
        rpc.as_str(),
        "0x1F573D6Fb3F13d689FF844B4cE37794d79a7FF1C",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
00000000: PUSH1 0x60
00000002: PUSH1 0x40
00000004: MSTORE
00000005: CALLDATASIZE
00000006: ISZERO
00000007: PUSH2 0x010f
0000000a: JUMPI
0000000b: PUSH4 0xffffffff
00000010: PUSH29 0x0100000000000000000000000000000000000000000000000000000000
0000002e: PUSH1 0x00
...
"#]]);
});

// tests that cast call properly applies state diff override
// <https://github.com/foundry-rs/foundry/issues/10930>
casttest!(cast_call_can_override_state_diff, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "--data",
        "0x",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B",
        "--override-state-diff",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B:1:1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x1337

"#]]);
    cmd.args(["--trace"]).assert_success().stdout_eq(str![[r#"
Traces:
  [7281] 0x1EA77b250eF79e917A5A637D5BB82D0980653F1B::fallback()
    ├─ [2275] 0xe537cb8a46Bd179c0C36aB7E3Fdecd759C8B80fc::fallback() [delegatecall]
    │   └─ ← [Return] 0x1337
    └─ ← [Return] 0x1337


Transaction successfully executed.
[GAS]

"#]]);
});

// Tests for negative number argument parsing
// Ensures that negative numbers in function arguments are properly parsed
// instead of being treated as command flags

// Test that cast call accepts negative numbers as function arguments
casttest!(cast_call_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    // Test with negative int parameter - should not treat -456789 as a flag
    cmd.args([
        "call",
        "0xAbCdEf1234567890aBcDeF1234567890aBcDeF12",
        "processValue(int128)",
        "-456789",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});

// Test negative numbers with multiple parameters
casttest!(cast_call_multiple_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
        "calculateDelta(int64,int32,uint16)",
        "-987654321",
        "-42",
        "65535",
    ])
    .assert_success();
});

// Test negative numbers mixed with flags
casttest!(cast_call_negative_with_flags, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--trace", // flag before
        "0x9876543210FeDcBa9876543210FeDcBa98765432",
        "updateBalance(int256)",
        "-777888",
        "--rpc-url",
        rpc.as_str(), // flag after
    ])
    .assert_success();
});

// Test that actual invalid flags are still caught
casttest!(cast_call_invalid_flag_still_caught, |_prj, cmd| {
    cmd.args([
        "call",
        "--invalid-flag", // This should be caught as invalid
        "0x5555555555555555555555555555555555555555",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
error: unexpected argument '--invalid-flag' found

  tip: to pass '--invalid-flag' as a value, use '-- --invalid-flag'

Usage: cast[..] call [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]

For more information, try '--help'.

"#]]);
});

// Test cast estimate with negative numbers
casttest!(cast_estimate_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "estimate",
        "0xBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBb",
        "rebalance(int64)",
        "-8888",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});

// Test cast mktx with negative numbers
casttest!(cast_mktx_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "mktx",
        "0x1111111111111111111111111111111111111111",
        "settleDebt(int256)",
        "-15000",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80", // anvil wallet #0
        "--rpc-url",
        rpc.as_str(),
        "--gas-limit",
        "100000",
    ])
    .assert_success();
});

// Test cast access-list with negative numbers
casttest!(cast_access_list_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "access-list",
        "0x9999999999999999999999999999999999999999",
        "adjustPosition(int128)",
        "-33333",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});
