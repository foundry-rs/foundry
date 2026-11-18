//! Gas estimates and impersonation for Tempo configurable accounts (TIP-1107, TIP-1111, TIP-1113).
//!
//! Owners have not signed when a transaction is estimated or impersonated, so Anvil models their
//! quorum with placeholder `0x05` approvals, as a Tempo node does for `eth_estimateGas`.

use super::*;
use alloy_provider::PendingTransactionBuilder;

/// A configuration over `owners`, given as keys and their weights.
fn weighted_config(
    salt: B256,
    version: u64,
    threshold: u8,
    owners: &[(&PrivateKeySigner, u8)],
) -> ConfigurableAccountConfig {
    let mut owners = owners
        .iter()
        .map(|(key, weight)| ConfigurableAccountOwner { owner: key.address(), weight: *weight })
        .collect::<Vec<_>>();
    owners.sort_by_key(|owner| owner.owner);
    ConfigurableAccountConfig { salt, version, threshold, owners }
}

/// A derived account requiring two of three equal owners, and one whose heaviest owner needs one
/// more approval to reach a threshold of three. The account holds only the approving keys.
fn quorum_accounts() -> Vec<TestConfigurableAccount> {
    let (a, b, c) = (seeded_key(0x51), seeded_key(0x52), seeded_key(0x53));
    let equal = weighted_config(B256::repeat_byte(0x01), 0, 2, &[(&a, 1), (&b, 1), (&c, 1)]);
    let weighted = weighted_config(B256::repeat_byte(0x02), 0, 3, &[(&a, 2), (&b, 1), (&c, 1)]);
    [(equal, vec![b.clone(), c]), (weighted, vec![a, b])]
        .into_iter()
        .map(|(config, keys)| TestConfigurableAccount {
            address: config.derive_account().unwrap(),
            config,
            keys,
        })
        .collect()
}

/// The `configurableAccountSimulation` witness of `account`, in which its keys approve with
/// signatures of `key_type`, or maximum-size `WebAuthn` signatures when it is `None`.
fn witness(account: &TestConfigurableAccount, key_type: Option<&str>) -> serde_json::Value {
    let approvals = account
        .config
        .owners
        .iter()
        .filter(|owner| account.keys.iter().any(|key| key.address() == owner.owner))
        .map(|owner| match key_type {
            Some(key_type) => serde_json::json!({ "owner": owner.owner, "keyType": key_type }),
            None => serde_json::json!({ "owner": owner.owner }),
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "config": Bytes::from(alloy_rlp::encode(&account.config)),
        "approvals": approvals,
    })
}

/// A Tempo request from `from` that pays its fees in AlphaUSD.
fn request(from: Address, nonce: u64) -> serde_json::Value {
    serde_json::json!({
        "from": from,
        "type": "0x76",
        "nonce": U256::from(nonce),
        "nonceKey": U256::ZERO,
        "feeToken": ALPHA_USD,
        "calls": [noop()],
    })
}

/// Returns the gas estimate of `request`, or the error it is rejected with.
async fn estimate(provider: &RetryProvider, request: serde_json::Value) -> Result<u64, String> {
    provider
        .raw_request::<_, U256>("eth_estimateGas".into(), (request,))
        .await
        .map(|gas| gas.to())
        .map_err(|err| err.to_string())
}

/// Sends `account`'s owner-signed no-op at `nonce` with `gas_limit` and returns its gas used.
async fn send_owner_signed(
    provider: &RetryProvider,
    account: &TestConfigurableAccount,
    nonce: u64,
    gas_limit: u64,
) -> Result<u64, String> {
    let mut tx = configurable_account_tx(provider, nonce, vec![noop()]).await;
    tx.gas_limit = gas_limit;
    let signature = account.sign(&tx).await;
    let receipt = send_signed_tempo_tx(provider, tx, signature).await?;
    assert!(receipt.status(), "owner-signed transaction should succeed: {receipt:?}");
    assert_eq!(receipt.from, account.address);
    Ok(receipt.gas_used)
}

/// Impersonates `account` and sends its no-op at `nonce`, which must carry a placeholder `0x05`
/// signature over the account's configuration. Without a `gas_limit`, the transaction is estimated
/// first and must use the gas it is estimated at. Returns its gas used.
async fn send_impersonated(
    provider: &RetryProvider,
    account: &TestConfigurableAccount,
    nonce: u64,
    gas_limit: Option<u64>,
) -> u64 {
    let address = account.address;
    provider.raw_request::<_, ()>("anvil_impersonateAccount".into(), (address,)).await.unwrap();
    let mut impersonated = request(address, nonce);
    let estimated = match gas_limit {
        Some(gas_limit) => {
            impersonated["gas"] = serde_json::json!(U256::from(gas_limit));
            None
        }
        None => Some(estimate(provider, impersonated.clone()).await.unwrap()),
    };
    let hash = provider
        .raw_request::<_, B256>("eth_sendTransaction".into(), (impersonated,))
        .await
        .unwrap();
    let receipt =
        PendingTransactionBuilder::new(provider.root().clone(), hash).get_receipt().await.unwrap();
    assert!(receipt.status(), "impersonated transaction should succeed: {receipt:?}");
    assert_eq!(receipt.from, address);
    if let Some(estimated) = estimated {
        assert_eq!(receipt.gas_used, estimated);
    }

    let raw = provider
        .raw_request::<_, Bytes>("eth_getRawTransactionByHash".into(), (hash,))
        .await
        .unwrap();
    let TempoTxEnvelope::AA(tx) = TempoTxEnvelope::decode_2718(&mut raw.as_ref()).unwrap() else {
        panic!("expected a Tempo transaction");
    };
    let TempoSignature::ConfigurableAccount(signature) = tx.signature() else {
        panic!("expected a 0x05 signature, got {:?}", tx.signature());
    };
    assert_eq!(signature.account(), address);
    assert_eq!(signature.config(), &account.config);
    receipt.gas_used
}

/// With a `configurableAccountSimulation` witness, the estimate of a transaction from a
/// configurable account is exactly the gas its owner-signed transaction then uses, both for the
/// transaction that registers the account and for a later one, and `eth_call` runs it.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t14_configurable_account_estimate_matches_owner_signed_gas() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let provider = handle.http_provider();

    for account in quorum_accounts() {
        deal(&provider, account.address).await;
        let mut used = Vec::new();
        for nonce in 0..2 {
            let mut simulated = request(account.address, nonce);
            simulated["configurableAccountSimulation"] = witness(&account, Some("secp256k1"));
            let output = provider
                .raw_request::<_, Bytes>("eth_call".into(), (simulated.clone(),))
                .await
                .unwrap();
            assert!(output.is_empty());
            let gas = estimate(&provider, simulated.clone()).await.unwrap();

            // An approval without a key type is priced as the largest `WebAuthn` assertion.
            simulated["configurableAccountSimulation"] = witness(&account, None);
            let webauthn = estimate(&provider, simulated).await.unwrap();
            assert!(webauthn > gas, "{webauthn} should exceed the secp256k1 estimate {gas}");

            // The estimate is tight: the owner-signed transaction fails with one gas less and
            // uses all of it.
            let error = send_owner_signed(&provider, &account, nonce, gas - 1).await.unwrap_err();
            assert!(error.contains("CallGasCostMoreThanGasLimit"), "unexpected error: {error}");
            assert_eq!(send_owner_signed(&provider, &account, nonce, gas).await.unwrap(), gas);
            used.push(gas);
        }
        assert_eq!(
            config_commitment(&provider, account.address).await,
            account.config.commitment().unwrap()
        );
        // Only the first transaction pays for registering the account.
        assert!(used[0] > used[1], "registration should cost more: {used:?}");
    }
}

/// Simulation checks the claimed quorum against the witness configuration and requires a
/// witness for a configurable account signature, as a Tempo node does.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t14_configurable_account_simulation_rejects_invalid_quorum() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let provider = handle.http_provider();
    let account = quorum_accounts().remove(0);
    deal(&provider, account.address).await;

    let mut simulated = request(account.address, 0);
    simulated["keyType"] = "configurableAccount".into();
    let error = estimate(&provider, simulated.clone()).await.unwrap_err();
    assert!(error.contains("requires a configuration witness"), "unexpected error: {error}");

    let mut witness = witness(&account, Some("secp256k1"));
    witness["approvals"].as_array_mut().unwrap().truncate(1);
    simulated["configurableAccountSimulation"] = witness.clone();
    let error = estimate(&provider, simulated.clone()).await.unwrap_err();
    assert!(error.contains("weight below threshold"), "unexpected error: {error}");

    witness["approvals"][0]["owner"] = serde_json::json!(Address::repeat_byte(0x99));
    simulated["configurableAccountSimulation"] = witness;
    let error = estimate(&provider, simulated).await.unwrap_err();
    assert!(error.contains("signer is not an owner"), "unexpected error: {error}");
}

/// An impersonated configurable account, registered or upgraded, sends under a placeholder
/// `0x05` signature instead of failing on its retired root key, and the transaction uses the
/// gas it is estimated at and the gas its owners' signed transaction uses.
///
/// On a fork of that node, whose blocks hold none of the configuration events, both accounts
/// sign with the configuration their events on the forked chain announce. The registered account
/// sends with a gas limit and the upgraded one is estimated first, so each path looks the
/// configuration up on the forked chain.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t14_impersonate_configurable_account_and_fork() {
    let config = || NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()));
    let (_api, handle) = spawn(config()).await;
    let provider = handle.http_provider();

    // A derived account registers with its first transaction.
    let registered = quorum_accounts().remove(1);
    deal(&provider, registered.address).await;
    send_owner_signed(&provider, &registered, 0, CONFIGURABLE_ACCOUNT_GAS).await.unwrap();

    // A primitive account upgrades in place to two of three owners.
    let root = seeded_key(0x54);
    let (a, b, c) = (seeded_key(0x55), seeded_key(0x56), seeded_key(0x57));
    let salt = keccak256([CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, root.address().as_slice()].concat());
    let upgraded = TestConfigurableAccount {
        address: root.address(),
        config: weighted_config(salt, 1, 2, &[(&a, 1), (&b, 1), (&c, 1)]),
        keys: vec![a, b],
    };
    deal(&provider, root.address()).await;
    let tx = configurable_account_tx(&provider, 0, vec![upgraded.upgrade_call()]).await;
    let signature = root.sign_hash(&tx.signature_hash()).await.unwrap();
    let receipt = send_signed_tempo_tx(&provider, tx, signature.into()).await.unwrap();
    assert!(receipt.status(), "upgradeAccount should succeed: {receipt:?}");

    for account in [&registered, &upgraded] {
        let address = account.address;
        let mut simulated = request(address, 1);
        simulated["configurableAccountSimulation"] = witness(account, Some("secp256k1"));
        let witnessed = estimate(&provider, simulated).await.unwrap();
        let signed = send_owner_signed(&provider, account, 1, witnessed).await.unwrap();

        assert_eq!(send_impersonated(&provider, account, 2, None).await, signed);
        assert_eq!(provider.get_transaction_count(address).await.unwrap(), 3);
    }

    let (_fork_api, fork) = spawn(
        config().with_no_storage_caching(true).with_eth_rpc_url(Some(handle.http_endpoint())),
    )
    .await;
    let provider = fork.http_provider();
    send_impersonated(&provider, &registered, 3, Some(CONFIGURABLE_ACCOUNT_GAS)).await;
    send_impersonated(&provider, &upgraded, 3, None).await;
    for account in [registered, upgraded] {
        assert_eq!(provider.get_transaction_count(account.address).await.unwrap(), 4);
    }
}

/// A configurable account acts as an access key of an EOA (TIP-1111). The EOA authorizes it with
/// key type `configurable`, and its owners approve the keychain V2 hash
/// `keccak256(0x04 || tx_hash || root)`, as Tempo and viem sign it, in a `0x05` signature that a
/// keychain signature wraps for the EOA. The estimate models that quorum from the `keyId` and the
/// account's witness, and the first such transaction registers the account. A quorum over the
/// bare transaction hash, which the account's own transactions carry, does not authorize.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t14_configurable_account_as_access_key() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
    let provider = handle.http_provider();
    let root = handle.dev_accounts().next().unwrap();
    deal(&provider, root).await;
    let account = TestConfigurableAccount::fixture();

    let authorize = authorizeKeyCall {
        keyId: account.address,
        signatureType: KeychainSignatureType::ConfigurableAccount,
        config: KeyRestrictions {
            expiry: u64::MAX,
            enforceLimits: false,
            limits: vec![],
            allowAnyCalls: true,
            allowedCalls: vec![],
        },
    };
    let mut authorization = request(root, 0);
    authorization["gas"] = serde_json::json!(U256::from(T5_PRECOMPILE_GAS));
    authorization["calls"] = serde_json::json!([Call {
        to: TxKind::Call(ACCOUNT_KEYCHAIN_ADDRESS),
        value: U256::ZERO,
        input: authorize.abi_encode().into(),
    }]);
    let hash = provider
        .raw_request::<_, B256>("eth_sendTransaction".into(), (authorization,))
        .await
        .unwrap();
    let receipt =
        PendingTransactionBuilder::new(provider.root().clone(), hash).get_receipt().await.unwrap();
    assert!(receipt.status(), "authorizeKey should succeed: {receipt:?}");

    let mut simulated = request(root, 1);
    simulated["keyId"] = serde_json::json!(account.address);
    simulated["configurableAccountSimulation"] = witness(&account, Some("secp256k1"));
    simulated["configurableAccountSimulation"]["signer"] = serde_json::json!(account.address);
    let gas = estimate(&provider, simulated).await.unwrap();

    let mut tx = configurable_account_tx(&provider, 1, vec![noop()]).await;
    tx.gas_limit = gas;
    let bare = TempoSignature::Keychain(KeychainSignature::new(
        root,
        account.approve(tx.signature_hash()).await,
    ));
    let error = send_signed_tempo_tx(&provider, tx.clone(), bare).await.unwrap_err();
    assert!(error.ends_with("Failed to recover the signer"), "unexpected error: {error}");

    let keychain_hash = KeychainSignature::signing_hash(tx.signature_hash(), root);
    let signature = TempoSignature::Keychain(KeychainSignature::new(
        root,
        account.approve(keychain_hash).await,
    ));
    let receipt = send_signed_tempo_tx(&provider, tx, signature).await.unwrap();
    assert!(receipt.status(), "the access key should authorize: {receipt:?}");
    assert_eq!(receipt.from, root);
    assert_eq!(receipt.gas_used, gas);
    assert_eq!(provider.get_transaction_count(root).await.unwrap(), 2);
    assert_eq!(config_commitment(&provider, account.address).await, VIEM_CA_COMMITMENT_V0);
}
