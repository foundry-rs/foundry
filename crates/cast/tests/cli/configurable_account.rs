//! CLI tests for Tempo configurable accounts (T14) in `cast wallet`.
//!
//! No public Tempo network schedules T14 yet, so the tests run against local Anvil T14 nodes.

use alloy_consensus::Transaction;
use alloy_eips::Decodable2718;
use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, U256, hex, keccak256};
use alloy_provider::Provider;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use anvil::NodeConfig;
use base64::Engine;
use foundry_cli::utils::parse_json;
use foundry_common::{provider::RetryProvider, tempo::configurable_account::single_owner_account};
use foundry_evm::core::tempo::ALPHA_USD_ADDRESS;
use foundry_test_utils::{TestCommand, assert_data_eq, util::OutputExt};
use p256::ecdsa::{SigningKey, signature::Signer as _};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use tempo_contracts::precompiles::ITIP20;
use tempo_hardfork::TempoHardfork;
use tempo_primitives::{
    TempoTxEnvelope,
    transaction::{
        CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, ConfigurableAccountConfig, ConfigurableAccountOwner,
        tt_signature::derive_p256_address,
    },
};

/// Returns the gas limit and the gas used of the last transaction in the latest block, and
/// checks that it succeeded.
async fn last_tx_gas(provider: &RetryProvider) -> (u64, u64) {
    let block = provider.get_block_number().await.unwrap();
    let receipts = provider.get_block_receipts(block.into()).await.unwrap().unwrap();
    let receipt = receipts.last().unwrap();
    assert!(receipt.status(), "{receipt:?}");
    let tx = provider.get_transaction_by_hash(receipt.transaction_hash()).await.unwrap().unwrap();
    (tx.gas_limit(), receipt.gas_used())
}

fn json_data(output: &str) -> serde_json::Value {
    let envelope: serde_json::Value = parse_json(output.trim()).expect("command emits JSON");
    assert_eq!(envelope["success"], true, "unexpected JSON envelope: {envelope}");
    envelope["data"].clone()
}

/// A WebAuthn assertion of `challenge` by the P-256 `key`, in the JSON shape that a browser's
/// `PublicKeyCredential.toJSON()` produces, plus the credential's public key.
fn passkey_assertion(key: &SigningKey, challenge: B256) -> serde_json::Value {
    let base64url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    // RP ID hash, then the user present and user verified flags and a zero signature counter.
    let mut authenticator_data = Sha256::digest(b"example.com").to_vec();
    authenticator_data.extend([0x05, 0, 0, 0, 0]);
    let client_data = format!(
        r#"{{"type":"webauthn.get","challenge":"{}","origin":"https://example.com","crossOrigin":false}}"#,
        base64url.encode(challenge)
    );
    let mut signed = authenticator_data.clone();
    signed.extend(Sha256::digest(client_data.as_bytes()));
    // The key hashes the message with SHA-256, as the authenticator does.
    let signature: p256::ecdsa::Signature = key.sign(&signed);
    serde_json::json!({
        "id": "passkey",
        "type": "public-key",
        "response": {
            "authenticatorData": base64url.encode(&authenticator_data),
            "clientDataJSON": base64url.encode(client_data.as_bytes()),
            "signature": base64url.encode(signature.to_der()),
        },
        "publicKey": hex::encode_prefixed(key.verifying_key().to_encoded_point(false).as_bytes()),
    })
}

fn passkey_owner(key: &SigningKey) -> Address {
    let point = key.verifying_key().to_encoded_point(false);
    derive_p256_address(
        &B256::from_slice(point.x().unwrap()),
        &B256::from_slice(point.y().unwrap()),
    )
}

/// Runs `cast wallet new` with `args` and returns its stdout and stderr.
fn wallet_new(cmd: &mut TestCommand, args: &[&str]) -> (String, String) {
    wallet_new_with_env(cmd, args, &[])
}

/// Runs `cast wallet new` with `args` and the environment `env`.
/// `ETH_RPC_URL` and `CHAIN` are cleared unless `env` sets them.
fn wallet_new_with_env(
    cmd: &mut TestCommand,
    args: &[&str],
    env: &[(&str, &str)],
) -> (String, String) {
    cmd.cast_fuse().args(["wallet", "new"]).args(args);
    cmd.unset_env("ETH_RPC_URL");
    cmd.unset_env("CHAIN");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.assert_success();
    let output = output.get_output();
    (output.stdout_lossy(), output.stderr_lossy())
}

/// `cast wallet new` creates a configurable account by default with a Tempo signal on a chain where
/// T14 is active, and prints the account and its owner key. `--eoa` opts out, and
/// `--configurable-account` also reports a missing Tempo signal.
#[casttest]
async fn tempo_t14_wallet_new_defaults_to_configurable_account(prj: _, cmd: _) {
    let (_t14_api, t14) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let (_t13_api, t13) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T13.into()))).await;
    let (_eth_api, eth) = anvil::spawn(NodeConfig::test()).await;
    let t14_rpc = t14.http_endpoint();

    let assert_eoa = |stdout: &str| {
        let [address, key] = stdout.trim().split('\t').collect::<Vec<_>>()[..] else {
            panic!("expected an EOA record, got {stdout}");
        };
        let signer = PrivateKeySigner::from_str(key).unwrap();
        assert_eq!(Address::from_str(address).unwrap(), signer.address());
    };
    let assert_account = |stdout: &str| {
        let [account, key, owner] = stdout.trim().split('\t').collect::<Vec<_>>()[..] else {
            panic!("expected a configurable account record, got {stdout}");
        };
        let owner = Address::from_str(owner).unwrap();
        assert_eq!(PrivateKeySigner::from_str(key).unwrap().address(), owner);
        assert_eq!(Address::from_str(account).unwrap(), single_owner_account(owner).unwrap());
    };

    // The default on a T14 Tempo RPC is a configurable account.
    let (stdout, stderr) = wallet_new(&mut cmd, &["--rpc-url", &t14_rpc]);
    assert_account(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Successfully created new Tempo configurable account.
Address:     [..]
Owner key:   [..]
Private key: [..]

"#]]
    );

    // No Tempo signal: a plain key, without consulting any RPC. Only `--configurable-account`
    // says why.
    let (stdout, stderr) = wallet_new(&mut cmd, &[]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );
    let (stdout, stderr) = wallet_new(&mut cmd, &["--configurable-account"]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
No Tempo chain was given (--chain, an RPC or foundry.toml); creating an EOA.
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );

    // A Tempo RPC with T14 active.
    let (stdout, stderr) = wallet_new(&mut cmd, &["--configurable-account", "--rpc-url", &t14_rpc]);
    assert_account(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Successfully created new Tempo configurable account.
Address:     [..]
Owner key:   [..]
Private key: [..]

"#]]
    );

    // `--eoa` opts out on a T14 Tempo RPC.
    let (stdout, stderr) = wallet_new(&mut cmd, &["--rpc-url", &t14_rpc, "--eoa"]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );

    // A Tempo RPC before T14, a non-Tempo RPC and a non-Tempo chain create a plain key; only
    // `--configurable-account` says why.
    for args in [
        &["--rpc-url", &t13.http_endpoint()][..],
        &["--rpc-url", &eth.http_endpoint()],
        &["--chain", "1"],
    ] {
        let (stdout, stderr) = wallet_new(&mut cmd, args);
        assert_eoa(&stdout);
        assert_data_eq!(
            stderr,
            str![[r#"
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
        );
    }
    let (stdout, stderr) =
        wallet_new(&mut cmd, &["--configurable-account", "--rpc-url", &t13.http_endpoint()]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Tempo T14 is not active on the RPC chain; creating an EOA.
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );
    let (stdout, stderr) =
        wallet_new(&mut cmd, &["--configurable-account", "--rpc-url", &eth.http_endpoint()]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
No Tempo chain was given (--chain, an RPC or foundry.toml); creating an EOA.
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );

    // The compiled schedule does not activate T14 on Tempo mainnet yet.
    let (stdout, stderr) = wallet_new(&mut cmd, &["--configurable-account", "--chain", "4217"]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Tempo T14 is not active on chain tempo; creating an EOA.
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );

    // `ETH_RPC_URL` and `foundry.toml` are Tempo signals too.
    let (stdout, _) =
        wallet_new_with_env(&mut cmd, &["--configurable-account"], &[("ETH_RPC_URL", &t14_rpc)]);
    assert_account(&stdout);
    std::fs::write(
        prj.root().join("foundry.toml"),
        format!("[profile.default]\neth_rpc_url = \"{t14_rpc}\"\n"),
    )
    .unwrap();
    cmd.cast_fuse().args(["wallet", "new", "--configurable-account"]).current_dir(prj.root());
    cmd.unset_env("ETH_RPC_URL");
    assert_account(&cmd.assert_success().get_output().stdout_lossy());

    // JSON names the account `address` and adds its owner key.
    let (stdout, _) =
        wallet_new(&mut cmd, &["--json", "--configurable-account", "--rpc-url", &t14_rpc]);
    let record = &json_data(&stdout)[0];
    let owner = Address::from_str(record["owner"].as_str().unwrap()).unwrap();
    assert_eq!(
        Address::from_str(record["address"].as_str().unwrap()).unwrap(),
        single_owner_account(owner).unwrap()
    );
}

/// A new account adds a passkey owner, then its owners sign on separate machines:
/// `mktx --raw-unsigned --tempo.digest`, `wallet sign --no-hash` and a browser WebAuthn assertion,
/// `wallet assemble`, then `publish`.
#[casttest]
async fn tempo_t14_wallet_configurable_account_multi_party_flow(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let fee_token = ALPHA_USD_ADDRESS.to_string();

    let stdout = cmd
        .cast_fuse()
        .args(["wallet", "new", "--configurable-account", "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let [account, key, owner] = stdout.trim().split('\t').collect::<Vec<_>>()[..] else {
        panic!("expected a configurable account record, got {stdout}");
    };
    let (account, key, owner) =
        (account.to_string(), key.to_string(), Address::from_str(owner).unwrap());
    api.anvil_deal_tip20(
        Address::from_str(&account).unwrap(),
        ALPHA_USD_ADDRESS,
        U256::from(1_000_000_000_000u64),
    )
    .await
    .unwrap();

    // The account is unregistered until its first transaction, so it has no onchain owners.
    cmd.cast_fuse()
        .args(["wallet", "owners", &account, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: [..] is not a registered configurable account. An account registers on its first transaction; until then its configuration is only known to its owners.

"#]]);

    // Only the owners know an unregistered account's configuration, so its quorum cannot be
    // simulated.
    let target = Address::repeat_byte(0x73).to_string();
    cmd.cast_fuse()
        .args(["mktx", &target, "--from", &account, "--raw-unsigned", "--tempo.digest"])
        .args(["--rpc-url", &rpc, "--tempo.fee-token", &fee_token])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: [..] is not registered yet, so its owners and the gas of its registration are unknown. Send its first transaction with `cast wallet owners`, or pass --gas-limit

"#]]);

    // The first transaction registers the 1-of-1 account and adds the passkey at threshold 2.
    let passkey = SigningKey::from_slice(&[0x22; 32]).unwrap();
    let passkey_owner = passkey_owner(&passkey);
    cmd.cast_fuse()
        .args(["wallet", "owners", "add", &account, &passkey_owner.to_string()])
        .args(["--threshold", "2", "--private-key", &key, "--rpc-url", &rpc])
        .args(["--tempo.fee-token", &fee_token])
        .assert_success();
    // The estimate models the owner's secp256k1 approval and the registration exactly.
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);
    let config = json_data(
        &cmd.cast_fuse()
            .args(["wallet", "owners", &account, "--rpc-url", &rpc, "--json"])
            .assert_success()
            .get_output()
            .stdout_lossy(),
    );
    assert_eq!(config["version"], 1);
    assert_eq!(config["threshold"], 2);
    let mut owners = [owner, passkey_owner];
    owners.sort();
    assert_eq!(
        config["owners"],
        serde_json::json!(
            owners
                .iter()
                .map(|owner| serde_json::json!({ "owner": owner, "weight": 1 }))
                .collect::<Vec<_>>()
        )
    );

    // One owner cannot change a 2-of-2 configuration alone.
    cmd.cast_fuse()
        .args(["wallet", "owners", "threshold", &account, "1", "--private-key", &key])
        .args(["--rpc-url", &rpc, "--tempo.fee-token", &fee_token])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: the wallet [..] has weight 1 in [..], which requires 2. Collect the owners' approvals instead:
  cast mktx 0xAACC000000000000000000000000000000000000 [..] --from [..] --raw-unsigned --tempo.digest
then `cast wallet sign --no-hash <DIGEST>` on each owner's machine, `cast wallet assemble`, and `cast publish`

"#]]);

    // The multi-party flow.
    let stdout = cmd
        .cast_fuse()
        .args(["mktx", &target, "--from", &account, "--raw-unsigned", "--tempo.digest"])
        .args(["--rpc-url", &rpc, "--tempo.fee-token", &fee_token])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let [unsigned, digest] = stdout.trim().split('\t').collect::<Vec<_>>()[..] else {
        panic!("expected the unsigned transaction and the digest, got {stdout}");
    };
    let digest = B256::from_str(digest).unwrap();

    let owner_signature = cmd
        .cast_fuse()
        .args(["wallet", "sign", "--no-hash", &digest.to_string(), "--private-key", &key])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let assertion = prj.root().join("passkey-assertion.json");
    std::fs::write(&assertion, passkey_assertion(&passkey, digest).to_string()).unwrap();

    // An approval of another digest is rejected.
    let wrong = passkey_assertion(&passkey, B256::repeat_byte(1)).to_string();
    cmd.cast_fuse()
        .args(["wallet", "assemble", unsigned, owner_signature.trim(), &wrong])
        .args(["--from", &account, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: signature 2 does not verify against the owner digest [..]; owners sign it with `cast wallet sign --no-hash [..]`

"#]]);

    let raw = cmd
        .cast_fuse()
        .args(["wallet", "assemble", unsigned, owner_signature.trim()])
        .arg(&assertion)
        .args(["--from", &account, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(raw.trim().starts_with("0x76"), "{raw}");
    cmd.cast_fuse().args(["publish", raw.trim(), "--rpc-url", &rpc]).assert_success();

    let block = provider.get_block_number().await.unwrap();
    let receipts = provider.get_block_receipts(block.into()).await.unwrap().unwrap();
    assert_eq!(receipts.last().unwrap().from(), Address::from_str(&account).unwrap());
    // Without the owners' key types, the estimate covers two maximum-size WebAuthn approvals,
    // which one secp256k1 approval and one passkey assertion stay below.
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert!(gas_used < gas_limit, "{gas_used} >= {gas_limit}");
}

/// `owners remove` and `owners threshold` send `updateConfig` signed by an owner that reaches the
/// threshold alone, and `owners` reads each new configuration back from the chain.
#[casttest]
async fn tempo_t14_wallet_owners_remove_and_threshold(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let fee_token = ALPHA_USD_ADDRESS.to_string();
    let key = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x31)).unwrap();
    let account = single_owner_account(key.address()).unwrap();
    api.anvil_deal_tip20(account, ALPHA_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    let key_arg = hex::encode_prefixed(key.to_bytes());
    let (account_arg, other) = (account.to_string(), Address::repeat_byte(0x44).to_string());

    let mut change = |args: &[&str]| {
        cmd.cast_fuse()
            .args(["wallet", "owners"])
            .args(args)
            .args(["--private-key", &key_arg, "--rpc-url", &rpc, "--tempo.fee-token", &fee_token])
            .assert_success();
        json_data(
            &cmd.cast_fuse()
                .args(["wallet", "owners", &account_arg, "--rpc-url", &rpc, "--json"])
                .assert_success()
                .get_output()
                .stdout_lossy(),
        )
    };

    // Each estimate, with the registration on the first transaction, matches the gas used.
    let config = change(&["add", &account_arg, &other, "--weight", "2"]);
    assert_eq!((config["version"].as_u64(), config["threshold"].as_u64()), (Some(1), Some(1)));
    assert_eq!(config["owners"].as_array().unwrap().len(), 2);
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);
    let added = provider.get_block_number().await.unwrap().to_string();

    let config = change(&["threshold", &account_arg, "3"]);
    assert_eq!((config["version"].as_u64(), config["threshold"].as_u64()), (Some(2), Some(3)));
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);

    // An estimate at an earlier block uses the configuration there, where the key alone still
    // reaches threshold 1.
    let estimate = cmd
        .cast_fuse()
        .args(["estimate", &other, "--from", &account_arg, "--private-key", &key_arg])
        .args(["--rpc-url", &rpc, "--tempo.fee-token", &fee_token, "--block", &added])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(estimate.trim().parse::<u64>().unwrap() > 0);

    // The key alone no longer reaches threshold 3.
    cmd.cast_fuse()
        .args(["wallet", "owners", "remove", &account_arg, &other, "--threshold", "1"])
        .args(["--private-key", &key_arg, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: the wallet 0x77952Ce83Ca3cad9F7AdcFabeDA85Bd2F1f52008 has weight 1 in 0xC114fe44bdC44dDE8b76b033D0C49ab350a80B20, which requires 3. Collect the owners' approvals instead:
  cast mktx 0xAACC000000000000000000000000000000000000 [..] --from 0xC114fe44bdC44dDE8b76b033D0C49ab350a80B20 --raw-unsigned --tempo.digest
then `cast wallet sign --no-hash <DIGEST>` on each owner's machine, `cast wallet assemble`, and `cast publish`

"#]]);
}

/// `cast wallet upgrade` asks before converting an EOA, warns that the account can no longer
/// sign MPP vouchers, and installs the TIP-1113 configuration.
#[casttest]
async fn tempo_t14_wallet_upgrade(cmd: _) {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().nth(3).unwrap();
    let key = hex::encode_prefixed(wallet.credential().to_bytes());
    let root = wallet.address();
    let (owner_a, owner_b) = (Address::repeat_byte(0x44), Address::repeat_byte(0x55));
    let fee_token = ALPHA_USD_ADDRESS.to_string();
    let upgrade_args = [
        "wallet".to_string(),
        "upgrade".to_string(),
        "--owner".to_string(),
        owner_a.to_string(),
        "--owner".to_string(),
        format!("{owner_b}:2"),
        "--threshold".to_string(),
        "2".to_string(),
        "--private-key".to_string(),
        key,
        "--rpc-url".to_string(),
        rpc.clone(),
        "--tempo.fee-token".to_string(),
        fee_token,
    ];

    // A `--from` other than the signer's account is rejected before anything is sent.
    cmd.cast_fuse()
        .args(&upgrade_args)
        .args(["--from", &owner_a.to_string(), "--force"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: --from 0x4444444444444444444444444444444444444444 does not match the signer [..]; the upgrade converts the signer's account

"#]]);

    // Declining leaves the account unchanged.
    cmd.cast_fuse().args(&upgrade_args).stdin("n\n").assert_success().stderr_eq(str![[r#"
Upgrading 0x90F79bf6EB2c4f870365E785982E1f101E93b906 to a configurable account:
  Threshold: 2
  Owner:     0x4444444444444444444444444444444444444444 (weight 1)
  Owner:     0x5555555555555555555555555555555555555555 (weight 2)
Warning: The upgrade is irreversible: the key of 0x90F79bf6EB2c4f870365E785982E1f101E93b906 can no longer sign for it, only its owners can. The account can also no longer act as an MPP voucher signer: opening or topping up payment channels reverts with RootKeyRetired. Keep an EOA for MPP.

Continue anyway? [y/N] Aborted.

"#]]);
    cmd.cast_fuse()
        .args(["wallet", "owners", &root.to_string(), "--rpc-url", &rpc])
        .assert_failure();

    cmd.cast_fuse().args(&upgrade_args).stdin("y\n").assert_success();
    let config = json_data(
        &cmd.cast_fuse()
            .args(["wallet", "owners", &root.to_string(), "--rpc-url", &rpc, "--json"])
            .assert_success()
            .get_output()
            .stdout_lossy(),
    );
    assert_eq!(config["account"], serde_json::json!(root));
    assert_eq!(config["version"], 1);
    assert_eq!(config["threshold"], 2);
    assert_eq!(
        config["salt"],
        serde_json::json!(keccak256(
            [CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, root.as_slice()].concat()
        ))
    );
    assert_eq!(
        config["owners"],
        serde_json::json!([{ "owner": owner_a, "weight": 1 }, { "owner": owner_b, "weight": 2 }])
    );

    // An account upgrades only once.
    cmd.cast_fuse().args(&upgrade_args).arg("--force").assert_failure().stderr_eq(str![[r#"
Error: [..] is already a configurable account

"#]]);
}

/// A 2-of-3 configurable account over the keys `0x61..61`, `0x62..62` and `0x63..63`, the
/// account it derives, its configuration as `--tempo.config` takes it, and the keys sorted by
/// address.
fn two_of_three() -> (Address, String, [PrivateKeySigner; 3]) {
    let mut keys = [0x61, 0x62, 0x63]
        .map(|byte| PrivateKeySigner::from_bytes(&B256::repeat_byte(byte)).unwrap());
    keys.sort_by_key(|key| key.address());
    let config = ConfigurableAccountConfig {
        salt: B256::repeat_byte(0x07),
        version: 0,
        threshold: 2,
        owners: keys
            .iter()
            .map(|key| ConfigurableAccountOwner { owner: key.address(), weight: 1 })
            .collect(),
    };
    let json = serde_json::json!({
        "salt": config.salt,
        "version": config.version,
        "threshold": config.threshold,
        "owners": config.owners.iter().map(|owner| serde_json::json!({
            "owner": owner.owner,
            "weight": owner.weight,
        })).collect::<Vec<_>>(),
    });
    (config.derive_account().unwrap(), json.to_string(), keys)
}

fn private_key(key: &PrivateKeySigner) -> String {
    hex::encode_prefixed(key.to_bytes())
}

/// Returns the receipt of the last transaction in the latest block, as JSON.
async fn last_receipt(provider: &RetryProvider) -> serde_json::Value {
    let block = provider.get_block_number().await.unwrap();
    let receipts = provider.get_block_receipts(block.into()).await.unwrap().unwrap();
    let hash = receipts.last().unwrap().transaction_hash();
    provider
        .raw_request::<_, serde_json::Value>("eth_getTransactionReceipt".into(), (hash,))
        .await
        .unwrap()
}

/// `cast send`, `mktx`, `batch-send` and `estimate` detect a 2-of-3 configurable account sender
/// from the chain and sign `0x05` with its local owners, without flags once it is registered.
/// The estimate models the owners' approvals exactly, missing owners fail clearly, and a plain
/// EOA keeps its secp256k1 signature.
#[casttest]
async fn tempo_t14_send_detects_configurable_account(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let (account, config, keys) = two_of_three();
    api.anvil_deal_tip20(account, ALPHA_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    let (account_arg, target) = (account.to_string(), Address::repeat_byte(0x73).to_string());
    let fee_token = ALPHA_USD_ADDRESS.to_string();
    let common = ["--rpc-url", &rpc, "--tempo.fee-token", &fee_token];
    let owner = |key: &PrivateKeySigner| format!("private-key://{}", private_key(key));

    // An account that has not sent its first transaction has no onchain configuration, so its
    // first transaction takes it from `--tempo.config` and registers the account.
    cmd.cast_fuse()
        .args(["send", &target, "--from", &account_arg, "--private-key", &private_key(&keys[0])])
        .args(["--tempo.owner", &owner(&keys[1]), "--tempo.config", &config])
        .args(common)
        .assert_success()
        .stderr_eq(format!(
            "Sending as Tempo configurable account {account}\nPaying gas in AlphaUSD \
             ({ALPHA_USD_ADDRESS})\n"
        ));
    let receipt = last_receipt(&provider).await;
    assert_eq!(receipt["from"], serde_json::json!(account));
    // The estimate models the two secp256k1 approvals and the registration exactly.
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);

    // Registered, the account is detected from its configuration event; the owners need not be
    // given in address order.
    let estimate = cmd
        .cast_fuse()
        .args([
            "estimate",
            &target,
            "--from",
            &account_arg,
            "--private-key",
            &private_key(&keys[2]),
        ])
        .args(["--tempo.owner", &owner(&keys[1])])
        .args(common)
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.cast_fuse()
        .args(["send", &target, "--from", &account_arg, "--private-key", &private_key(&keys[2])])
        .args(["--tempo.owner", &owner(&keys[1])])
        .args(common)
        .assert_success();
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);
    assert_eq!(estimate.trim().parse::<u64>().unwrap(), gas_used);
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(account));

    // `estimate`, like `send`, rejects a `--tempo.configurable-account` other than `--from`.
    let other = keys[2].address();
    cmd.cast_fuse()
        .args(["estimate", &target, "--from", &other.to_string()])
        .args(["--tempo.configurable-account", &account_arg])
        .args(["--private-key", &private_key(&keys[2])])
        .args(common)
        .assert_failure()
        .stderr_eq(format!(
            "Error: --tempo.configurable-account {account} does not match the sender {other}\n"
        ));

    // `--tempo.configurable-account` names the sender without `--from`.
    let raw = cmd
        .cast_fuse()
        .args(["mktx", &target, "--tempo.configurable-account", &account_arg])
        .args(["--private-key", &private_key(&keys[0]), "--tempo.owner", &owner(&keys[2])])
        .args(common)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let envelope = TempoTxEnvelope::decode_2718_exact(&hex::decode(raw.trim()).unwrap()).unwrap();
    let TempoTxEnvelope::AA(signed) = &envelope else {
        panic!("expected a Tempo transaction");
    };
    assert_eq!(signed.signature().as_configurable_account().unwrap().signatures().len(), 2);
    cmd.cast_fuse().args(["publish", raw.trim(), "--rpc-url", &rpc]).assert_success();
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(account));

    cmd.cast_fuse()
        .args(["batch-send", "--call", &target, "--call", &target, "--from", &account_arg])
        .args(["--private-key", &private_key(&keys[1]), "--tempo.owner", &owner(&keys[0])])
        .args(common)
        .assert_success();
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(account));

    // One owner does not reach the threshold.
    cmd.cast_fuse()
        .args(["send", &target, "--from", &account_arg, "--private-key", &private_key(&keys[0])])
        .args(common)
        .assert_failure()
        .stderr_eq(format!(
            "Error: {account} is a Tempo configurable account that needs owner weight 2, but the \
             local owner keys {} reach weight 1. Its owners are {} (weight 1), {} (weight 1), {} \
             (weight 1). Pass the other owner keys with --tempo.owner, or have each owner sign \
             the digest from `cast mktx --raw-unsigned --tempo.digest` with `cast wallet sign \
             --no-hash` and combine the signatures with `cast wallet assemble`.\n",
            keys[0].address(),
            keys[0].address(),
            keys[1].address(),
            keys[2].address(),
        ));

    // A plain EOA keeps its secp256k1 signature.
    let eoa = handle.dev_wallets().next().unwrap();
    cmd.cast_fuse()
        .args(["send", &target, "--private-key", &private_key(&eoa)])
        .args(["--from", &eoa.address().to_string()])
        .args(common)
        .assert_success()
        .stderr_eq(str![[r#"
Paying gas in AlphaUSD (0x20C0000000000000000000000000000000000001)

"#]]);
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(eoa.address()));
    let block = provider.get_block_number().await.unwrap();
    let receipts = provider.get_block_receipts(block.into()).await.unwrap().unwrap();
    let raw = provider
        .raw_request::<_, alloy_primitives::Bytes>(
            "eth_getRawTransactionByHash".into(),
            (receipts.last().unwrap().transaction_hash(),),
        )
        .await
        .unwrap();
    let TempoTxEnvelope::AA(signed) = TempoTxEnvelope::decode_2718_exact(&raw).unwrap() else {
        panic!("expected a Tempo transaction");
    };
    assert_eq!(
        signed.signature().primitive_signature_type(),
        Some(tempo_primitives::SignatureType::Secp256k1)
    );
}

/// A configurable account sends with a sponsor paying its fees: the owners sign the sender's
/// `0x05` signature and the sponsor its fee-payer signature.
#[casttest]
async fn tempo_t14_send_configurable_account_sponsored(cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let (account, config, keys) = two_of_three();
    let sponsor = handle.dev_wallets().nth(1).unwrap();
    api.anvil_deal_tip20(sponsor.address(), ALPHA_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    let balance = |address: Address| {
        let provider = provider.clone();
        async move {
            let call = ITIP20::balanceOfCall { account: address };
            let output = provider
                .raw_request::<_, alloy_primitives::Bytes>(
                    "eth_call".into(),
                    (serde_json::json!({
                        "to": ALPHA_USD_ADDRESS,
                        "data": alloy_primitives::Bytes::from(call.abi_encode()),
                    }),),
                )
                .await
                .unwrap();
            U256::from_be_slice(&output)
        }
    };
    // Gas estimation runs before the sponsor signs, so it checks the sender's balance as for any
    // sponsored sender; the send itself must leave it untouched.
    api.anvil_deal_tip20(account, ALPHA_USD_ADDRESS, U256::from(1_000_000_000_000u64))
        .await
        .unwrap();
    let (sponsor_before, account_before) =
        (balance(sponsor.address()).await, balance(account).await);

    cmd.cast_fuse()
        .args(["send", &Address::repeat_byte(0x73).to_string(), "--from", &account.to_string()])
        .args(["--private-key", &private_key(&keys[0]), "--tempo.config", &config])
        .args(["--tempo.owner", &format!("private-key://{}", private_key(&keys[2]))])
        .args(["--tempo.fee-token", &ALPHA_USD_ADDRESS.to_string(), "--rpc-url", &rpc])
        .args(["--tempo.sponsor", &sponsor.address().to_string()])
        .args(["--tempo.sponsor-signer", &format!("private-key://{}", private_key(&sponsor))])
        .assert_success();
    let receipt = last_receipt(&provider).await;
    assert_eq!(receipt["from"], serde_json::json!(account));
    assert_eq!(receipt["feePayer"], serde_json::json!(sponsor.address()));
    assert!(balance(sponsor.address()).await < sponsor_before);
    assert_eq!(balance(account).await, account_before);
}

/// `cast wallet new` links a new keystore to its configurable account, and on T14 `--account`
/// sends from that account. `--from` or `--eoa` sends from the owner key, a bare `--private-key`
/// stays an EOA, and before T14 the keystore sends from its key.
// HOME does not override the default keystore directory on Windows.
#[cfg(not(windows))]
#[casttest]
async fn tempo_t14_send_from_linked_keystore(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let (t13_api, t13) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T13.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let home = prj.root();
    let password = home.join("password");
    std::fs::write(&password, "secret").unwrap();

    cmd.cast_fuse().env("HOME", home);
    cmd.args(["wallet", "new", "linked", "--unsafe-password", "secret", "--rpc-url", &rpc]);
    let stdout = cmd.assert_success().get_output().stdout_lossy();
    let [account, owner] = stdout.trim().split('\t').collect::<Vec<_>>()[..] else {
        panic!("expected a configurable account record, got {stdout}");
    };
    let (account, owner) = (Address::from_str(account).unwrap(), Address::from_str(owner).unwrap());
    assert_eq!(account, single_owner_account(owner).unwrap());
    let link: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".foundry/keystores/linked.tempo-account")).unwrap(),
    )
    .unwrap();
    assert_eq!(link, serde_json::json!({ "version": 1, "account": account, "owner": owner }));

    // The link is not listed as an account.
    cmd.cast_fuse().env("HOME", home);
    cmd.args(["wallet", "list"]);
    assert_eq!(cmd.assert_success().get_output().stdout_lossy(), "linked (Local)\n");

    for address in [account, owner] {
        for api in [&api, &t13_api] {
            api.anvil_deal_tip20(address, ALPHA_USD_ADDRESS, U256::from(1_000_000_000_000u64))
                .await
                .unwrap();
        }
    }
    cmd.cast_fuse().env("HOME", home);
    cmd.args(["wallet", "decrypt-keystore", "linked", "--unsafe-password", "secret", "--json"]);
    let key = json_data(&cmd.assert_success().get_output().stdout_lossy())["private_key"]
        .as_str()
        .unwrap()
        .to_string();
    let target = Address::repeat_byte(0x73).to_string();
    let fee_token = ALPHA_USD_ADDRESS.to_string();
    let mut send = |rpc: &str, args: &[&str]| {
        cmd.cast_fuse().env("HOME", home);
        cmd.args(["send", &target, "--rpc-url", rpc]);
        cmd.args(["--tempo.fee-token", &fee_token]).args(args);
        cmd.assert_success().get_output().stderr_lossy()
    };
    let password = password.to_str().unwrap();
    let keystore = ["--account", "linked", "--password-file", password];

    // On T14 the keystore sends from its account, and the gas estimate is exact.
    let paying = format!("Paying gas in AlphaUSD ({ALPHA_USD_ADDRESS})\n");
    assert_data_eq!(
        send(&rpc, &keystore),
        format!("Sending as Tempo configurable account {account}\n{paying}")
    );
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(account));
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);

    // `--from` with the owner key, or `--eoa`, sends from the key.
    let owner_arg = owner.to_string();
    for extra in [&["--from", owner_arg.as_str()][..], &["--eoa"]] {
        assert_data_eq!(send(&rpc, &[&keystore[..], extra].concat()), paying.clone());
        assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(owner));
    }

    // A bare private key of the owner stays an EOA.
    assert_data_eq!(send(&rpc, &["--private-key", &key]), paying.clone());
    assert_eq!(last_receipt(&provider).await["from"], serde_json::json!(owner));

    // Before T14 the keystore sends from its key.
    assert_data_eq!(send(&t13.http_endpoint(), &keystore), paying);
    assert_eq!(last_receipt(&t13.http_provider()).await["from"], serde_json::json!(owner));
}
