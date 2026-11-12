//! CLI tests for Tempo configurable accounts (T14) in `cast wallet`.
//!
//! No public Tempo network schedules T14 yet, so the tests run against local Anvil T14 nodes.

use alloy_consensus::Transaction;
use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, U256, hex, keccak256};
use alloy_provider::Provider;
use alloy_signer_local::PrivateKeySigner;
use anvil::NodeConfig;
use base64::Engine;
use foundry_cli::utils::parse_json;
use foundry_common::{provider::RetryProvider, tempo::configurable_account::single_owner_account};
use foundry_evm::core::tempo::ALPHA_USD_ADDRESS;
use foundry_test_utils::{TestCommand, assert_data_eq, util::OutputExt};
use p256::ecdsa::{SigningKey, signature::Signer as _};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use tempo_hardfork::TempoHardfork;
use tempo_primitives::transaction::{
    CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, tt_signature::derive_p256_address,
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

/// `cast wallet new` creates an EOA by default. With `--configurable-account` it creates a
/// configurable account only with a Tempo signal on a chain where T14 is active, and prints the
/// account and its owner key.
#[casttest]
async fn tempo_t14_wallet_new_configurable_account_opt_in(prj: _, cmd: _) {
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

    // The default is an EOA, even on a T14 Tempo RPC.
    let (stdout, stderr) = wallet_new(&mut cmd, &["--rpc-url", &t14_rpc]);
    assert_eoa(&stdout);
    assert_data_eq!(
        stderr,
        str![[r#"
Successfully created new keypair.
Address:     [..]
Private key: [..]

"#]]
    );

    // No Tempo signal: a plain key, without consulting any RPC.
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

    // A `CHAIN` that is not a chain, such as one an app's `.env` sets, does not break key
    // generation, but it is still a Tempo signal for `--configurable-account`.
    let (stdout, _) = wallet_new_with_env(&mut cmd, &[], &[("CHAIN", "local")]);
    assert_eoa(&stdout);
    let (stdout, stderr) =
        wallet_new_with_env(&mut cmd, &["--configurable-account"], &[("CHAIN", "4217")]);
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

    // An explicit `--chain` is still validated.
    cmd.cast_fuse().args(["wallet", "new", "--chain", "locla"]);
    cmd.unset_env("CHAIN");
    cmd.assert_failure().stderr_eq(str![[r#"
error: invalid value 'locla' for '--chain <CHAIN>': invalid digit found in string

For more information, try '--help'.

"#]]);

    // `--eoa` asks for the default explicitly.
    let (stdout, _) = wallet_new(&mut cmd, &["--chain", "tempo", "--eoa"]);
    assert_eoa(&stdout);

    // A Tempo RPC before T14, and a non-Tempo RPC.
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

    let config = change(&["threshold", &account_arg, "3"]);
    assert_eq!((config["version"].as_u64(), config["threshold"].as_u64()), (Some(2), Some(3)));
    let (gas_limit, gas_used) = last_tx_gas(&provider).await;
    assert_eq!(gas_limit, gas_used);

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
