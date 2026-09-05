//! CLI tests for wallet signing commands.

use super::*;

// tests that `cast wallet sign --json` wraps signature in envelope
casttest!(wallet_sign_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--json",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "test",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":"0xfe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c","errors":[],"warnings":[]}

"#]]);
});

// tests that `cast wallet sign -v --json` wraps verbose output in envelope
casttest!(wallet_sign_json_verbose, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--json",
        "-v",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "test",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":{"message":"test","address":"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf","signature":"fe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c"},"errors":[],"warnings":[]}

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
Error: Validation failed. Address 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf did not sign this message.

"#]]);
});

// tests that `cast wallet sign --json` outputs JSON on stdout regardless of verbosity
casttest!(wallet_sign_message_json, |_prj, cmd| {
    cmd.args([
        "wallet",
        "sign",
        "--json",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "test",
    ])
.assert_success()
.stdout_eq(str![[r#"
{"schema_version":1,"success":true,"data":"0xfe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c","errors":[],"warnings":[]}

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

casttest!(wallet_sign_auth_zero_chain_requires_confirmation, |_prj, cmd| {
    use alloy_rlp::Decodable;

    let delegate = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";
    let private_key = "0x0000000000000000000000000000000000000000000000000000000000000001";

    cmd.args(["wallet", "sign-auth", "--nonce", "100", "--chain", "0", delegate])
        .stdin("n\n")
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Warning: Chain ID 0 creates an EIP-7702 authorization that is valid on every chain.

Continue anyway? [y/N] Aborted.

"#]]);

    let confirmed = cmd
        .cast_fuse()
        .args([
            "wallet",
            "sign-auth",
            "--private-key",
            private_key,
            "--nonce",
            "100",
            "--chain",
            "0",
            delegate,
        ])
        .stdin("y\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Chain ID 0 creates an EIP-7702 authorization that is valid on every chain.

Continue anyway? [y/N] "#]])
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    let bytes = hex::decode(confirmed.strip_prefix("0x").unwrap()).unwrap();
    let auth = alloy_eips::eip7702::SignedAuthorization::decode(&mut bytes.as_slice()).unwrap();
    assert_eq!(*auth.chain_id(), U256::ZERO);
    assert_eq!(auth.nonce(), 100);

    cmd.cast_fuse()
        .args([
            "wallet",
            "sign-auth",
            "--private-key",
            private_key,
            "--nonce",
            "100",
            "--chain",
            "0",
            "--force",
            delegate,
        ])
        .assert_success()
        .stdout_eq(format!("{confirmed}\n"))
        .stderr_eq(str![""]);
});

casttest!(wallet_sign_auth_rpc_zero_chain_requires_confirmation, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(0u64))).await;

    cmd.args([
        "wallet",
        "sign-auth",
        "--rpc-url",
        &handle.http_endpoint(),
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Warning: Chain ID 0 creates an EIP-7702 authorization that is valid on every chain.

Continue anyway? [y/N] Aborted.

"#]]);
});

// tests that `cast wallet sign-auth --self-broadcast` uses nonce + 1
casttest!(wallet_sign_auth_self_broadcast, async |_prj, cmd| {
    use alloy_rlp::Decodable;
    use alloy_signer_local::PrivateKeySigner;

    let (_, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();

    let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let signer: PrivateKeySigner = private_key.parse().unwrap();
    let signer_address = signer.address();
    let delegate_address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    // Get the current nonce from the RPC
    let provider = ProviderBuilder::new().connect_http(endpoint.parse().unwrap());
    let current_nonce = provider.get_transaction_count(signer_address).await.unwrap();

    // First, get the auth without --self-broadcast (should use current nonce)
    let output_normal = cmd
        .args([
            "wallet",
            "sign-auth",
            "--private-key",
            private_key,
            "--rpc-url",
            &endpoint,
            &delegate_address.to_string(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Then, get the auth with --self-broadcast (should use current nonce + 1)
    let output_self_broadcast = cmd
        .cast_fuse()
        .args([
            "wallet",
            "sign-auth",
            "--private-key",
            private_key,
            "--rpc-url",
            &endpoint,
            "--self-broadcast",
            &delegate_address.to_string(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // The outputs should be different due to different nonces
    assert_ne!(
        output_normal, output_self_broadcast,
        "self-broadcast should produce different signature due to nonce + 1"
    );

    // Decode the RLP to verify the nonces
    let normal_bytes = hex::decode(output_normal.strip_prefix("0x").unwrap()).unwrap();
    let self_broadcast_bytes =
        hex::decode(output_self_broadcast.strip_prefix("0x").unwrap()).unwrap();

    let normal_auth =
        alloy_eips::eip7702::SignedAuthorization::decode(&mut normal_bytes.as_slice()).unwrap();
    let self_broadcast_auth =
        alloy_eips::eip7702::SignedAuthorization::decode(&mut self_broadcast_bytes.as_slice())
            .unwrap();

    assert_eq!(normal_auth.nonce(), current_nonce, "normal auth should have current nonce");
    assert_eq!(
        self_broadcast_auth.nonce(),
        current_nonce + 1,
        "self-broadcast auth should have current nonce + 1"
    );
});
