//! CLI tests for Tempo key authorizations signed by a configurable account's owners (T14,
//! TIP-1111).

use alloy_eips::Decodable2718;
use alloy_primitives::{Address, B256, U256, hex};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use anvil::NodeConfig;
use foundry_cli::utils::parse_json;
use foundry_common::tempo::configurable_account::single_owner_account;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::{TestCommand, assert_data_eq, snapbox::IntoData, util::OutputExt};
use tempo_hardfork::TempoHardfork;
use tempo_primitives::{
    TempoTxEnvelope,
    transaction::{
        KeyAuthorization, SignatureType, SignedKeyAuthorization, configurable_account_digest,
    },
};

/// Anvil's Tempo chain ID.
const CHAIN_ID: u64 = 31337;

/// Returns whether `key` is an active access key of `account`.
fn is_provisioned(cmd: &mut TestCommand, rpc: &str, account: Address, key: Address) -> bool {
    let check = cmd
        .cast_fuse()
        .args(["keychain", "check", &account.to_string(), &key.to_string(), "--rpc-url", rpc])
        .arg("--json")
        .assert_success()
        .get_output()
        .stdout_lossy();
    parse_json::<serde_json::Value>(check.trim()).unwrap()["data"]["provisioned"] == true
}

/// A configurable account's owners authorize access keys. While the wallet's owner reaches the
/// threshold alone, `cast keychain authorize` sends `authorizeKey` from the account in a `0x05`
/// transaction, which also registers the account. Once the account is 2-of-2, it refuses, and
/// `cast key-authorization sign` signs an inline authorization with one owner through the wallet
/// and the other through `--tempo.owner-signature`, reading the configuration at version 1 from
/// the chain. The authorized key then sends a transaction for the account.
#[casttest]
async fn tempo_t14_configurable_account_authorizes_access_keys(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let fee_token = PATH_USD_ADDRESS.to_string();
    let [first, second] =
        [0x11, 0x22].map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap());
    let first_key = hex::encode_prefixed(first.to_bytes());
    let account = single_owner_account(first.address()).unwrap();
    api.anvil_deal_tip20(account, PATH_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    let [onchain_key, inline_key] =
        [0x33, 0x34].map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap());
    let send_args = [
        "--tempo.configurable-account".to_string(),
        account.to_string(),
        "--private-key".into(),
        first_key.clone(),
        "--rpc-url".into(),
        rpc.clone(),
        "--tempo.fee-token".into(),
        fee_token.clone(),
    ];

    // A selected session must not fall back to the owner key.
    cmd.cast_fuse()
        .args(["keychain", "authorize", &onchain_key.address().to_string()])
        .args(&send_args)
        .args(["--tempo.session", &B256::repeat_byte(0x55).to_string()])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: configurable account owners approve with their own keys; Tempo Accounts sessions cannot sign for them

"#]]);
    assert!(!is_provisioned(&mut cmd, &rpc, account, onchain_key.address()));

    // The account's first transaction registers it and authorizes the key.
    cmd.cast_fuse()
        .args(["keychain", "authorize", &onchain_key.address().to_string()])
        .args(&send_args)
        .assert_success();
    assert!(is_provisioned(&mut cmd, &rpc, account, onchain_key.address()));

    cmd.cast_fuse()
        .args(["wallet", "owners", "add", &account.to_string(), &second.address().to_string()])
        .args(["--threshold", "2", "--private-key", &first_key, "--rpc-url", &rpc])
        .args(["--tempo.fee-token", &fee_token])
        .assert_success();
    let mut owners = [first.address(), second.address()];
    owners.sort();
    cmd.cast_fuse()
        .args(["keychain", "authorize", &inline_key.address().to_string()])
        .args(&send_args)
        .assert_failure()
        .stderr_eq(format!(
            "Error: {account} is a Tempo configurable account that needs owner weight 2, but the \
             local owner keys {} reach weight 1. Its owners are {} (weight 1), {} (weight 1). Pass \
             the other owner keys with --tempo.owner, or have each owner sign the digest from \
             `cast mktx --raw-unsigned --tempo.digest` with `cast wallet sign --no-hash` and \
             combine the signatures with `cast wallet assemble`.\n",
            first.address(),
            owners[0],
            owners[1],
        ));

    // With `--tempo.owner`, both owners approve locally. A lane has its own nonce, which differs
    // from the account's protocol nonce, 2 after its first two transactions: the lane's first
    // transaction takes 0 and its next 1. Expiring nonce mode overrides the lane: its nonce is the
    // discriminator, 0 by default, not the lane's counter, which is then 2.
    let second_owner = format!("private-key://{}", hex::encode_prefixed(second.to_bytes()));
    let lanes = tempfile::tempdir().unwrap();
    let lanes_file = lanes.path().join("tempo.lanes.toml");
    std::fs::write(&lanes_file, "grants = 7\n").unwrap();
    let cases: [(u8, &[&str], _); 3] = [
        (
            0x35,
            &[],
            str![[r#"
{
  "success": true,
  "data": {
    "nonce": "0x0",
    "nonceKey": "0x7",
    "...": "{...}"
  },
  "...": "{...}"
}
"#]],
        ),
        (
            0x37,
            &[],
            str![[r#"
{
  "success": true,
  "data": {
    "nonce": "0x1",
    "nonceKey": "0x7",
    "...": "{...}"
  },
  "...": "{...}"
}
"#]],
        ),
        (
            0x36,
            &["--tempo.expires", "60"],
            str![[r#"
{
  "success": true,
  "data": {
    "nonce": "0x0",
    "nonceKey": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    "...": "{...}"
  },
  "...": "{...}"
}
"#]],
        ),
    ];
    for (byte, extra, sent) in cases {
        let key = PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap();
        let receipt = cmd
            .cast_fuse()
            .args(["keychain", "authorize", &key.address().to_string()])
            .args(&send_args)
            .args(["--tempo.owner", &second_owner, "--tempo.lane", "grants"])
            .args(["--tempo.lanes-file", lanes_file.to_str().unwrap(), "--json"])
            .args(extra)
            .assert_success()
            .get_output()
            .stdout_lossy();
        let receipt = parse_json::<serde_json::Value>(receipt.trim()).unwrap();
        let hash = receipt["transactionHash"].as_str().unwrap().to_owned();
        cmd.cast_fuse().args(["tx", &hash, "--rpc-url", &rpc, "--json"]).assert_json_stdout(sent);
        assert!(is_provisioned(&mut cmd, &rpc, account, key.address()));
    }

    // A selected access key is rejected even when the owners reach the threshold without it.
    let first_owner = format!("private-key://{first_key}");
    cmd.cast_fuse()
        .args(["keychain", "authorize", &inline_key.address().to_string()])
        .args(["--tempo.configurable-account", &account.to_string(), "--rpc-url", &rpc])
        .args(["--tempo.owner", &first_owner, "--tempo.owner", &second_owner])
        .args(["--tempo.access-key", &hex::encode_prefixed(onchain_key.to_bytes())])
        .args(["--tempo.root-account", &account.to_string(), "--tempo.fee-token", &fee_token])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: configurable account owners approve with their own keys, not a Tempo access key

"#]]);
    assert!(!is_provisioned(&mut cmd, &rpc, account, inline_key.address()));

    // The second owner approves the owner digest of the account-bound authorization elsewhere.
    let authorization =
        KeyAuthorization::unrestricted(CHAIN_ID, SignatureType::Secp256k1, inline_key.address())
            .with_account(account);
    let digest = configurable_account_digest(authorization.signature_hash(), account, 1);
    let approval = hex::encode_prefixed(second.sign_hash_sync(&digest).unwrap().as_bytes());
    let signed = cmd
        .cast_fuse()
        .args(["key-authorization", "sign", &inline_key.address().to_string()])
        .args(["--chain-id", &CHAIN_ID.to_string()])
        .args(["--tempo.configurable-account", &account.to_string()])
        .args(["--tempo.owner-signature", &approval, "--private-key", &first_key])
        .args(["--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let signed = signed.trim().to_owned();

    let decoded =
        alloy_rlp::decode_exact::<SignedKeyAuthorization>(hex::decode(&signed).unwrap()).unwrap();
    assert_eq!(decoded.authorization(), &authorization);
    let signature = decoded.signature().as_configurable_account().expect("a 0x05 signature");
    assert_eq!(signature.account(), account);
    assert_eq!(signature.config().version, 1);
    assert_eq!(signature.config().threshold, 2);
    assert_eq!(signature.signatures().len(), 2);
    signature.verify_approvals(authorization.signature_hash()).unwrap();

    // Offline, only the account's onchain commitment ties a version-1 configuration to it, so
    // `inspect` names no signer.
    cmd.cast_fuse()
        .args(["key-authorization", "inspect", &signed, "--account", &account.to_string()])
        .arg("--json")
        .assert_json_stdout(str![[r#"
{
  "signed": true,
  "signer": null,
  "chain_id": 31337,
  "is_admin": false,
  "...": "{...}"
}
"#]]);

    // The inline key sends a transaction for the account with the authorization attached.
    let tempo_home = tempfile::tempdir().unwrap();
    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.args(["tempo", "import-access-key", "--account", &account.to_string()])
        .args(["--access-key", &hex::encode_prefixed(inline_key.to_bytes())])
        .args(["--authorization", &signed])
        .assert_success();

    // `keychain doctor` verifies the pending authorization against the account's commitment.
    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let doctor = cmd
        .args(["keychain", "doctor", &inline_key.address().to_string()])
        .args(["--root-account", &account.to_string(), "--rpc-url", &rpc, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let doctor = parse_json::<serde_json::Value>(doctor.trim()).unwrap();
    let registration = doctor["data"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["name"] == "key_registration")
        .unwrap();
    assert_eq!(registration["status"], "pass", "{doctor}");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let receipt = cmd
        .args(["send", &fee_token, "approve(address,uint256)"])
        .args([&Address::repeat_byte(0x44).to_string(), "1", "--from", &account.to_string()])
        .args(["--tempo.fee-token", &fee_token, "--rpc-url", &rpc, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let receipt = parse_json::<serde_json::Value>(receipt.trim()).unwrap();
    assert_eq!(receipt["status"], "0x1", "{receipt}");
    assert_eq!(receipt["from"].as_str().unwrap().parse::<Address>().unwrap(), account);
    assert!(is_provisioned(&mut cmd, &rpc, account, inline_key.address()));
}

/// An EOA authorizes a configurable account as its access key (key type 3), which then signs for
/// the EOA with `--tempo.configurable-access-key`: its owner approves the keychain V2 hash in a
/// `0x05` signature that a keychain signature wraps for the EOA.
#[casttest]
async fn tempo_t14_keychain_authorizes_configurable_account_as_access_key(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let root = handle.dev_wallets().next().unwrap();
    let owner = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
    let account = single_owner_account(owner.address()).unwrap();

    cmd.cast_fuse()
        .args([
            "keychain",
            "authorize",
            &account.to_string(),
            "configurable",
            "--private-key",
            &hex::encode_prefixed(root.to_bytes()),
            "--rpc-url",
            &rpc,
        ])
        .assert_success();

    cmd.cast_fuse()
        .args(["keychain", "check", &root.address().to_string(), &account.to_string()])
        .args(["--rpc-url", &rpc, "--json"])
        .assert_json_stdout(str![[r#"
{
  "success": true,
  "data": {
    "provisioned": true,
    "signatureType": "configurableaccount",
    "is_revoked": false,
    "...": "{...}"
  },
  "...": "{...}"
}
"#]]);

    // The browser wallet cannot serialize this key type yet, so the browser flow never starts.
    cmd.cast_fuse()
        .args(["key-authorization", "sign", &account.to_string(), "--key-type", "configurable"])
        .args(["--chain-id", &CHAIN_ID.to_string(), "--browser"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: browser key authorization signing does not support the configurable key type yet

"#]]);

    // The account then signs for the EOA as its access key. Only its local owners can sign the
    // keychain signature, and an access key cannot manage the keys of the account it signs for.
    let fee_token = PATH_USD_ADDRESS.to_string();
    let call =
        [&fee_token, "approve(address,uint256)", &Address::repeat_byte(0x44).to_string(), "1"];
    let account_address = account.to_string();
    let owner_key = hex::encode_prefixed(owner.to_bytes());
    let access_key = [
        "--tempo.configurable-access-key",
        &account_address,
        "--rpc-url",
        &rpc,
        "--tempo.fee-token",
        &fee_token,
    ];
    let access_args = [&access_key[..], &["--private-key", &owner_key]].concat();
    let root_address = root.address().to_string();
    let from = ["--from", root_address.as_str()];
    let rejected = [
        (
            &["send"][..],
            call.to_vec(),
            str![[r#"
Error: --tempo.configurable-access-key needs the account it signs for: pass --from

"#]],
        ),
        (
            &["mktx", "--raw-unsigned"],
            [&call[..], &from].concat(),
            str![[r#"
Error: --tempo.configurable-access-key cannot be combined with --raw-unsigned, --ethsign, --signature or a Tempo access key

"#]],
        ),
        (
            &["send"],
            [&from[..], &["--create", "0x00"]].concat(),
            str![[r#"
Error: --tempo.configurable-access-key cannot deploy: Tempo access keys cannot sign CREATE

"#]],
        ),
        (
            &["keychain", "authorize", "0x5555555555555555555555555555555555555555"],
            from.to_vec(),
            str![[r#"
Error: an access key cannot manage the keys of the account it signs for

"#]],
        ),
    ];
    for (command, args, error) in rejected {
        cmd.cast_fuse()
            .args(command)
            .args(&access_args)
            .args(args)
            .assert_failure()
            .stderr_eq(error);
    }

    // The owner approves the keychain V2 hash, which recovers the account as the access key.
    let raw = cmd
        .cast_fuse()
        .arg("mktx")
        .args(call)
        .args(from)
        .args(&access_args)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let TempoTxEnvelope::AA(signed) =
        TempoTxEnvelope::decode_2718_exact(&hex::decode(raw.trim()).unwrap()).unwrap()
    else {
        panic!("expected a Tempo transaction");
    };
    let keychain = signed.signature().as_keychain().expect("a keychain signature");
    assert_eq!(keychain.user_address, root.address());
    assert!(!keychain.is_legacy());
    assert_eq!(keychain.key_id(&signed.signature_hash()).unwrap(), account);
    assert_eq!(keychain.signature.as_configurable_account().unwrap().config().version, 0);

    // The owners bind their approvals to the account's configuration version, 1 after the
    // account adds an owner. Root's own access key in the Tempo Accounts store never signs in
    // place of the configurable access key, even without a wallet option. In both cases the
    // estimate models the keychain-wrapped quorum exactly.
    api.anvil_deal_tip20(account, PATH_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    cmd.cast_fuse()
        .args(["wallet", "owners", "add", &account_address])
        .args([&Address::repeat_byte(0x66).to_string(), "--private-key", &owner_key])
        .args(["--rpc-url", &rpc, "--tempo.fee-token", &fee_token])
        .assert_success();
    let root_key = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x77)).unwrap();
    let authorization = cmd
        .cast_fuse()
        .args(["key-authorization", "sign", &root_key.address().to_string()])
        .args(["--chain-id", &CHAIN_ID.to_string(), "--private-key"])
        .arg(hex::encode_prefixed(root.to_bytes()))
        .assert_success()
        .get_output()
        .stdout_lossy();
    let tempo_home = tempfile::tempdir().unwrap();
    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.args(["tempo", "import-access-key", "--account", &root_address])
        .args(["--access-key", &hex::encode_prefixed(root_key.to_bytes())])
        .args(["--authorization", authorization.trim()])
        .assert_success();

    let owner_signer = format!("private-key://{owner_key}");
    // Each send approves a new amount, so that no send costs less than its estimate.
    let wallets =
        [(["--private-key", owner_key.as_str()], "2"), (["--tempo.owner", &owner_signer], "3")];
    for (wallet, amount) in wallets {
        let access_args = [&access_key[..], &wallet].concat();
        let call = [call[0], call[1], call[2], amount];
        cmd.cast_fuse();
        cmd.env("TEMPO_HOME", tempo_home.path());
        let estimate = cmd
            .arg("estimate")
            .args(call)
            .args(from)
            .args(&access_args)
            .assert_success()
            .get_output()
            .stdout_lossy();
        cmd.cast_fuse();
        cmd.env("TEMPO_HOME", tempo_home.path());
        let receipt = cmd
            .arg("send")
            .args(call)
            .args(from)
            .args(&access_args)
            .arg("--json")
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert_data_eq!(
            receipt.trim().is_json(),
            format!(
                r#"{{
  "status": "0x1",
  "from": "{:#x}",
  "gasUsed": "{:#x}",
  "...": "{{...}}"
}}"#,
                root.address(),
                estimate.trim().parse::<u64>().unwrap()
            )
            .is_json()
            .unordered()
        );
        let hash = parse_json::<serde_json::Value>(receipt.trim()).unwrap()["transactionHash"]
            .as_str()
            .unwrap()
            .to_owned();
        let raw = cmd
            .cast_fuse()
            .args(["tx", &hash, "--raw", "--rpc-url", &rpc])
            .assert_success()
            .get_output()
            .stdout_lossy();
        let TempoTxEnvelope::AA(sent) =
            TempoTxEnvelope::decode_2718_exact(&hex::decode(raw.trim()).unwrap()).unwrap()
        else {
            panic!("expected a Tempo transaction");
        };
        let keychain = sent.signature().as_keychain().expect("a keychain signature");
        assert_eq!(keychain.key_id(&sent.signature_hash()).unwrap(), account);
        assert_eq!(keychain.signature.as_configurable_account().unwrap().config().version, 1);
    }

    // The batch commands also skip root's stored access key, with the owner given only through
    // `--tempo.owner`.
    let access_args = [&access_key[..], &["--tempo.owner", &owner_signer]].concat();
    for (command, amount) in [("batch-mktx", "4"), ("batch-send", "5")] {
        let batch_call = format!("{fee_token}::approve(address,uint256):{},{amount}", call[2]);
        cmd.cast_fuse();
        cmd.env("TEMPO_HOME", tempo_home.path());
        let output = cmd
            .args([command, "--call", &batch_call])
            .args(from)
            .args(&access_args)
            .arg("--json")
            .assert_success()
            .get_output()
            .stdout_lossy();
        let raw = if command == "batch-mktx" {
            let raw = parse_json::<serde_json::Value>(output.trim())
                .map(|json| json["data"].as_str().unwrap_or_default().to_owned())
                .unwrap_or_else(|_| output.trim().to_owned());
            cmd.cast_fuse().args(["publish", &raw, "--rpc-url", &rpc]).assert_success();
            raw
        } else {
            assert_data_eq!(
                output.trim().is_json(),
                format!(
                    r#"{{
  "status": "0x1",
  "from": "{:#x}",
  "...": "{{...}}"
}}"#,
                    root.address()
                )
                .is_json()
                .unordered()
            );
            let hash = parse_json::<serde_json::Value>(output.trim()).unwrap()["transactionHash"]
                .as_str()
                .unwrap()
                .to_owned();
            cmd.cast_fuse()
                .args(["tx", &hash, "--raw", "--rpc-url", &rpc])
                .assert_success()
                .get_output()
                .stdout_lossy()
        };
        let TempoTxEnvelope::AA(sent) =
            TempoTxEnvelope::decode_2718_exact(&hex::decode(raw.trim()).unwrap()).unwrap()
        else {
            panic!("expected a Tempo transaction");
        };
        let keychain = sent.signature().as_keychain().expect("a keychain signature");
        assert_eq!(keychain.user_address, root.address());
        assert_eq!(keychain.key_id(&sent.signature_hash()).unwrap(), account);
    }
}
