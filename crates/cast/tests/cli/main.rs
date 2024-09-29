//! Contains various tests for checking cast commands

use alloy_chains::NamedChain;
use alloy_primitives::{b256, B256};
use anvil::{EthereumHardfork, NodeConfig};
use foundry_test_utils::{
    casttest, file,
    rpc::{
        next_http_rpc_endpoint, next_mainnet_etherscan_api_key, next_rpc_endpoint,
        next_ws_rpc_endpoint,
    },
    str,
    util::OutputExt,
};
use std::{fs, io::Write, path::Path, str::FromStr};

// tests `--help` is printed to std out
casttest!(print_help, |_prj, cmd| {
    cmd.arg("--help").assert_success().stdout_eq(str![[r#"
Perform Ethereum RPC calls from the comfort of your command line

Usage: cast[..] <COMMAND>

Commands:
...

Options:
  -h, --help     Print help
  -V, --version  Print version

Find more information in the book: http://book.getfoundry.sh/reference/cast/cast.html

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
transactionsRoot     [..]
receiptsRoot         [..]
sha3Uncles           [..]
size                 [..]
stateRoot            [..]
timestamp            [..]
withdrawalsRoot      [..]
totalDifficulty      [..]
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

// tests that we can create a new wallet with keystore
casttest!(new_wallet_keystore_with_password, |_prj, cmd| {
    cmd.args(["wallet", "new", ".", "--unsafe-password", "test"]).assert_success().stdout_eq(str![
        [r#"
Created new encrypted keystore file: [..]
[ADDRESS]

"#]
    ]);
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
Error: 
Validation failed. Address 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf did not sign this message.

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
    cmd.args(["wallet", "new-mnemonic", "--entropy", "0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c"])
        .assert_success()
        .stdout_eq(str![[r#"
Generating mnemonic from provided entropy...
Successfully generated a new mnemonic.
Phrase:
test test test test test test test test test test test junk

Accounts:
- Account 0:
Address:     0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Private key: 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80


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
        b256!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");

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

// tests that the `cast upload-signatures` command works correctly
casttest!(upload_signatures, |_prj, cmd| {
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
            .into_os_string()
            .into_string()
            .unwrap()
            .as_str(),
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
        .assert_success()
        .stdout_eq(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"totalDifficulty":"0x4dea420908b","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

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
    .assert_success()
    .stdout_eq(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"totalDifficulty":"0x4dea420908b","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

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
    .assert_success()
    .stdout_eq(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"totalDifficulty":"0x4dea420908b","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

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
    let rpc = next_http_rpc_endpoint();
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
    let rpc = next_http_rpc_endpoint();

    // <https://etherscan.io/tx/0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.args([
        "receipt",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"

blockHash               0x2cfe65be49863676b6dbc04d58176a14f39b123f1e2f4fea0383a2d82c2c50d0
blockNumber             16239315
contractAddress         
cumulativeGasUsed       10743428
effectiveGasPrice       10539984136
from                    0x199D5ED7F45F4eE35960cF22EAde2076e95B253F
gasUsed                 21000
logs                    []
logsBloom               0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                    
status                  1 (success)
transactionHash         0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e
transactionIndex        116
type                    0
blobGasPrice            
blobGasUsed             
authorizationList       
to                      0x91da5bf3F8Eb72724E6f50Ec6C3D199C6355c59c

"#]]);

    let rpc = next_http_rpc_endpoint();

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

blockHash               0x883f974b17ca7b28cb970798d1c80f4d4bb427473dc6d39b2a7fe24edc02902d
blockNumber             14839405
contractAddress         
cumulativeGasUsed       20273649
effectiveGasPrice       21491736378
from                    0x3cF412d970474804623bb4e3a42dE13F9bCa5436
gasUsed                 24952
logs                    []
logsBloom               0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                    
status                  0 (failed)
transactionHash         0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c
transactionIndex        173
type                    2
blobGasPrice            
blobGasUsed             
authorizationList       
to                      0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45
revertReason            Transaction too old, data: "0x08c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000135472616e73616374696f6e20746f6f206f6c6400000000000000000000000000"

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
    let rpc = next_http_rpc_endpoint();
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
    let rpc = next_http_rpc_endpoint();
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
    let rpc = next_http_rpc_endpoint();
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
    let rpc = next_http_rpc_endpoint();
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
Error: 
Must specify a recipient address or contract code to deploy

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
Error: 
The specified sender via CLI/env vars does not match the sender configured via
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
Error: 
Must specify a recipient address or contract code to deploy

"#]]);
});

casttest!(storage, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();
    cmd.args(["storage", "vitalik.eth", "1", "--rpc-url", &rpc]).assert_success().stdout_eq(str![
        [r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]
    ]);

    let rpc = next_http_rpc_endpoint();
    cmd.cast_fuse()
        .args(["storage", "vitalik.eth", "0x01", "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_rpc_endpoint();
    let usdt = "0xdac17f958d2ee523a2206206994597c13d831ec7";
    let decimals_slot = "0x09";
    cmd.cast_fuse()
        .args(["storage", usdt, decimals_slot, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000006

"#]]);

    let rpc = next_http_rpc_endpoint();
    let total_supply_slot = "0x01";
    let block_before = "4634747";
    let block_after = "4634748";
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_before])
        .assert_success()
        .stdout_eq(str![[r#"
0x0000000000000000000000000000000000000000000000000000000000000000

"#]]);

    let rpc = next_http_rpc_endpoint();
    cmd.cast_fuse()
        .args(["storage", usdt, total_supply_slot, "--rpc-url", &rpc, "--block", block_after])
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000174876e800

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/6319>
casttest!(storage_layout, |_prj, cmd| {
    cmd.args([
            "storage",
            "--rpc-url",
            next_rpc_endpoint(NamedChain::Optimism).as_str(),
            "--block",
            "110000000",
            "--etherscan-api-key",
            "JQNGFHINKS1W7Y5FRXU4SPBYF43J3NYK46",
            "0xB67c152E69217b5aCB85A2e19dF13423351b0E27",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
| Name                          | Type                                                            | Slot | Offset | Bytes | Value                                             | Hex Value                                                          | Contract                                           |
|-------------------------------|-----------------------------------------------------------------|------|--------|-------|---------------------------------------------------|--------------------------------------------------------------------|----------------------------------------------------|
| gov                           | address                                                         | 0    | 0      | 20    | 1352965747418285184211909460723571462248744342032 | 0x000000000000000000000000ecfd15165d994c2766fbe0d6bacdc2e8dedfd210 | contracts/perp/PositionManager.sol:PositionManager |
| _status                       | uint256                                                         | 1    | 0      | 32    | 1                                                 | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/perp/PositionManager.sol:PositionManager |
| admin                         | address                                                         | 2    | 0      | 20    | 1352965747418285184211909460723571462248744342032 | 0x000000000000000000000000ecfd15165d994c2766fbe0d6bacdc2e8dedfd210 | contracts/perp/PositionManager.sol:PositionManager |
| feeCalculator                 | address                                                         | 3    | 0      | 20    | 1297482016264593221714872710065075000476194625473 | 0x000000000000000000000000e3451b170806aab3e24b5cd03a331c1ccdb4d7c1 | contracts/perp/PositionManager.sol:PositionManager |
| oracle                        | address                                                         | 4    | 0      | 20    | 241116142622541106669066767052022920958068430970  | 0x0000000000000000000000002a3c0592dcb58accd346ccee2bb46e3fb744987a | contracts/perp/PositionManager.sol:PositionManager |
| referralStorage               | address                                                         | 5    | 0      | 20    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| minExecutionFee               | uint256                                                         | 6    | 0      | 32    | 20000                                             | 0x0000000000000000000000000000000000000000000000000000000000004e20 | contracts/perp/PositionManager.sol:PositionManager |
| minBlockDelayKeeper           | uint256                                                         | 7    | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| minTimeExecuteDelayPublic     | uint256                                                         | 8    | 0      | 32    | 180                                               | 0x00000000000000000000000000000000000000000000000000000000000000b4 | contracts/perp/PositionManager.sol:PositionManager |
| minTimeCancelDelayPublic      | uint256                                                         | 9    | 0      | 32    | 180                                               | 0x00000000000000000000000000000000000000000000000000000000000000b4 | contracts/perp/PositionManager.sol:PositionManager |
| maxTimeDelay                  | uint256                                                         | 10   | 0      | 32    | 1800                                              | 0x0000000000000000000000000000000000000000000000000000000000000708 | contracts/perp/PositionManager.sol:PositionManager |
| isUserExecuteEnabled          | bool                                                            | 11   | 0      | 1     | 1                                                 | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/perp/PositionManager.sol:PositionManager |
| isUserCancelEnabled           | bool                                                            | 11   | 1      | 1     | 1                                                 | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/perp/PositionManager.sol:PositionManager |
| allowPublicKeeper             | bool                                                            | 11   | 2      | 1     | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| allowUserCloseOnly            | bool                                                            | 11   | 3      | 1     | 1                                                 | 0x0000000000000000000000000000000000000000000000000000000000000001 | contracts/perp/PositionManager.sol:PositionManager |
| openPositionRequestKeys       | bytes32[]                                                       | 12   | 0      | 32    | 9287                                              | 0x0000000000000000000000000000000000000000000000000000000000002447 | contracts/perp/PositionManager.sol:PositionManager |
| closePositionRequestKeys      | bytes32[]                                                       | 13   | 0      | 32    | 5782                                              | 0x0000000000000000000000000000000000000000000000000000000000001696 | contracts/perp/PositionManager.sol:PositionManager |
| openPositionRequestKeysStart  | uint256                                                         | 14   | 0      | 32    | 9287                                              | 0x0000000000000000000000000000000000000000000000000000000000002447 | contracts/perp/PositionManager.sol:PositionManager |
| closePositionRequestKeysStart | uint256                                                         | 15   | 0      | 32    | 5782                                              | 0x0000000000000000000000000000000000000000000000000000000000001696 | contracts/perp/PositionManager.sol:PositionManager |
| isPositionKeeper              | mapping(address => bool)                                        | 16   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| openPositionsIndex            | mapping(address => uint256)                                     | 17   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| openPositionRequests          | mapping(bytes32 => struct PositionManager.OpenPositionRequest)  | 18   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| closePositionsIndex           | mapping(address => uint256)                                     | 19   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| closePositionRequests         | mapping(bytes32 => struct PositionManager.ClosePositionRequest) | 20   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| managers                      | mapping(address => bool)                                        | 21   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |
| approvedManagers              | mapping(address => mapping(address => bool))                    | 22   | 0      | 32    | 0                                                 | 0x0000000000000000000000000000000000000000000000000000000000000000 | contracts/perp/PositionManager.sol:PositionManager |

"#]]);
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
        &next_mainnet_etherscan_api_key(),
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
Error: 
ENS resolver not found for name "emo"

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
Error: 
unsupported formula ID: unknown

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

casttest!(send_eip7702, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::PragueEOF.into())))
            .await;
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
0x50b2c43fd39106bafbba0da34fc430e1f91e3c96ea2acee2bc34119f92b37750

"#]]);
});
