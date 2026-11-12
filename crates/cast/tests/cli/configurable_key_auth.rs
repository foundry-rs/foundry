//! CLI tests for Tempo key authorizations signed by a configurable account's owners (T14,
//! TIP-1111).

use alloy_primitives::{Address, B256, U256, hex};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use anvil::NodeConfig;
use foundry_cli::utils::parse_json;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::{TestCommand, util::OutputExt};
use tempo_hardfork::TempoHardfork;
use tempo_primitives::transaction::{
    ConfigurableAccountConfig, ConfigurableAccountOwner, KeyAuthorization, SignatureType,
    SignedKeyAuthorization, configurable_account_digest,
};

/// Anvil's Tempo chain ID.
const CHAIN_ID: u64 = 31337;

/// The account that a 1-of-1, salt-0 configuration of `owner` derives.
fn single_owner_account(owner: Address) -> Address {
    ConfigurableAccountConfig {
        salt: B256::ZERO,
        version: 0,
        threshold: 1,
        owners: vec![ConfigurableAccountOwner { owner, weight: 1 }],
    }
    .derive_account()
    .unwrap()
}

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
    let account = single_owner_account(first.address());
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
    let stderr = cmd
        .cast_fuse()
        .args(["keychain", "authorize", &onchain_key.address().to_string()])
        .args(&send_args)
        .args(["--tempo.session", &B256::repeat_byte(0x55).to_string()])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stderr.contains("Tempo Accounts sessions cannot sign"), "{stderr}");
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
    let stderr = cmd
        .cast_fuse()
        .args(["keychain", "authorize", &inline_key.address().to_string()])
        .args(&send_args)
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stderr.contains("reach weight 1"), "{stderr}");

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
    let inspect = cmd
        .cast_fuse()
        .args(["key-authorization", "inspect", &signed, "--account", &account.to_string()])
        .arg("--json")
        .assert_success()
        .get_output()
        .stdout_lossy();
    let inspect = parse_json::<serde_json::Value>(inspect.trim()).unwrap();
    assert_eq!(inspect["signer"], serde_json::Value::Null, "{inspect}");

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

/// An EOA authorizes a configurable account as its access key (key type 3).
#[casttest]
async fn tempo_t14_keychain_authorizes_configurable_account_as_access_key(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let root = handle.dev_wallets().next().unwrap();
    let account = single_owner_account(Address::repeat_byte(0x11));

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

    let check = cmd
        .cast_fuse()
        .args([
            "keychain",
            "check",
            &root.address().to_string(),
            &account.to_string(),
            "--rpc-url",
            &rpc,
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let check = parse_json::<serde_json::Value>(check.trim()).unwrap();
    assert_eq!(check["data"]["provisioned"], true, "{check}");
    assert_eq!(check["data"]["signatureType"], "configurableaccount", "{check}");

    // The browser wallet cannot serialize this key type yet, so the browser flow never starts.
    let stderr = cmd
        .cast_fuse()
        .args(["key-authorization", "sign", &account.to_string(), "--key-type", "configurable"])
        .args(["--chain-id", &CHAIN_ID.to_string(), "--browser"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stderr.contains("does not support the configurable key type"), "{stderr}");
}
