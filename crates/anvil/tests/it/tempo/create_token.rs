//! Chosen-address native TIP-20 creation through Anvil's public RPC.

use super::*;
use tempo_precompiles::{
    tip20::{IRolesAuth, ISSUER_ROLE, ITIP20, roles::DEFAULT_ADMIN_ROLE},
    tip20_factory::ITIP20Factory,
};

const OUSD: Address = address!("0x20c0000000000000000000006a37da5c996874be");
const EURO: Address = address!("0x20c0000000000000000000000000000000001234");

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_tempo_create_token() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let admin = handle.dev_accounts().next().unwrap();
    let recipient = Address::random();
    let amount = U256::from(1_000_000);
    let mut request = token_request(OUSD, admin);
    request["mint"] = serde_json::json!([{ "to": recipient, "amount": amount }]);
    assert_eq!(create_token(&provider, request).await, OUSD);

    assert_eq!(provider.get_transaction_count(OUSD).await.unwrap(), 0);
    assert!(
        ITIP20Factory::new(TIP20_FACTORY_ADDRESS, &provider).isTIP20(OUSD).call().await.unwrap()
    );
    let token = ITIP20::new(OUSD, &provider);
    assert_eq!(token.name().call().await.unwrap(), "OUSD");
    assert_eq!(token.symbol().call().await.unwrap(), "OUSD");
    assert_eq!(token.decimals().call().await.unwrap(), 6);
    assert_eq!(token.currency().call().await.unwrap(), "USD");
    assert_eq!(token.quoteToken().call().await.unwrap(), PATH_USD);
    assert_eq!(token.totalSupply().call().await.unwrap(), amount);
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), amount);
    assert_eq!(token.balanceOf(admin).call().await.unwrap(), U256::ZERO);
    let roles = IRolesAuth::new(OUSD, &provider);
    assert!(roles.hasRole(admin, DEFAULT_ADMIN_ROLE).call().await.unwrap());
    assert!(roles.hasRole(admin, ISSUER_ROLE).call().await.unwrap());

    let mut request = token_request(EURO, admin);
    request["quoteToken"] = serde_json::json!(OUSD);
    assert_eq!(create_token(&provider, request).await, EURO);
    assert!(
        ITIP20Factory::new(TIP20_FACTORY_ADDRESS, &provider).isTIP20(EURO).call().await.unwrap()
    );
    assert_eq!(ITIP20::new(EURO, &provider).quoteToken().call().await.unwrap(), OUSD);

    assert_eq!(ITIP20::new(EURO, &provider).totalSupply().call().await.unwrap(), U256::ZERO);
    assert!(IRolesAuth::new(EURO, &provider).hasRole(admin, ISSUER_ROLE).call().await.unwrap());
    let replacement_admin = Address::random();

    // Imported state can retain initialization storage without its code marker, including
    // genesis PathUSD with its zero quote token.
    assert_eq!(ITIP20::new(PATH_USD, &provider).quoteToken().call().await.unwrap(), Address::ZERO);
    for address in [OUSD, EURO, PATH_USD] {
        let mut imported = api.serialized_state(false).await.unwrap();
        imported.accounts.get_mut(&address).unwrap().code = Bytes::new();
        assert!(api.anvil_load_state(serde_json::to_vec(&imported).unwrap().into()).await.unwrap());
        assert!(provider.get_code_at(address).await.unwrap().is_empty());
        let state = api.anvil_dump_state(None).await.unwrap();
        let mut request = token_request(address, replacement_admin);
        request["quoteToken"] = serde_json::json!(ALPHA_USD_ADDRESS);
        request["mint"] = serde_json::json!([{ "to": replacement_admin, "amount": U256::ONE }]);
        let error = provider
            .raw_request::<_, Address>("anvil_tempoCreateToken".into(), (request,))
            .await
            .unwrap_err();
        let response = error.as_error_resp().unwrap();
        assert_eq!(response.code, 3, "{address}");
        assert_eq!(
            response.as_revert_data(),
            Some(ITIP20Factory::TokenAlreadyExists { token: address }.abi_encode().into()),
            "{address}",
        );
        assert_eq!(api.anvil_dump_state(None).await.unwrap(), state, "{address}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_tempo_create_token_rejects_non_tempo() {
    let (_api, handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let error = provider
        .raw_request::<_, Address>(
            "anvil_tempoCreateToken".into(),
            (token_request(OUSD, handle.dev_accounts().next().unwrap()),),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.as_error_resp().unwrap()).unwrap(),
        serde_json::json!({ "code": -32603, "message": "Not implemented", "data": null }),
    );
    assert!(provider.get_code_at(OUSD).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_tempo_create_token_rejects_invalid_requests() {
    let (api, handle) = spawn(
        NodeConfig::test_tempo().with_no_mining(true).with_hardfork(Some(TempoHardfork::T9.into())),
    )
    .await;
    let provider = handle.http_provider();
    let admin = handle.dev_accounts().next().unwrap();
    let replacement_admin = Address::random();
    let recipient = Address::random();
    let mut euro = token_request(EURO, admin);
    euro["currency"] = serde_json::json!("EUR");
    create_token(&provider, euro.clone()).await;
    euro["name"] = serde_json::json!("Replacement");
    euro["admin"] = serde_json::json!(replacement_admin);
    euro["mint"] = serde_json::json!([{ "to": recipient, "amount": U256::ONE }]);

    let invalid_address = address!("0x1234567890123456789012345678901234567890");
    let mut cases = vec![
        (token_request(invalid_address, admin), ITIP20::InvalidToken {}.abi_encode()),
        (euro, ITIP20Factory::TokenAlreadyExists { token: EURO }.abi_encode()),
    ];
    for quote in [Address::ZERO, address!("0x20c0000000000000000000000000000000002345"), EURO] {
        let mut request = token_request(OUSD, admin);
        request["quoteToken"] = serde_json::json!(quote);
        cases.push((request, ITIP20::InvalidQuoteToken {}.abi_encode()));
    }
    // T9 initialization writes the token's transfer policy into the TIP-403 registry before the
    // second mint fails.
    let mut request = token_request(OUSD, admin);
    request["mint"] = serde_json::json!([
        { "to": recipient, "amount": U256::ONE },
        { "to": Address::random(), "amount": U256::from(u128::MAX) },
    ]);
    cases.push((request, ITIP20::SupplyCapExceeded {}.abi_encode()));

    let state = api.anvil_dump_state(None).await.unwrap();
    for (request, expected) in cases {
        let error = provider
            .raw_request::<_, Address>("anvil_tempoCreateToken".into(), (&request,))
            .await
            .unwrap_err();
        let response = error.as_error_resp().unwrap();
        assert_eq!(response.code, 3, "{request}");
        assert_eq!(response.as_revert_data(), Some(expected.into()), "{request}");
        assert_eq!(api.anvil_dump_state(None).await.unwrap(), state, "{request}");
    }

    let mut request = token_request(OUSD, replacement_admin);
    request["mint"] = serde_json::json!([]);
    assert_eq!(create_token(&provider, request).await, OUSD);
    let token = ITIP20::new(OUSD, &provider);
    assert_eq!(token.totalSupply().call().await.unwrap(), U256::ZERO);
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::ZERO);
    let roles = IRolesAuth::new(OUSD, &provider);
    assert!(!roles.hasRole(admin, ISSUER_ROLE).call().await.unwrap());
    assert!(roles.hasRole(replacement_admin, ISSUER_ROLE).call().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_tempo_create_token_state_lifecycle() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let admin = handle.dev_accounts().next().unwrap();
    let recipient = Address::random();
    let amount = U256::from(1_000_000);
    let initial = api.anvil_dump_state(None).await.unwrap();
    let snapshot = api.evm_snapshot().await.unwrap();
    let mut request = token_request(OUSD, admin);
    request["mint"] = serde_json::json!([{ "to": admin, "amount": amount }]);
    create_token(&provider, request).await;
    let created = api.anvil_dump_state(None).await.unwrap();

    assert!(api.evm_revert(snapshot).await.unwrap());
    assert_eq!(api.anvil_dump_state(None).await.unwrap(), initial);

    assert!(api.anvil_load_state(created).await.unwrap());
    let token = ITIP20::new(OUSD, &provider);
    assert_eq!(token.balanceOf(admin).call().await.unwrap(), amount);
    let roles = IRolesAuth::new(OUSD, &provider);
    assert!(roles.hasRole(admin, ISSUER_ROLE).call().await.unwrap());
    let mut transaction = tempo_call_request(
        admin,
        [Call {
            to: TxKind::Call(OUSD),
            value: U256::ZERO,
            input: ITIP20::transferCall { to: recipient, amount }.abi_encode().into(),
        }],
    );
    transaction["feeToken"] = serde_json::json!(PATH_USD);
    let hash: B256 =
        provider.raw_request("eth_sendTransaction".into(), (transaction,)).await.unwrap();
    api.mine_one().await.unwrap();
    assert!(provider.get_transaction_receipt(hash).await.unwrap().unwrap().status());
    assert_eq!(token.balanceOf(admin).call().await.unwrap(), U256::ZERO);
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), amount);

    api.anvil_reset(None).await.unwrap();
    assert!(provider.get_code_at(OUSD).await.unwrap().is_empty());
    create_token(&provider, token_request(OUSD, recipient)).await;
    assert_eq!(token.totalSupply().call().await.unwrap(), U256::ZERO);
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::ZERO);
    assert!(!roles.hasRole(admin, ISSUER_ROLE).call().await.unwrap());
    assert!(roles.hasRole(recipient, ISSUER_ROLE).call().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_tempo_create_token_preserves_account() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let balance = U256::from(123_456);
    let slot = U256::MAX;
    let value = U256::from(42);
    let admin = handle.dev_accounts().next().unwrap();
    for (address, balance, nonce) in [
        (OUSD, balance, 0),
        (EURO, balance, 7),
        // An account with only storage is empty to the EVM, but its slots must survive.
        (address!("0x20c0000000000000000000000000000000003456"), U256::ZERO, 0),
    ] {
        api.anvil_set_balance(address, balance).await.unwrap();
        api.anvil_set_nonce(address, U256::from(nonce)).await.unwrap();
        api.anvil_set_storage_at(address, slot, value.into()).await.unwrap();
        create_token(&provider, token_request(address, admin)).await;
        assert_eq!(provider.get_balance(address).await.unwrap(), balance);
        assert_eq!(provider.get_transaction_count(address).await.unwrap(), nonce);
        assert_eq!(provider.get_storage_at(address, slot).await.unwrap(), value);
        assert_eq!(ITIP20::new(address, &provider).symbol().call().await.unwrap(), "OUSD");
    }
}

fn token_request(address: Address, admin: Address) -> serde_json::Value {
    serde_json::json!({
        "address": address,
        "name": "OUSD",
        "symbol": "OUSD",
        "currency": "USD",
        "quoteToken": PATH_USD,
        "admin": admin,
    })
}

async fn create_token(provider: &RetryProvider, request: serde_json::Value) -> Address {
    provider.raw_request("anvil_tempoCreateToken".into(), (request,)).await.unwrap()
}
