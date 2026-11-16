//! Gas estimates and impersonation for Tempo configurable accounts (TIP-1107, TIP-1113).
//!
//! Owners have not signed when a transaction is estimated or impersonated, so Anvil models their
//! quorum with placeholder `0x05` approvals, as a Tempo node does for `eth_estimateGas`.

use super::*;
use alloy_provider::PendingTransactionBuilder;

/// A no-op call that every transaction in these tests makes.
const fn noop() -> Call {
    Call { to: TxKind::Call(Address::repeat_byte(0x73)), value: U256::ZERO, input: Bytes::new() }
}

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

async fn deal(provider: &RetryProvider, account: Address) {
    provider
        .raw_request::<_, ()>(
            "anvil_dealTIP20".into(),
            (account, ALPHA_USD, U256::from(1_000_000_000_000u64)),
        )
        .await
        .unwrap();
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
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t14_impersonate_configurable_account() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T14.into()))).await;
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

    for account in [registered, upgraded] {
        let address = account.address;
        let mut simulated = request(address, 1);
        simulated["configurableAccountSimulation"] = witness(&account, Some("secp256k1"));
        let witnessed = estimate(&provider, simulated).await.unwrap();
        let signed = send_owner_signed(&provider, &account, 1, witnessed).await.unwrap();

        provider.raw_request::<_, ()>("anvil_impersonateAccount".into(), (address,)).await.unwrap();
        let impersonated = request(address, 2);
        let gas = estimate(&provider, impersonated.clone()).await.unwrap();
        assert_eq!(gas, signed);
        let hash = provider
            .raw_request::<_, B256>("eth_sendTransaction".into(), (impersonated,))
            .await
            .unwrap();
        let receipt = PendingTransactionBuilder::new(provider.root().clone(), hash)
            .get_receipt()
            .await
            .unwrap();
        assert!(receipt.status(), "impersonated transaction should succeed: {receipt:?}");
        assert_eq!(receipt.from, address);
        assert_eq!(receipt.gas_used, signed);

        let raw = provider
            .raw_request::<_, Bytes>("eth_getRawTransactionByHash".into(), (hash,))
            .await
            .unwrap();
        let TempoTxEnvelope::AA(tx) = TempoTxEnvelope::decode_2718(&mut raw.as_ref()).unwrap()
        else {
            panic!("expected a Tempo transaction");
        };
        let TempoSignature::ConfigurableAccount(signature) = tx.signature() else {
            panic!("expected a 0x05 signature, got {:?}", tx.signature());
        };
        assert_eq!(signature.account(), address);
        assert_eq!(signature.config(), &account.config);
        assert_eq!(provider.get_transaction_count(address).await.unwrap(), 3);
    }
}
