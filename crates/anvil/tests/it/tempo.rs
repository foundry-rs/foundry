//! Tests for Tempo-specific features in Anvil.
//!
//! This module tests Tempo's payment-native protocol features including:
//! - TIP20 fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD)
//! - Tempo precompiles initialization (sentinel bytecode)
//! - Native value transfer rejection
//! - Basic transaction behavior in Tempo mode

use std::num::NonZeroU64;
#[cfg(feature = "cli")]
use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

#[cfg(feature = "cli")]
use crate::utils::http_provider;
use alloy_consensus::Typed2718;
use alloy_eips::eip2718::Encodable2718;
use alloy_genesis::Genesis;
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address, aliases::U96, keccak256};
use alloy_provider::{Provider, ext::TxPoolApi};
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest, anvil::Forking};
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolEvent, SolValue, sol};
use anvil::{NodeConfig, spawn};
use foundry_evm::core::tempo::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, ITIP20ChannelReserve, PATH_USD_ADDRESS,
    TEMPO_PRECOMPILE_ADDRESSES, TEMPO_TIP20_TOKENS, THETA_USD_ADDRESS,
    active_tempo_precompile_addresses,
};
use tempo_alloy::{primitives::TempoTxEnvelope, rpc::TempoHeaderResponse};
use tempo_hardfork::{
    TempoHardfork,
    constants::gas::{TEMPO_T1_BASE_FEE, TEMPO_T7_BASE_FEE_CAP, TEMPO_T7_BASE_FEE_FLOOR},
};
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, ADDRESS_REGISTRY_ADDRESS, DEFAULT_FEE_TOKEN,
    RECEIVE_POLICY_GUARD_ADDRESS, STABLECOIN_DEX_ADDRESS, TIP_FEE_MANAGER_ADDRESS,
    TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS, TIP403_REGISTRY_ADDRESS,
    receive_policy_guard::{IReceivePolicyGuard, InboundKind},
    tip403_registry::{ALLOW_ALL_POLICY_ID, ITIP403Registry, REJECT_ALL_POLICY_ID},
};
use tempo_primitives::{
    AASigned, TempoHeader, TempoSignature, TempoTransaction,
    transaction::{Call, KeyAuthorization, PrimitiveSignature, SignatureType},
};

const PATH_USD: Address = PATH_USD_ADDRESS;
const ALPHA_USD: Address = ALPHA_USD_ADDRESS;
const BETA_USD: Address = BETA_USD_ADDRESS;
const THETA_USD: Address = THETA_USD_ADDRESS;
const TEMPO_ADMIN: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");
const DEX_MIN_ORDER_AMOUNT: u128 = 100_000_000;

/// Gas limit for TIP20 transfer calls (precompile interactions need more gas).
const TIP20_TRANSFER_GAS: u64 = 300_000;
const T5_PRECOMPILE_GAS: u64 = 10_000_000;

fn assert_tempo_header_fields(header: &TempoHeaderResponse) {
    let inner: &TempoHeader = header.as_ref();
    assert_eq!(header.timestamp_millis, inner.timestamp_millis());
    assert_eq!(inner.general_gas_limit, inner.inner.gas_limit);
    assert_eq!(inner.shared_gas_limit, 0);
    assert_eq!(inner.timestamp_millis_part, 0);
}

#[cfg(feature = "cli")]
struct ChildGuard(Child);

#[cfg(feature = "cli")]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(feature = "cli")]
fn anvil_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_anvil") {
        return PathBuf::from(path);
    }

    std::env::current_exe()
        .expect("test executable path")
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/debug directory")
        .join("anvil")
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_tempo_header_by_number() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    api.mine_one().await;

    let provider = handle.http_provider();
    for number in ["0x1", "pending"] {
        let header: Option<TempoHeaderResponse> =
            provider.client().request("eth_getHeaderByNumber", (number,)).await.unwrap();
        assert_tempo_header_fields(&header.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_fork_detects_hardfork_from_fork_timestamp() {
    use tempo_hardfork::TempoHardfork;

    let fork_timestamp = TempoHardfork::T3.mainnet_activation_timestamp().unwrap();
    let (_source_api, source_handle) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(4217u64))
            .with_genesis_timestamp(Some(fork_timestamp)),
    )
    .await;

    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_eth_rpc_url(Some(source_handle.http_endpoint()))).await;

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.hard_fork, "T3");

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, TIP_FEE_MANAGER_ADDRESS);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_reset_to_fork_uses_fee_manager_beneficiary() {
    let (_source_api, source_handle) = spawn(NodeConfig::test()).await;

    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(source_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, TIP_FEE_MANAGER_ADDRESS);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_reset_to_fork_preserves_explicit_coinbase() {
    let (_source_api, source_handle) = spawn(NodeConfig::test()).await;
    let custom_coinbase = address!("0x1111111111111111111111111111111111111111");

    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    api.anvil_set_coinbase(custom_coinbase).await.unwrap();
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(source_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, custom_coinbase);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_fork_with_default_genesis_uses_fee_manager_beneficiary() {
    let (_source_api, source_handle) = spawn(NodeConfig::test()).await;

    let (api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_eth_rpc_url(Some(source_handle.http_endpoint()))
            .with_genesis(Some(Genesis::default())),
    )
    .await;

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, TIP_FEE_MANAGER_ADDRESS);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_fork_with_loaded_zero_beneficiary_state_uses_fee_manager_beneficiary() {
    let (source_api, source_handle) = spawn(NodeConfig::test()).await;
    let mut state = source_api.serialized_state(false).await.unwrap();
    state.block.as_mut().unwrap().beneficiary = Address::ZERO;

    let (api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_eth_rpc_url(Some(source_handle.http_endpoint()))
            .with_init_state(Some(state)),
    )
    .await;

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, TIP_FEE_MANAGER_ADDRESS);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_fork_runtime_load_state_uses_fee_manager_beneficiary() {
    let (source_api, source_handle) = spawn(NodeConfig::test()).await;
    let mut state = source_api.serialized_state(false).await.unwrap();
    state.block.as_mut().unwrap().beneficiary = Address::ZERO;

    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_eth_rpc_url(Some(source_handle.http_endpoint()))).await;

    api.anvil_load_state(Bytes::from(serde_json::to_vec(&state).unwrap())).await.unwrap();

    api.mine_one().await;
    let latest_block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest_block.header.beneficiary, TIP_FEE_MANAGER_ADDRESS);
}

sol! {
    #[sol(rpc)]
    interface IFeeManagerRpc {
        function userTokens(address user) external view returns (address);
        function validatorTokens(address validator) external view returns (address);
        function collectedFees(address validator, address token) external view returns (uint256);

        struct Pool {
            uint128 reserveUserToken;
            uint128 reserveValidatorToken;
        }

        function getPool(address userToken, address validatorToken) external view returns (Pool memory);
        function getPoolId(address userToken, address validatorToken) external pure returns (bytes32);
        function totalSupply(bytes32 poolId) external view returns (uint256);
        function mint(address userToken, address validatorToken, uint256 amountValidatorToken, address to) external returns (uint256 liquidity);
    }
}

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }
}

sol! {
    #[sol(rpc)]
    interface IAddressRegistryRpc {
        function isImplicitlyApproved(address precompile) external view returns (bool);
    }
}

sol! {
    #[sol(rpc)]
    interface IAccountKeychainT5Rpc {
        function burnKeyAuthorizationWitness(bytes32 witness) external;
        function isKeyAuthorizationWitnessBurned(address account, bytes32 witness) external view returns (bool);
    }
}

sol! {
    #[sol(rpc)]
    interface IStablecoinDexT5Rpc {
        struct Order {
            uint128 orderId;
            address maker;
            bytes32 bookKey;
            bool isBid;
            int16 tick;
            uint128 amount;
            uint128 remaining;
            uint128 prev;
            uint128 next;
            bool isFlip;
            int16 flipTick;
        }

        function place(address token, uint128 amount, bool isBid, int16 tick) external returns (uint128 orderId);
        function placeFlip(address token, uint128 amount, bool isBid, int16 tick, int16 flipTick) external returns (uint128 orderId);
        function swapExactAmountIn(address tokenIn, address tokenOut, uint128 amountIn, uint128 minAmountOut) external returns (uint128 amountOut);
        function getOrder(uint128 orderId) external view returns (Order memory);

        event OrderFlipped(uint128 indexed orderId, address indexed maker, address indexed token, uint128 amount, bool isBid, int16 tick, int16 flipTick);
    }
}

sol! {
    #[sol(rpc)]
    interface ITIP20T5Rpc {
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function mint(address to, uint256 amount) external;
        function grantRole(bytes32 role, address account) external;
        function ISSUER_ROLE() external view returns (bytes32);
        function logoURI() external view returns (string memory);
        function setLogoURI(string memory logoURI) external;
    }
}

sol! {
    #[sol(rpc)]
    #[allow(clippy::too_many_arguments)]
    interface ITIP20FactoryT5Rpc {
        function createToken(
            string memory name,
            string memory symbol,
            string memory currency,
            address quoteToken,
            address admin,
            bytes32 salt,
            string memory logoURI
        ) external returns (address);
        function getTokenAddress(address sender, bytes32 salt) external pure returns (address);
    }
}

sol! {
    #[sol(rpc)]
    #[allow(clippy::too_many_arguments)]
    interface ITIP20ChannelReserveT5Rpc {
        struct ChannelDescriptor {
            address payer;
            address payee;
            address operator;
            address token;
            bytes32 salt;
            address authorizedSigner;
            bytes32 expiringNonceHash;
        }

        struct ChannelState {
            uint96 settled;
            uint96 deposit;
            uint32 closeRequestedAt;
        }

        function open(
            address payee,
            address operator,
            address token,
            uint96 deposit,
            bytes32 salt,
            address authorizedSigner
        ) external returns (bytes32 channelId);
        function computeChannelId(
            address payer,
            address payee,
            address operator,
            address token,
            bytes32 salt,
            address authorizedSigner,
            bytes32 expiringNonceHash
        ) external pure returns (bytes32);
        function getChannelState(bytes32 channelId) external view returns (ChannelState memory);
        function getVoucherDigest(bytes32 channelId, uint96 cumulativeAmount) external view returns (bytes32);
        function domainSeparator() external view returns (bytes32);

        event ChannelOpened(
            bytes32 indexed channelId,
            address indexed payer,
            address indexed payee,
            address operator,
            address token,
            address authorizedSigner,
            bytes32 salt,
            bytes32 expiringNonceHash,
            uint96 deposit
        );
    }
}

// ============================================================================
// Tempo Genesis: Precompile Initialization
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_precompiles_have_code() {
    use tempo_hardfork::TempoHardfork;

    let (api, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;

    assert!(
        TEMPO_PRECOMPILE_ADDRESSES.contains(&TIP20_CHANNEL_RESERVE_ADDRESS),
        "T5 channel reserve should be tracked as a Tempo precompile"
    );

    // Tempo precompiles should have sentinel bytecode (0xef)
    for addr in active_tempo_precompile_addresses(TempoHardfork::T5) {
        let code = api.get_code(addr, None).await.unwrap();
        assert!(!code.is_empty(), "Precompile {addr} should have code");
    }

    // All TIP20 token addresses should also have code
    for addr in TEMPO_TIP20_TOKENS {
        let code = api.get_code(*addr, None).await.unwrap();
        assert!(!code.is_empty(), "Token {addr} should have code deployed");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_deal_tip20() {
    let (_api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let recipient = Address::random();
    let token = IERC20::new(ALPHA_USD, &provider);
    let supply_before = token.totalSupply().call().await.unwrap();

    provider
        .raw_request::<_, ()>("anvil_dealTIP20".into(), (recipient, ALPHA_USD, U256::from(100)))
        .await
        .unwrap();
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::from(100));
    assert_eq!(token.totalSupply().call().await.unwrap(), supply_before);

    provider
        .raw_request::<_, ()>("anvil_dealTIP20".into(), (recipient, ALPHA_USD, U256::from(40)))
        .await
        .unwrap();
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::from(40));
    assert_eq!(token.totalSupply().call().await.unwrap(), supply_before);
    assert_eq!(provider.txpool_status().await.unwrap().pending, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_deal_erc20_supports_tip20() {
    let (_api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let recipient = Address::random();
    let token = IERC20::new(ALPHA_USD, &provider);
    let supply_before = token.totalSupply().call().await.unwrap();

    provider
        .raw_request::<_, ()>("anvil_dealERC20".into(), (recipient, ALPHA_USD, U256::from(100)))
        .await
        .unwrap();
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::from(100));
    assert_eq!(token.totalSupply().call().await.unwrap(), supply_before);

    provider
        .raw_request::<_, ()>("anvil_dealERC20".into(), (recipient, ALPHA_USD, U256::from(40)))
        .await
        .unwrap();
    assert_eq!(token.balanceOf(recipient).call().await.unwrap(), U256::from(40));
    assert_eq!(token.totalSupply().call().await.unwrap(), supply_before);
    assert_eq!(provider.txpool_status().await.unwrap().pending, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_deal_tip20_rejects_invalid_token() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();
    let result: std::result::Result<(), _> = provider
        .raw_request("anvil_dealTIP20".into(), (Address::random(), Address::random(), U256::ONE))
        .await;

    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_deal_tip20_rejects_non_tempo_node() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let result: std::result::Result<(), _> = provider
        .raw_request("anvil_dealTIP20".into(), (Address::random(), ALPHA_USD, U256::ONE))
        .await;

    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pre_t5_channel_reserve_has_no_sentinel_code() {
    let (api, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T4.into()))).await;

    let code = api.get_code(TIP20_CHANNEL_RESERVE_ADDRESS, None).await.unwrap();
    assert!(code.is_empty(), "pre-T5 channel reserve address should not have sentinel code");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_config_reports_channel_reserve_precompile() {
    let (api, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;

    let config = api.config().unwrap();

    assert_eq!(
        config.current.precompiles.get("TIP20ChannelReserve"),
        Some(&TIP20_CHANNEL_RESERVE_ADDRESS)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_config_filters_hardfork_gated_precompiles() {
    let (api_t4, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T4.into()))).await;
    let config_t4 = api_t4.config().unwrap();
    assert!(!config_t4.current.precompiles.contains_key("TIP20ChannelReserve"));
    assert!(!config_t4.current.precompiles.contains_key("ReceivePolicyGuard"));

    let (api_t5, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let config_t5 = api_t5.config().unwrap();
    assert_eq!(
        config_t5.current.precompiles.get("TIP20ChannelReserve"),
        Some(&TIP20_CHANNEL_RESERVE_ADDRESS)
    );
    assert!(!config_t5.current.precompiles.contains_key("ReceivePolicyGuard"));

    let (api_t6, _handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let config_t6 = api_t6.config().unwrap();
    assert_eq!(
        config_t6.current.precompiles.get("ReceivePolicyGuard"),
        Some(&RECEIVE_POLICY_GUARD_ADDRESS)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t6_receive_policy_blocks_and_claims_transfer() {
    let (_api_t5, handle_t5) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider_t5 = handle_t5.http_provider();
    let registry_t5 = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, &provider_t5);
    let guard_t5 = IReceivePolicyGuard::new(RECEIVE_POLICY_GUARD_ADDRESS, &provider_t5);

    assert!(
        registry_t5.receivePolicy(Address::ZERO).call().await.is_err(),
        "receive-policy selectors should be unavailable before T6"
    );
    assert!(
        guard_t5.balanceOf(Bytes::default()).call().await.is_err(),
        "ReceivePolicyGuard should be unavailable before T6"
    );

    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let receiver = accounts[1];
    let recovery = accounts[2];
    let claim_target = accounts[3];
    let amount = U256::from(123_456u64);

    let code = api.get_code(RECEIVE_POLICY_GUARD_ADDRESS, None).await.unwrap();
    assert!(!code.is_empty(), "ReceivePolicyGuard should have sentinel code at T6");

    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, &provider);
    let set_policy_tx = TransactionRequest::default()
        .from(receiver)
        .to(TIP403_REGISTRY_ADDRESS)
        .with_input(
            registry
                .setReceivePolicy(REJECT_ALL_POLICY_ID, ALLOW_ALL_POLICY_ID, recovery)
                .calldata()
                .clone(),
        )
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let receipt = provider
        .send_transaction(WithOtherFields::new(set_policy_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "setReceivePolicy should succeed at T6");

    let validation =
        registry.validateReceivePolicy(PATH_USD, sender, receiver).call().await.unwrap();
    assert!(!validation.authorized, "REJECT_ALL sender policy should hold the transfer");
    assert_eq!(validation.blockedReason, ITIP403Registry::BlockedReason::RECEIVE_POLICY);

    let token = ITIP20T5Rpc::new(PATH_USD, &provider);
    let receiver_balance_before = token.balanceOf(receiver).call().await.unwrap();
    let guard_balance_before = token.balanceOf(RECEIVE_POLICY_GUARD_ADDRESS).call().await.unwrap();
    let claim_target_balance_before = token.balanceOf(claim_target).call().await.unwrap();

    let transfer_tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(token.transfer(receiver, amount).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let transfer_receipt = provider
        .send_transaction(WithOtherFields::new(transfer_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(transfer_receipt.status(), "blocked transfers should still succeed");

    let blocked = transfer_receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| IReceivePolicyGuard::TransferBlocked::decode_log(&log.inner).ok())
        .expect("blocked transfer should emit TransferBlocked");
    let decoded = IReceivePolicyGuard::ClaimReceiptV1::abi_decode(&blocked.receipt).unwrap();

    assert_eq!(blocked.token, PATH_USD);
    assert_eq!(blocked.receiver, receiver);
    assert_eq!(blocked.amount, amount);
    assert_eq!(blocked.receiptVersion, 1);
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.token, PATH_USD);
    assert_eq!(decoded.recoveryAuthority, recovery);
    assert_eq!(decoded.originator, sender);
    assert_eq!(decoded.recipient, receiver);
    assert_eq!(decoded.blockedNonce, blocked.blockedNonce);
    assert_eq!(decoded.blockedReason, ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8);
    assert_eq!(decoded.kind, InboundKind::TRANSFER);
    assert_eq!(decoded.memo, B256::ZERO);

    let guard = IReceivePolicyGuard::new(RECEIVE_POLICY_GUARD_ADDRESS, &provider);
    assert_eq!(guard.balanceOf(blocked.receipt.clone()).call().await.unwrap(), amount);
    assert_eq!(token.balanceOf(receiver).call().await.unwrap(), receiver_balance_before);
    assert_eq!(
        token.balanceOf(RECEIVE_POLICY_GUARD_ADDRESS).call().await.unwrap(),
        guard_balance_before + amount
    );

    let claim_tx = TransactionRequest::default()
        .from(recovery)
        .to(RECEIVE_POLICY_GUARD_ADDRESS)
        .with_input(guard.claim(claim_target, blocked.receipt.clone()).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let claim_receipt = provider
        .send_transaction(WithOtherFields::new(claim_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(claim_receipt.status(), "recovery authority should be able to claim held funds");

    let claimed = claim_receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| IReceivePolicyGuard::ReceiptClaimed::decode_log(&log.inner).ok())
        .expect("claim should emit ReceiptClaimed");
    assert_eq!(claimed.token, PATH_USD);
    assert_eq!(claimed.receiver, receiver);
    assert_eq!(claimed.blockedNonce, decoded.blockedNonce);
    assert_eq!(claimed.caller, recovery);
    assert_eq!(claimed.to, claim_target);
    assert_eq!(claimed.amount, amount);

    assert_eq!(guard.balanceOf(blocked.receipt.clone()).call().await.unwrap(), U256::ZERO);
    assert_eq!(
        token.balanceOf(claim_target).call().await.unwrap(),
        claim_target_balance_before + amount
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_channel_reserve_compute_channel_id_call() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let reserve = ITIP20ChannelReserve::new(TIP20_CHANNEL_RESERVE_ADDRESS, &provider);

    let payer = address!("0x0000000000000000000000000000000000000101");
    let payee = address!("0x0000000000000000000000000000000000000202");
    let operator = address!("0x0000000000000000000000000000000000000303");
    let salt = B256::with_last_byte(0x42);
    let authorized_signer = address!("0x0000000000000000000000000000000000000404");
    let expiring_nonce_hash = B256::with_last_byte(0x99);

    let channel_id = reserve
        .computeChannelId(
            payer,
            payee,
            operator,
            PATH_USD,
            salt,
            authorized_signer,
            expiring_nonce_hash,
        )
        .call()
        .await
        .unwrap();
    let expected = keccak256(
        (
            payer,
            payee,
            operator,
            PATH_USD,
            salt,
            authorized_signer,
            expiring_nonce_hash,
            TIP20_CHANNEL_RESERVE_ADDRESS,
            U256::from(provider.get_chain_id().await.unwrap()),
        )
            .abi_encode(),
    );

    assert_eq!(channel_id, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_implicit_approvals_are_hardfork_gated() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T4.into()))).await;
    let provider = handle.http_provider();
    let registry = IAddressRegistryRpc::new(ADDRESS_REGISTRY_ADDRESS, &provider);

    let pre_t5_result = registry.isImplicitlyApproved(TIP20_CHANNEL_RESERVE_ADDRESS).call().await;
    assert!(pre_t5_result.is_err(), "T5 implicit-approval selector should be unavailable at T4");

    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let registry = IAddressRegistryRpc::new(ADDRESS_REGISTRY_ADDRESS, &provider);

    let code = api.get_code(TIP20_CHANNEL_RESERVE_ADDRESS, None).await.unwrap();
    assert!(!code.is_empty(), "T5 channel reserve precompile should be registered");

    for approved in [TIP_FEE_MANAGER_ADDRESS, STABLECOIN_DEX_ADDRESS, TIP20_CHANNEL_RESERVE_ADDRESS]
    {
        assert!(
            registry.isImplicitlyApproved(approved).call().await.unwrap(),
            "{approved} should be implicitly approved at T5"
        );
    }

    let random_address = Address::random();
    assert!(
        !registry.isImplicitlyApproved(random_address).call().await.unwrap(),
        "arbitrary addresses should not be implicitly approved"
    );
}

#[cfg(feature = "cli")]
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_cli_tempo_t5_hardfork_precompile_smoke() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let port_arg = port.to_string();

    let mut child = ChildGuard(
        Command::new(anvil_binary())
            .args([
                "--network",
                "tempo",
                "--hardfork",
                "tempo:T5",
                "--host",
                "127.0.0.1",
                "--port",
                &port_arg,
                "-q",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn anvil --hardfork tempo:T5"),
    );

    let endpoint = format!("http://127.0.0.1:{port}");
    let provider = http_provider(&endpoint);
    let mut ready = false;
    for _ in 0..100 {
        if provider.get_chain_id().await.is_ok() {
            ready = true;
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("anvil exited before serving RPC: {status}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "anvil --hardfork tempo:T5 should start serving RPC");

    let registry = IAddressRegistryRpc::new(ADDRESS_REGISTRY_ADDRESS, &provider);
    assert!(registry.isImplicitlyApproved(TIP20_CHANNEL_RESERVE_ADDRESS).call().await.unwrap());

    let reserve = ITIP20ChannelReserveT5Rpc::new(TIP20_CHANNEL_RESERVE_ADDRESS, &provider);
    assert_ne!(reserve.domainSeparator().call().await.unwrap(), B256::ZERO);
}

#[cfg(feature = "cli")]
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_cli_tempo_t6_hardfork_receive_policy_guard_smoke() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let port_arg = port.to_string();

    let mut child = ChildGuard(
        Command::new(anvil_binary())
            .args([
                "--network",
                "tempo",
                "--hardfork",
                "tempo:T6",
                "--host",
                "127.0.0.1",
                "--port",
                &port_arg,
                "-q",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn anvil --hardfork tempo:T6"),
    );

    let endpoint = format!("http://127.0.0.1:{port}");
    let provider = http_provider(&endpoint);
    let mut ready = false;
    for _ in 0..100 {
        if provider.get_chain_id().await.is_ok() {
            ready = true;
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("anvil exited before serving RPC: {status}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "anvil --hardfork tempo:T6 should start serving RPC");

    let receipt = IReceivePolicyGuard::ClaimReceiptV1::new(
        PATH_USD,
        address!("0x0000000000000000000000000000000000000002"),
        address!("0x0000000000000000000000000000000000000003"),
        address!("0x0000000000000000000000000000000000000004"),
        1,
        1,
        ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8,
        InboundKind::TRANSFER,
        B256::ZERO,
    )
    .abi_encode()
    .into();
    let guard = IReceivePolicyGuard::new(RECEIVE_POLICY_GUARD_ADDRESS, &provider);
    assert_eq!(guard.balanceOf(receipt).call().await.unwrap(), U256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_tip20_logo_uri_validation_and_update() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    api.anvil_set_balance(TEMPO_ADMIN, U256::MAX).await.unwrap();
    api.anvil_impersonate_account(TEMPO_ADMIN).await.unwrap();

    let token = ITIP20T5Rpc::new(PATH_USD, &provider);
    assert_eq!(token.logoURI().call().await.unwrap(), "");

    let logo_uri = "https://example.com/pathusd.png".to_string();
    let tx = TransactionRequest::default()
        .from(TEMPO_ADMIN)
        .to(PATH_USD)
        .with_input(token.setLogoURI(logo_uri.clone()).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);

    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "setLogoURI should succeed for a valid URI");
    assert_eq!(token.logoURI().call().await.unwrap(), logo_uri);

    let logo_event_topic = alloy_primitives::keccak256(b"LogoURIUpdated(address,string)");
    assert!(
        receipt
            .inner
            .logs()
            .iter()
            .any(|log| log.address() == PATH_USD && log.topics()[0] == logo_event_topic),
        "setLogoURI should emit LogoURIUpdated"
    );

    let invalid_tx = TransactionRequest::default()
        .from(TEMPO_ADMIN)
        .to(PATH_USD)
        .with_input(
            token.setLogoURI("ftp://example.com/pathusd.png".to_string()).calldata().clone(),
        )
        .with_gas_limit(T5_PRECOMPILE_GAS);
    assert!(
        provider.call(WithOtherFields::new(invalid_tx)).await.is_err(),
        "unsupported logoURI schemes should revert"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_stablecoin_dex_allows_same_tick_flip_order() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T4.into()))).await;
    let provider = handle.http_provider();
    let sender = handle.dev_accounts().next().unwrap();
    let dex = IStablecoinDexT5Rpc::new(STABLECOIN_DEX_ADDRESS, &provider);

    let tick = 100i16;
    let t4_call = dex.placeFlip(ALPHA_USD, DEX_MIN_ORDER_AMOUNT, true, tick, tick);
    let t4_tx = TransactionRequest::default()
        .from(sender)
        .to(STABLECOIN_DEX_ADDRESS)
        .with_input(t4_call.calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    assert!(
        provider.call(WithOtherFields::new(t4_tx)).await.is_err(),
        "same-tick flip orders should be rejected before T5"
    );

    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let taker = accounts[1];
    let dex = IStablecoinDexT5Rpc::new(STABLECOIN_DEX_ADDRESS, &provider);

    let t5_call = dex.placeFlip(ALPHA_USD, DEX_MIN_ORDER_AMOUNT, true, tick, tick);
    let t5_tx = TransactionRequest::default()
        .from(sender)
        .to(STABLECOIN_DEX_ADDRESS)
        .with_input(t5_call.calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    provider
        .call(WithOtherFields::new(t5_tx.clone()))
        .await
        .expect("same-tick flip order should be callable at T5");

    let receipt = provider
        .send_transaction(WithOtherFields::new(t5_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "same-tick flip order should execute at T5 without approval");

    let order = dex.getOrder(1).call().await.unwrap();
    assert_eq!(order.orderId, 1);
    assert_eq!(order.maker, sender);
    assert!(order.isBid);
    assert!(order.isFlip);
    assert_eq!(order.tick, tick);
    assert_eq!(order.flipTick, tick);
    assert_eq!(order.remaining, DEX_MIN_ORDER_AMOUNT);

    let fill_call = dex.swapExactAmountIn(ALPHA_USD, PATH_USD, DEX_MIN_ORDER_AMOUNT, 0);
    let fill_tx = TransactionRequest::default()
        .from(taker)
        .to(STABLECOIN_DEX_ADDRESS)
        .with_input(fill_call.calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let receipt = provider
        .send_transaction(WithOtherFields::new(fill_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "opposite-side order should fill and flip the T5 order");

    let order_flipped_topic = alloy_primitives::keccak256(
        b"OrderFlipped(uint128,address,address,uint128,bool,int16,int16)",
    );
    let flipped_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| {
            log.address() == STABLECOIN_DEX_ADDRESS && log.topics()[0] == order_flipped_topic
        })
        .expect("fill should emit OrderFlipped");
    assert_eq!(
        flipped_log.topics()[1],
        B256::from(U256::from(1u64).to_be_bytes::<32>()),
        "OrderFlipped should preserve the original order ID"
    );

    let flipped_order = dex.getOrder(1).call().await.unwrap();
    assert_eq!(flipped_order.orderId, 1);
    assert_eq!(flipped_order.maker, sender);
    assert!(!flipped_order.isBid, "filled bid flip order should become an ask");
    assert!(flipped_order.isFlip);
    assert_eq!(flipped_order.tick, tick);
    assert_eq!(flipped_order.flipTick, tick);
    assert_eq!(flipped_order.remaining, DEX_MIN_ORDER_AMOUNT);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_key_authorization_witness_burn_flow() {
    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T4.into()))).await;
    let provider = handle.http_provider();
    let account = handle.dev_accounts().next().unwrap();
    let keychain = IAccountKeychainT5Rpc::new(ACCOUNT_KEYCHAIN_ADDRESS, &provider);
    let witness = B256::repeat_byte(0x53);

    let pre_t5_result = keychain.isKeyAuthorizationWitnessBurned(account, witness).call().await;
    assert!(pre_t5_result.is_err(), "witness check selector should be unavailable at T4");

    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let account = handle.dev_accounts().next().unwrap();
    let keychain = IAccountKeychainT5Rpc::new(ACCOUNT_KEYCHAIN_ADDRESS, &provider);

    let chain_id = provider.get_chain_id().await.unwrap();
    let signer = dev_key(0);
    assert_eq!(signer.address(), account);
    let access_key = PrivateKeySigner::random();
    let unsigned_without_witness =
        KeyAuthorization::unrestricted(chain_id, SignatureType::Secp256k1, access_key.address());
    let authorization = unsigned_without_witness.clone().with_witness(witness);
    assert_eq!(authorization.witness(), Some(witness));
    assert_ne!(
        authorization.signature_hash(),
        unsigned_without_witness.signature_hash(),
        "witness must be part of the key authorization signing digest"
    );
    assert_ne!(authorization.signature_hash(), B256::ZERO);
    assert!(!keychain.isKeyAuthorizationWitnessBurned(account, witness).call().await.unwrap());

    let calldata = keychain.burnKeyAuthorizationWitness(witness).calldata().clone();
    let base_fee = provider.get_gas_price().await.unwrap();
    let key_auth_signature = signer.sign_hash(&authorization.signature_hash()).await.unwrap();
    let signed_key_auth =
        authorization.into_signed(PrimitiveSignature::Secp256k1(key_auth_signature));
    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: T5_PRECOMPILE_GAS,
        calls: vec![Call {
            to: TxKind::Call(ACCOUNT_KEYCHAIN_ADDRESS),
            value: U256::ZERO,
            input: calldata,
        }],
        access_list: Default::default(),
        nonce_key: U256::ZERO,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: Some(signed_key_auth),
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let receipt =
        provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status(), "burnKeyAuthorizationWitness should succeed at T5");

    assert!(keychain.isKeyAuthorizationWitnessBurned(account, witness).call().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_tip20_channel_reserve_basic_views() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let payer = accounts[0];
    let payee = accounts[1];
    let reserve = ITIP20ChannelReserveT5Rpc::new(TIP20_CHANNEL_RESERVE_ADDRESS, &provider);

    let code = api.get_code(TIP20_CHANNEL_RESERVE_ADDRESS, None).await.unwrap();
    assert!(!code.is_empty(), "channel reserve should have sentinel code");

    let salt = B256::repeat_byte(0x34);
    let expiring_nonce_hash = B256::repeat_byte(0x35);
    let channel_id = reserve
        .computeChannelId(payer, payee, Address::ZERO, PATH_USD, salt, payer, expiring_nonce_hash)
        .call()
        .await
        .unwrap();

    let state = reserve.getChannelState(channel_id).call().await.unwrap();
    assert_eq!(state.settled, 0);
    assert_eq!(state.deposit, 0);
    assert_eq!(state.closeRequestedAt, 0);
    assert_ne!(reserve.domainSeparator().call().await.unwrap(), B256::ZERO);
    assert_ne!(
        reserve.getVoucherDigest(channel_id, U96::from(1)).call().await.unwrap(),
        B256::ZERO
    );

    let token = ITIP20T5Rpc::new(PATH_USD, &provider);
    let payer_balance_before = token.balanceOf(payer).call().await.unwrap();
    let reserve_balance_before =
        token.balanceOf(TIP20_CHANNEL_RESERVE_ADDRESS).call().await.unwrap();
    let deposit_amount = U256::from(1_000_000u64);
    let open_call = reserve.open(
        payee,
        Address::ZERO,
        PATH_USD,
        U96::from(1_000_000u64),
        B256::repeat_byte(0x36),
        Address::ZERO,
    );

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();
    let signer = dev_key(0);
    assert_eq!(signer.address(), payer);
    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: T5_PRECOMPILE_GAS,
        calls: vec![Call {
            to: TxKind::Call(TIP20_CHANNEL_RESERVE_ADDRESS),
            value: U256::ZERO,
            input: open_call.calldata().clone(),
        }],
        access_list: Default::default(),
        nonce_key: U256::MAX,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(block.header.timestamp + 25),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let receipt =
        provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status(), "channel reserve open should succeed at T5");

    let channel_opened_topic = alloy_primitives::keccak256(
        b"ChannelOpened(bytes32,address,address,address,address,address,bytes32,bytes32,uint96)",
    );
    let opened_log = receipt
        .inner
        .logs()
        .iter()
        .find(|log| {
            log.address() == TIP20_CHANNEL_RESERVE_ADDRESS
                && log.topics()[0] == channel_opened_topic
        })
        .expect("open should emit ChannelOpened");
    let opened_channel_id = opened_log.topics()[1];
    let opened_state = reserve.getChannelState(opened_channel_id).call().await.unwrap();
    assert_eq!(opened_state.settled, 0);
    assert_eq!(opened_state.deposit, U96::from(1_000_000u64));
    assert_eq!(opened_state.closeRequestedAt, 0);

    let payer_balance_after = token.balanceOf(payer).call().await.unwrap();
    let reserve_balance_after =
        token.balanceOf(TIP20_CHANNEL_RESERVE_ADDRESS).call().await.unwrap();
    assert_eq!(payer_balance_before - payer_balance_after, deposit_amount);
    assert_eq!(reserve_balance_after - reserve_balance_before, deposit_amount);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t5_fee_amm_two_hop_route_for_local_fees() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let validator = accounts[3];
    let recipient = accounts[1];

    api.anvil_set_coinbase(validator).await.unwrap();
    api.anvil_set_validator_fee_token(validator, BETA_USD).await.unwrap();

    let factory = ITIP20FactoryT5Rpc::new(TIP20_FACTORY_ADDRESS, &provider);
    let salt = B256::repeat_byte(0xa5);
    let fee_token = factory.getTokenAddress(sender, salt).call().await.unwrap();

    let create_tx = TransactionRequest::default()
        .from(sender)
        .to(TIP20_FACTORY_ADDRESS)
        .with_input(
            factory
                .createToken(
                    "RouteUSD".to_string(),
                    "RouteUSD".to_string(),
                    "USD".to_string(),
                    PATH_USD,
                    sender,
                    salt,
                    "https://example.com/routeusd.png".to_string(),
                )
                .calldata()
                .clone(),
        )
        .with_gas_limit(T5_PRECOMPILE_GAS);
    provider
        .call(WithOtherFields::new(create_tx.clone()))
        .await
        .expect("T5 factory createToken with logoURI should be callable");
    let receipt = provider
        .send_transaction(WithOtherFields::new(create_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "T5 factory createToken with logoURI should succeed");

    let token = ITIP20T5Rpc::new(fee_token, &provider);
    let issuer_role = token.ISSUER_ROLE().call().await.unwrap();
    let grant_issuer_tx = TransactionRequest::default()
        .from(sender)
        .to(fee_token)
        .with_input(token.grantRole(issuer_role, sender).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let receipt = provider
        .send_transaction(WithOtherFields::new(grant_issuer_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "token admin should be able to grant ISSUER_ROLE");

    let mint_tx = TransactionRequest::default()
        .from(sender)
        .to(fee_token)
        .with_input(token.mint(sender, U256::from(u64::MAX)).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);
    let receipt = provider
        .send_transaction(WithOtherFields::new(mint_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "new TIP-20 fee token mint should succeed");

    let fee_manager = IFeeManagerRpc::new(TIP_FEE_MANAGER_ADDRESS, &provider);
    let direct_pool_before = fee_manager.getPool(fee_token, BETA_USD).call().await.unwrap();
    assert_eq!(direct_pool_before.reserveUserToken, 0);
    assert_eq!(direct_pool_before.reserveValidatorToken, 0);

    let liquidity_amount = U256::from(100_000_000_000_000_000u64);
    for (user_token, validator_token) in [(fee_token, PATH_USD), (PATH_USD, BETA_USD)] {
        let mint_liquidity_tx = TransactionRequest::default()
            .from(sender)
            .to(TIP_FEE_MANAGER_ADDRESS)
            .with_input(
                fee_manager
                    .mint(user_token, validator_token, liquidity_amount, sender)
                    .calldata()
                    .clone(),
            )
            .with_gas_limit(T5_PRECOMPILE_GAS);
        let receipt = provider
            .send_transaction(WithOtherFields::new(mint_liquidity_tx))
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(receipt.status(), "FeeAMM liquidity mint should succeed");
    }
    api.anvil_set_fee_token(sender, fee_token).await.unwrap();

    let first_hop_pool_before = fee_manager.getPool(fee_token, PATH_USD).call().await.unwrap();
    let second_hop_pool_before = fee_manager.getPool(PATH_USD, BETA_USD).call().await.unwrap();
    let collected_before = fee_manager.collectedFees(validator, BETA_USD).call().await.unwrap();
    let transfer_tx = TransactionRequest::default()
        .from(sender)
        .to(fee_token)
        .with_input(token.transfer(recipient, U256::from(1000)).calldata().clone())
        .with_gas_limit(T5_PRECOMPILE_GAS);

    let receipt = provider
        .send_transaction(WithOtherFields::new(transfer_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "local fee payment should route through the T5 two-hop FeeAMM path");

    let collected_after = fee_manager.collectedFees(validator, BETA_USD).call().await.unwrap();
    assert!(
        collected_after > collected_before,
        "validator should receive BETA fees through feeToken -> PATH -> BETA"
    );

    let direct_pool_after = fee_manager.getPool(fee_token, BETA_USD).call().await.unwrap();
    assert_eq!(direct_pool_after.reserveUserToken, 0);
    assert_eq!(direct_pool_after.reserveValidatorToken, 0);

    let first_hop_pool_after = fee_manager.getPool(fee_token, PATH_USD).call().await.unwrap();
    assert!(
        first_hop_pool_after.reserveUserToken > first_hop_pool_before.reserveUserToken,
        "feeToken -> PATH pool should collect fee token input"
    );
    assert!(
        first_hop_pool_after.reserveValidatorToken < first_hop_pool_before.reserveValidatorToken,
        "feeToken -> PATH pool should spend PATH output"
    );

    let second_hop_pool_after = fee_manager.getPool(PATH_USD, BETA_USD).call().await.unwrap();
    assert!(
        second_hop_pool_after.reserveUserToken > second_hop_pool_before.reserveUserToken,
        "PATH -> BETA pool should collect PATH input"
    );
    assert!(
        second_hop_pool_after.reserveValidatorToken < second_hop_pool_before.reserveValidatorToken,
        "PATH -> BETA pool should spend BETA output"
    );
}

// ============================================================================
// Tempo Genesis: Fee Token Metadata
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_token_metadata() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let tokens = [
        (PATH_USD, "PathUSD", "PathUSD"),
        (ALPHA_USD, "AlphaUSD", "AlphaUSD"),
        (BETA_USD, "BetaUSD", "BetaUSD"),
        (THETA_USD, "ThetaUSD", "ThetaUSD"),
    ];

    for (addr, expected_name, expected_symbol) in tokens {
        let token = IERC20::new(addr, &provider);
        let name = token.name().call().await.unwrap();
        let symbol = token.symbol().call().await.unwrap();
        let decimals = token.decimals().call().await.unwrap();

        assert_eq!(name, expected_name, "Token at {addr} name mismatch");
        assert_eq!(symbol, expected_symbol, "Token at {addr} symbol mismatch");
        assert_eq!(decimals, 6, "All TIP20 tokens should use 6 decimals");
    }
}

// ============================================================================
// Tempo Genesis: Fee Token Balances
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_token_balances_minted_to_dev_accounts() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let dev_accounts: Vec<Address> = handle.dev_accounts().collect();
    assert!(!dev_accounts.is_empty());

    for account in dev_accounts.iter().take(3) {
        for token_addr in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
            let token = IERC20::new(token_addr, &provider);
            let balance = token.balanceOf(*account).call().await.unwrap();
            assert!(
                balance > U256::ZERO,
                "Account {account} should have {token_addr} balance, got 0"
            );
        }
    }
}

// ============================================================================
// Tempo Genesis: Dev Account Balance
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_dev_accounts_have_balance() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();

    for account in handle.dev_accounts() {
        let balance = provider.get_balance(account).await.unwrap();
        assert_eq!(balance, genesis_balance, "Dev account {account} should have genesis balance");
    }
}

// ============================================================================
// TIP20 Token Operations: Transfer
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_transfer() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);

    let sender_balance_before = token.balanceOf(sender).call().await.unwrap();
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(1_000_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());

    let sender_balance_after = token.balanceOf(sender).call().await.unwrap();
    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();

    assert_eq!(
        sender_balance_before - transfer_amount,
        sender_balance_after,
        "Sender balance should decrease by transfer amount"
    );
    assert_eq!(
        recipient_balance_before + transfer_amount,
        recipient_balance_after,
        "Recipient balance should increase by transfer amount"
    );
}

// ============================================================================
// TIP20 Token Operations: Approve and TransferFrom
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_approve_and_transfer_from() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let owner = accounts[0];
    let spender = accounts[1];
    let recipient = accounts[2];

    let token = IERC20::new(BETA_USD, &provider);

    // Owner approves spender
    let approve_amount = U256::from(5_000_000);
    let approve_call = token.approve(spender, approve_amount);
    let calldata: Bytes = approve_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(owner)
        .to(BETA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let allowance = token.allowance(owner, spender).call().await.unwrap();
    assert_eq!(allowance, approve_amount);

    // Spender transfers from owner to recipient
    let transfer_amount = U256::from(2_000_000);
    let transfer_from_call = token.transferFrom(owner, recipient, transfer_amount);
    let calldata: Bytes = transfer_from_call.calldata().clone();

    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let tx = TransactionRequest::default()
        .from(spender)
        .to(BETA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(recipient_balance_before + transfer_amount, recipient_balance_after);

    let allowance_after = token.allowance(owner, spender).call().await.unwrap();
    assert_eq!(allowance_after, approve_amount - transfer_amount);
}

// ============================================================================
// TIP20 Token Operations: Total Supply
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_total_supply() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let token = IERC20::new(PATH_USD, &provider);
    let total_supply = token.totalSupply().call().await.unwrap();

    assert!(total_supply > U256::ZERO, "Total supply should be non-zero");
}

// ============================================================================
// TIP20 Token Operations: Transfer Between Different Fee Tokens
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_between_different_fee_tokens() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    for token_addr in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
        let token = IERC20::new(token_addr, &provider);
        let balance_before = token.balanceOf(recipient).call().await.unwrap();

        let transfer_amount = U256::from(100_000);
        let transfer_call = token.transfer(recipient, transfer_amount);
        let calldata: Bytes = transfer_call.calldata().clone();

        let tx = TransactionRequest::default()
            .from(sender)
            .to(token_addr)
            .with_input(calldata)
            .with_gas_limit(TIP20_TRANSFER_GAS);

        let tx = WithOtherFields::new(tx);
        let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
        assert!(receipt.status(), "Transfer for {token_addr} failed");

        let balance_after = token.balanceOf(recipient).call().await.unwrap();
        assert_eq!(balance_after, balance_before + transfer_amount);
    }
}

// ============================================================================
// TIP20 Token Operations: All Fee Tokens Have Correct Metadata
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_all_fee_tokens_have_correct_metadata() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let tokens = [
        (PATH_USD, "PathUSD"),
        (ALPHA_USD, "AlphaUSD"),
        (BETA_USD, "BetaUSD"),
        (THETA_USD, "ThetaUSD"),
    ];

    for (addr, expected_name) in tokens {
        let token = IERC20::new(addr, &provider);
        let name = token.name().call().await.unwrap();
        let decimals = token.decimals().call().await.unwrap();

        assert_eq!(name, expected_name, "Token at {addr} should be named {expected_name}");
        assert_eq!(decimals, 6, "All TIP20 tokens use 6 decimals");
    }
}

// ============================================================================
// TIP20 Token Operations: Transfer Emits Event
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_emits_event() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_amount = U256::from(1_000_000);
    let transfer_call = token.transfer(to, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(!receipt.inner.logs().is_empty(), "Transfer should emit event");

    let log = &receipt.inner.logs()[0];
    assert_eq!(log.address(), ALPHA_USD);

    let transfer_topic =
        alloy_primitives::keccak256("Transfer(address,address,uint256)".as_bytes());
    assert_eq!(log.topics()[0], transfer_topic);
}

// ============================================================================
// Tempo Transactions: Native Value Transfer Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_native_value_transfer_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let tx = TransactionRequest::default()
        .from(from)
        .to(to)
        .value(U256::from(1_000_000_000_000_000_000u64)); // 1 ETH

    let tx = WithOtherFields::new(tx);
    let result = provider.send_transaction(tx).await;
    assert!(result.is_err(), "Native ETH transfers should be rejected in Tempo mode");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("native value transfer not allowed"),
        "Expected 'native value transfer not allowed' error, got: {err}"
    );
}

// ============================================================================
// Tempo Transactions: Zero-Value EIP-1559 Tx Succeeds
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_zero_value_tx_succeeds() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    // TIP20 transfer (value=0, only calldata)
    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1_000_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
}

// ============================================================================
// Tempo Transactions: Contract Deployment
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_contract_deployment() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];

    // Minimal contract: PUSH1 0x00 PUSH1 0x00 RETURN (returns empty)
    let bytecode = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3]);

    let tx =
        TransactionRequest::default().from(sender).with_input(bytecode).with_gas_limit(100_000);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
    assert!(receipt.contract_address.is_some(), "Should have deployed a contract");
}

// ============================================================================
// Tempo Transactions: Nonce Increments
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_nonce_increments() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let nonce_before = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce_before, 0);

    // Send a TIP20 transfer
    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(to, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let nonce_after = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce_after, 1);
}

// ============================================================================
// Tempo AA Transaction Tests (Type 0x76)
// ============================================================================

/// Helper to get the private key for a dev account.
fn dev_key(index: u32) -> PrivateKeySigner {
    let mnemonic = "test test test test test test test test test test test junk";
    alloy_signer_local::MnemonicBuilder::<alloy_signer_local::coins_bip39::English>::default()
        .phrase(mnemonic)
        .index(index)
        .expect("valid mnemonic")
        .build()
        .expect("valid key")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_basic() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(100_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::ZERO,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_2d_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    // Send two transactions with different nonce keys (can be parallelized)
    let nonce_keys = [U256::from(1), U256::from(2)];

    for (i, nonce_key) in nonce_keys.iter().enumerate() {
        let transfer_amount = U256::from(50_000 * (i + 1) as u64);
        let transfer_call = token.transfer(recipient, transfer_amount);
        let calldata: Bytes = transfer_call.calldata().clone();

        let tempo_tx = TempoTransaction {
            chain_id,
            fee_token: Some(ALPHA_USD),
            max_priority_fee_per_gas: base_fee / 10,
            max_fee_per_gas: base_fee * 2,
            gas_limit: TIP20_TRANSFER_GAS,
            calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
            access_list: Default::default(),
            nonce_key: *nonce_key,
            nonce: 0,
            fee_payer_signature: None,
            valid_before: None,
            valid_after: None,
            key_authorization: None,
            tempo_authorization_list: vec![],
        };

        let sig_hash = tempo_tx.signature_hash();
        let signature = signer.sign_hash(&sig_hash).await.unwrap();
        let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
        let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
        let envelope = TempoTxEnvelope::AA(signed_tx);

        let mut encoded = Vec::new();
        envelope.encode_2718(&mut encoded);
        let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
        let receipt = tx_hash.get_receipt().await.unwrap();

        assert!(receipt.status(), "Tempo AA transaction with nonce_key {nonce_key} should succeed");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_nonzero_lane_pending_tx_does_not_advance_scalar_nonce() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);
    let from = signer.address();

    let pending_nonce =
        provider.get_transaction_count(from).block_id(BlockId::pending()).await.unwrap();
    assert_eq!(pending_nonce, 0);

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(42),
        nonce: 7,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let _ = provider.send_raw_transaction(&encoded).await.unwrap();

    let pending_nonce =
        provider.get_transaction_count(from).block_id(BlockId::pending()).await.unwrap();
    assert_eq!(pending_nonce, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_txpool_keeps_nonzero_nonce_lanes_separate() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);
    let from = signer.address();

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();
    let nonce_keys = [U256::from(100), U256::from(101)];

    for nonce_key in nonce_keys {
        let tempo_tx = TempoTransaction {
            chain_id,
            fee_token: Some(ALPHA_USD),
            max_priority_fee_per_gas: base_fee / 10,
            max_fee_per_gas: base_fee * 2,
            gas_limit: TIP20_TRANSFER_GAS,
            calls: vec![Call {
                to: TxKind::Call(PATH_USD),
                value: U256::ZERO,
                input: calldata.clone(),
            }],
            nonce_key,
            ..Default::default()
        };

        let sig_hash = tempo_tx.signature_hash();
        let signature = signer.sign_hash(&sig_hash).await.unwrap();
        let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
        let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
        let envelope = TempoTxEnvelope::AA(signed_tx);

        let mut encoded = Vec::new();
        envelope.encode_2718(&mut encoded);
        let _ = provider.send_raw_transaction(&encoded).await.unwrap();
    }

    let inspect = provider.txpool_inspect().await.unwrap();
    let pending = inspect.pending.get(&from).unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.contains_key("100:0"));
    assert!(pending.contains_key("101:0"));

    let content = provider.txpool_content().await.unwrap();
    let pending = content.pending.get(&from).unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.contains_key("100:0"));
    assert!(pending.contains_key("101:0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_valid_before() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 30;

    let transfer_call = token.transfer(recipient, U256::from(75_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(3),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with valid_before should succeed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_valid_after() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_after = current_time;
    let valid_before = current_time + 30;

    let transfer_call = token.transfer(recipient, U256::from(60_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(4),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: NonZeroU64::new(valid_after),
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(
        receipt.status(),
        "Tempo AA transaction with valid_after (already valid) should succeed"
    );
}

// ============================================================================
// Tempo AA Transaction Error Cases
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_expired_valid_before() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time.saturating_sub(10); // 10 seconds ago

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(100),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with expired valid_before should be rejected");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_valid_after_future() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_after = current_time + 5;
    let valid_before = current_time + 60;
    let transfer_amount = U256::from(50_000);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(101),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: NonZeroU64::new(valid_after),
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    // Transaction enters pool but is not yet valid
    let pending = provider.send_raw_transaction(&encoded).await.unwrap();
    let tx_hash = *pending.tx_hash();

    api.mine_one().await;
    let receipt = provider.get_transaction_receipt(tx_hash).await.unwrap();
    assert!(receipt.is_none(), "Transaction should not be mined before valid_after");
    let recipient_balance = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(recipient_balance, recipient_balance_before);

    // Advance time past valid_after
    api.evm_set_next_block_timestamp(valid_after + 1).unwrap();
    api.mine_one().await;

    let receipt = pending.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction should succeed after valid_after time");
    let recipient_balance = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(recipient_balance, recipient_balance_before + transfer_amount);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_replay_same_key() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let nonce_key = U256::from(200);

    // First transaction with nonce 0
    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx1 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(PATH_USD),
            value: U256::ZERO,
            input: calldata.clone(),
        }],
        access_list: Default::default(),
        nonce_key,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx1.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx1, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First transaction should succeed");

    // Second transaction with nonce=1 on the same key should succeed
    let transfer_call2 = token.transfer(recipient, U256::from(60_000));
    let calldata2: Bytes = transfer_call2.calldata().clone();

    let tempo_tx2 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata2 }],
        access_list: Default::default(),
        nonce_key,
        nonce: 1,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx2.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx2, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash2 = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt2 = tx_hash2.get_receipt().await.unwrap();
    assert!(receipt2.status(), "Second transaction with nonce=1 should succeed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_parallel_nonces_different_keys() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    // Send two transactions with the SAME nonce (0) but DIFFERENT nonce keys
    let mut tx_hashes = vec![];

    for nonce_key_val in [300u64, 301u64] {
        let transfer_call = token.transfer(recipient, U256::from(10_000));
        let calldata: Bytes = transfer_call.calldata().clone();

        let tempo_tx = TempoTransaction {
            chain_id,
            fee_token: Some(ALPHA_USD),
            max_priority_fee_per_gas: base_fee / 10,
            max_fee_per_gas: base_fee * 2,
            gas_limit: TIP20_TRANSFER_GAS,
            calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
            access_list: Default::default(),
            nonce_key: U256::from(nonce_key_val),
            nonce: 0,
            fee_payer_signature: None,
            valid_before: None,
            valid_after: None,
            key_authorization: None,
            tempo_authorization_list: vec![],
        };

        let sig_hash = tempo_tx.signature_hash();
        let signature = signer.sign_hash(&sig_hash).await.unwrap();
        let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
        let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
        let envelope = TempoTxEnvelope::AA(signed_tx);

        let mut encoded = Vec::new();
        envelope.encode_2718(&mut encoded);

        let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
        tx_hashes.push(tx_hash);
    }

    for tx_hash in tx_hashes {
        let receipt = tx_hash.get_receipt().await.unwrap();
        assert!(receipt.status(), "Parallel transactions with different nonce keys should succeed");
    }

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + U256::from(20_000),
        "Recipient should receive both transfers"
    );
}

// ============================================================================
// Gas Estimation
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default().from(accounts[0]).to(ALPHA_USD).with_input(calldata);

    let gas_estimate = provider.estimate_gas(tx.into()).await.unwrap();

    // TIP20 transfer should use more than 21000 gas
    assert!(gas_estimate > 21000, "TIP20 transfer should use more than 21000 gas");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_for_contract_call() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default().from(accounts[0]).to(ALPHA_USD).with_input(calldata);

    let gas_estimate = provider.estimate_gas(tx.into()).await.unwrap();

    // Contract call should use more gas than simple transfer
    assert!(gas_estimate > 21000, "Contract call should use more than 21000 gas");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_with_value_fails() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    // Gas estimation with native value should fail in Tempo mode
    let tx = TransactionRequest::default()
        .from(accounts[0])
        .to(accounts[1])
        .value(U256::from(1_000_000_000_000_000_000u64));

    let result = provider.estimate_gas(tx.into()).await;
    assert!(result.is_err(), "Gas estimation with native value should fail in Tempo mode");
}

// ============================================================================
// Gas Price & Base Fee
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_price() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let gas_price = provider.get_gas_price().await.unwrap();

    assert!(gas_price > 0, "Gas price should be non-zero");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_base_fee() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();

    assert!(block.header.base_fee_per_gas.is_some());
}

// ============================================================================
// Fee Token Deduction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_eip1559_fee_token_deduction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    // Check fee token balance before (ALPHA_USD is the default fee token)
    let fee_token = IERC20::new(ALPHA_USD, &provider);
    let fee_balance_before = fee_token.balanceOf(sender).call().await.unwrap();

    // Transfer PATH_USD so balance change is only from gas fees, not the transfer itself
    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(100_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");

    // Fee token balance should have decreased (gas fees paid in ALPHA_USD)
    let fee_balance_after = fee_token.balanceOf(sender).call().await.unwrap();
    assert!(
        fee_balance_after < fee_balance_before,
        "Fee token balance should decrease after paying gas (before: {fee_balance_before}, after: {fee_balance_after})"
    );
}

// ============================================================================
// Anvil Control: Set Balance
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_balance() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let random_address = Address::random();
    let new_balance = U256::from(1_000_000_000_000_000_000u64);

    let balance_before = provider.get_balance(random_address).await.unwrap();
    assert_eq!(balance_before, U256::ZERO);

    api.anvil_set_balance(random_address, new_balance).await.unwrap();

    let balance_after = provider.get_balance(random_address).await.unwrap();
    assert_eq!(balance_after, new_balance);
}

// ============================================================================
// Anvil Control: Set Code
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    let target = Address::random();

    let code_before = api.get_code(target, None).await.unwrap();
    assert!(code_before.is_empty());

    let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH 0, PUSH 0, RETURN
    api.anvil_set_code(target, bytecode.clone().into()).await.unwrap();

    let code_after = api.get_code(target, None).await.unwrap();
    assert_eq!(code_after.as_ref(), bytecode.as_slice());
}

// ============================================================================
// Anvil Control: Auto Mine Toggle
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_auto_mine_toggle() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    assert!(api.anvil_get_auto_mine().unwrap());

    api.anvil_set_auto_mine(false).await.unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());

    api.anvil_set_auto_mine(true).await.unwrap();
    assert!(api.anvil_get_auto_mine().unwrap());
}

// ============================================================================
// Anvil Control: Manual Mining
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_manual_mining() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let block_before = provider.get_block_number().await.unwrap();

    api.mine_one().await;

    let block_after = provider.get_block_number().await.unwrap();
    assert_eq!(block_after, block_before + 1);

    api.anvil_mine(Some(U256::from(5)), None).await.unwrap();

    let block_final = provider.get_block_number().await.unwrap();
    assert_eq!(block_final, block_after + 5);
}

// ============================================================================
// Anvil Control: Impersonate Account
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let impersonated = handle.dev_accounts().next().unwrap();
    let recipient = handle.dev_accounts().nth(1).unwrap();

    api.anvil_impersonate_account(impersonated).await.unwrap();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(impersonated)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    api.anvil_stop_impersonating_account(impersonated).await.unwrap();
}

// ============================================================================
// Anvil Control: Snapshot and Revert
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_snapshot_and_revert() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let token = IERC20::new(ALPHA_USD, &provider);
    let balance_before = token.balanceOf(to).call().await.unwrap();
    let block_before = provider.get_block_number().await.unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();

    let transfer_call = token.transfer(to, U256::from(1_000_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let balance_after_tx = token.balanceOf(to).call().await.unwrap();
    assert!(balance_after_tx > balance_before);

    api.evm_revert(snapshot_id).await.unwrap();

    let balance_reverted = token.balanceOf(to).call().await.unwrap();
    let block_reverted = provider.get_block_number().await.unwrap();

    assert_eq!(balance_reverted, balance_before);
    assert_eq!(block_reverted, block_before);
}

// ============================================================================
// Block & Chain: Tempo Mode Enabled
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_mode_enabled_by_default() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 0);
}

// ============================================================================
// Block & Chain: Fee Tokens Deployed
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_tokens_deployed() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    for token in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
        let code = api.get_code(token, None).await.unwrap();
        assert!(!code.is_empty(), "Token {token} should have code deployed");
    }
}

// ============================================================================
// Block & Chain: Block Has Timestamp
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_has_timestamp() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(1.into()).await.unwrap().unwrap();
    assert!(block.header.timestamp > 0, "Block should have a timestamp");
}

// ============================================================================
// Block & Chain: Block Timestamp Increases
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_timestamp_increases() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;
    let block1 = provider.get_block(1.into()).await.unwrap().unwrap();

    let future_timestamp = block1.header.timestamp + 100;
    api.evm_set_next_block_timestamp(future_timestamp).unwrap();

    api.mine_one().await;
    let block2 = provider.get_block(2.into()).await.unwrap().unwrap();

    assert_eq!(block2.header.timestamp, future_timestamp);
    assert!(block2.header.timestamp > block1.header.timestamp);
}

// ============================================================================
// Block & Chain: Block Timestamps Are Monotonic
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_timestamps_are_monotonic() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;
    let block1 = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let timestamp1 = block1.header.timestamp;

    let future_timestamp = timestamp1 + 10;
    api.evm_set_next_block_timestamp(future_timestamp).unwrap();

    api.mine_one().await;
    let block2 = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let timestamp2 = block2.header.timestamp;

    assert!(
        timestamp2 > timestamp1,
        "Block timestamps must be strictly increasing: {timestamp2} should be > {timestamp1}",
    );
    assert_eq!(timestamp2, future_timestamp, "Block timestamp should match the set value");
}

// ============================================================================
// Block & Chain: Block Gas Limit
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_gas_limit() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();

    assert!(block.header.gas_limit > 0);
}

// ============================================================================
// Block & Chain: Transaction Respects Gas Limit
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_respects_gas_limit() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(accounts[0])
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    assert!(receipt.gas_used <= TIP20_TRANSFER_GAS);
}

// ============================================================================
// Block & Chain: Multiple Transactions in Block
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_transactions_in_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);

    let transfer1 = token.transfer(accounts[1], U256::from(1000));
    let calldata1: Bytes = transfer1.calldata().clone();
    let tx1 = TransactionRequest::default()
        .from(accounts[0])
        .to(ALPHA_USD)
        .with_input(calldata1)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let transfer2 = token.transfer(accounts[3], U256::from(2000));
    let calldata2: Bytes = transfer2.calldata().clone();
    let tx2 = TransactionRequest::default()
        .from(accounts[2])
        .to(ALPHA_USD)
        .with_input(calldata2)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx1 = WithOtherFields::new(tx1);
    let tx2 = WithOtherFields::new(tx2);

    let pending1 = provider.send_transaction(tx1).await.unwrap();
    let pending2 = provider.send_transaction(tx2).await.unwrap();

    api.mine_one().await;

    let receipt1 = pending1.get_receipt().await.unwrap();
    let receipt2 = pending2.get_receipt().await.unwrap();

    assert_eq!(receipt1.block_number, receipt2.block_number);
}

// ============================================================================
// Block & Chain: Chain ID
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 31337);
}

// ============================================================================
// Block & Chain: Custom Chain ID
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_custom_chain_id() {
    let custom_chain_id = 42069u64;
    let (_api, handle) = spawn(NodeConfig::test_tempo().with_chain_id(Some(custom_chain_id))).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, custom_chain_id);
}

// ============================================================================
// Tempo AA: Expiring Nonce Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_expiring_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 25;

    let transfer_call = token.transfer(recipient, U256::from(80_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::MAX,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with expiring nonce should succeed");
}

// ============================================================================
// Tempo AA: Expiring Nonce Replay
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_expiring_nonce_replay() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 25;

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::MAX,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let first_tx_hash = *tx_hash.tx_hash();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First expiring nonce transaction should succeed");

    // Replay the exact same transaction bytes
    let result = provider.send_raw_transaction(&encoded).await;

    if let Ok(pending) = result {
        let second_tx_hash = *pending.tx_hash();
        assert_eq!(
            first_tx_hash, second_tx_hash,
            "Replaying same transaction should return same tx hash (not execute again)"
        );
    }
}

// ============================================================================
// Tempo AA: Multiple Calls in Single Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_multiple_calls() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient1 = accounts[1];
    let recipient2 = accounts[2];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let recipient1_balance_before = token.balanceOf(recipient1).call().await.unwrap();
    let recipient2_balance_before = token.balanceOf(recipient2).call().await.unwrap();

    let amount1 = U256::from(25_000);
    let amount2 = U256::from(35_000);

    let call1_data: Bytes = token.transfer(recipient1, amount1).calldata().clone();
    let call2_data: Bytes = token.transfer(recipient2, amount2).calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS * 2,
        calls: vec![
            Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: call1_data },
            Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: call2_data },
        ],
        access_list: Default::default(),
        nonce_key: U256::from(5),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with multiple calls should succeed");

    let recipient1_balance_after = token.balanceOf(recipient1).call().await.unwrap();
    let recipient2_balance_after = token.balanceOf(recipient2).call().await.unwrap();

    assert_eq!(recipient1_balance_after, recipient1_balance_before + amount1);
    assert_eq!(recipient2_balance_after, recipient2_balance_before + amount2);
}

// ============================================================================
// Tempo AA: Nonce Keys Are Isolated
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_keys_are_isolated() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx1 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(PATH_USD),
            value: U256::ZERO,
            input: calldata.clone(),
        }],
        access_list: Default::default(),
        nonce_key: U256::from(100),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx1.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx1, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First tx with nonce_key=100 should succeed");

    let tempo_tx2 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(101),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx2.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx2, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash2 = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt2 = tx_hash2.get_receipt().await.unwrap();
    assert!(
        receipt2.status(),
        "Tx with different nonce_key should succeed even with same nonce value"
    );
}

// ============================================================================
// Tempo AA: Explicit Fee Token Selection
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_explicit_fee_token_selection() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];
    let signer = dev_key(0);

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let path_token = IERC20::new(PATH_USD, &provider);
    let alpha_token = IERC20::new(ALPHA_USD, &provider);
    let path_balance_before = path_token.balanceOf(sender).call().await.unwrap();
    let alpha_balance_before = alpha_token.balanceOf(sender).call().await.unwrap();

    let transfer_call = path_token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(400),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction with explicit fee token should succeed");

    let path_balance_after = path_token.balanceOf(sender).call().await.unwrap();
    let alpha_balance_after = alpha_token.balanceOf(sender).call().await.unwrap();

    assert!(
        path_balance_after < path_balance_before,
        "PATH_USD balance should decrease (transfer + fees)"
    );
    assert_eq!(
        alpha_balance_after, alpha_balance_before,
        "ALPHA_USD balance should not change when using PATH_USD for fees"
    );
}

// ============================================================================
// Tempo AA: Fee Token Swap (Different Tokens)
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_token_swap_different_tokens() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let alpha_token = IERC20::new(ALPHA_USD, &provider);
    let alice_alpha_before = alpha_token.balanceOf(sender).call().await.unwrap();

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(100_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");

    let alice_alpha_after = alpha_token.balanceOf(sender).call().await.unwrap();
    assert!(
        alice_alpha_after < alice_alpha_before,
        "Alice's AlphaUSD should decrease due to gas fees"
    );
}

// ============================================================================
// Tempo AA: Receipt Fields
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_receipt_fields() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(500),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");
    assert!(receipt.gas_used > 0, "Gas used should be non-zero");
    assert!(!receipt.inner.logs().is_empty(), "Should have Transfer event logs");
}

// ============================================================================
// Tempo AA: Get Transaction By Hash
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_get_transaction_by_hash() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(501),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let pending = provider.send_raw_transaction(&encoded).await.unwrap();
    let tx_hash = *pending.tx_hash();
    pending.get_receipt().await.unwrap();

    let tx = api.transaction_by_hash(tx_hash).await.unwrap();
    assert!(tx.is_some(), "Transaction should be retrievable by hash");

    let tx = tx.unwrap();
    assert_eq!(tx.ty(), 0x76, "Transaction type should be 0x76 (Tempo)");
    assert_eq!(TransactionResponse::from(&tx), sender, "From address should match sender");
}

// ============================================================================
// Tempo AA: Wrong Chain ID Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_wrong_chain_id_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let correct_chain_id = provider.get_chain_id().await.unwrap();
    let wrong_chain_id = correct_chain_id + 1;
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id: wrong_chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(1),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with wrong chain ID should be rejected");
}

// ============================================================================
// Tempo AA: Gas Too Low Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_gas_too_low_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: 1000,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(2),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with gas limit below intrinsic cost should be rejected");
}

// ============================================================================
// Tempo AA: Value In Call Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_value_in_call_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(recipient),
            value: U256::from(1_000_000),
            input: Bytes::new(),
        }],
        access_list: Default::default(),
        nonce_key: U256::from(3),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    if let Ok(pending) = result {
        let receipt = pending.get_receipt().await;
        if let Ok(r) = receipt {
            assert!(!r.status(), "Transaction with ETH value should fail in Tempo mode");
        }
    }
}

// ============================================================================
// Tempo AA: Nonce Too High Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_too_high_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(999),
        nonce: 5,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    // Transaction may be accepted into pool but should fail during execution
    let result = provider.send_raw_transaction(&encoded).await;
    if result.is_ok() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

// ============================================================================
// Gas Estimation: Tempo AA Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default().from(accounts[0]).to(PATH_USD).with_input(calldata),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    assert!(
        gas_estimate > 21000,
        "Tempo AA gas estimate should be greater than 21000, got: {gas_estimate}"
    );
}

// ============================================================================
// Gas Estimation: Tempo AA with 2D Nonce
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_with_2d_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no 2D nonce)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    // 2D nonce tx with nonce=0 (new nonce key)
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata)
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!("0x64")),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // New 2D nonce key (nonce=0) charges COLD_SLOAD + SSTORE_SET = 22100 gas
    let nonce_key_delta = gas_estimate - baseline_gas;
    assert!(
        nonce_key_delta >= 22_100,
        "2D nonce should add >= 22100 gas, got delta: {nonce_key_delta} \
         (baseline: {baseline_gas}, 2d_nonce: {gas_estimate})"
    );
}

// ============================================================================
// Gas Estimation: Tempo AA with Expiring Nonce
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_expiring_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no expiring nonce)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    let max_nonce_key = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata)
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(max_nonce_key)),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // At T0, expiring nonces are treated as 2D nonces (22100 gas).
    // At T1+, this charges EXPIRING_NONCE_GAS = 13000 instead.
    let expiring_delta = gas_estimate - baseline_gas;
    assert!(
        expiring_delta >= 22_100,
        "Expiring nonce should add >= 22100 gas at T0, got delta: {expiring_delta} \
         (baseline: {baseline_gas}, expiring: {gas_estimate})"
    );
}

// ============================================================================
// Gas Estimation: T1 Hardfork Nonce Gas Costs
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_t1_nonce_costs() {
    use tempo_hardfork::TempoHardfork;

    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T1.into()))).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no nonce key)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    // TIP-1000: nonce=0 pays 250k for account creation
    assert!(
        baseline_gas > 250_000,
        "T1 baseline should include 250k (TIP-1000), got: {baseline_gas}"
    );

    // Existing 2D nonce key (nonce=1) charges 5,000 gas
    let nonce_2d_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(1),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!("0x64")),
        ]
        .into_iter()
        .collect(),
    };
    let nonce_2d_gas = provider.estimate_gas(nonce_2d_tx).await.unwrap();

    let baseline_nonce1_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(1),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_nonce1_gas = provider.estimate_gas(baseline_nonce1_tx).await.unwrap();
    let nonce_2d_delta = nonce_2d_gas - baseline_nonce1_gas;

    assert!(
        nonce_2d_delta >= 5_000,
        "T1: existing 2D nonce key should add >= 5000 gas, got: {nonce_2d_delta}"
    );

    // TIP-1000: nonce=0 should cost 250k more than nonce=1
    let tip1000_delta = baseline_gas - baseline_nonce1_gas;
    assert!(
        tip1000_delta >= 250_000,
        "T1: TIP-1000 delta should be >= 250000, got: {tip1000_delta}"
    );

    // Expiring nonce (nonce_key=MAX) at T1 should charge ~13K for ring buffer ops
    // (2*COLD_SLOAD + WARM_SLOAD + 3*WARM_SSTORE_RESET), NOT 22K like at T0.
    let max_nonce_key = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let valid_before = block.header.timestamp + 25;

    let expiring_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(max_nonce_key)),
            ("validBefore".to_string(), serde_json::json!(valid_before)),
        ]
        .into_iter()
        .collect(),
    };
    let expiring_gas = provider.estimate_gas(expiring_tx).await.unwrap();

    // Compare against baseline_nonce1 (nonce=1, no nonce key) since both the baseline (nonce=0)
    // and expiring tx (nonce=0) include the 250k account creation cost.
    let expiring_delta = expiring_gas - baseline_nonce1_gas;
    assert!(
        expiring_delta >= 13_000,
        "T1: expiring nonce should add at least ~13K gas for ring buffer ops, got delta: {expiring_delta}"
    );
}

// ============================================================================
// Gas Estimation: 2D Nonce Estimate Sufficient for Real Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_2d_nonce_converges() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Estimate gas with 2D nonce
    let nonce_key = U256::from(0x64);
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(format!("{nonce_key:#x}"))),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // Send the actual transaction with the estimated gas to verify it's sufficient
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: gas_estimate,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(
        receipt.status(),
        "2D nonce transaction should succeed with estimated gas: {gas_estimate}"
    );
    assert!(
        receipt.gas_used() <= gas_estimate,
        "Gas used ({}) should be <= estimate ({}) for 2D nonce tx",
        receipt.gas_used(),
        gas_estimate
    );
}

// ============================================================================
// Gas Estimation: Converges for Tempo Intrinsic Gas
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_converges_for_tempo_intrinsic_gas() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // Send the actual transaction with the estimated gas to verify convergence
    let signer = dev_key(0);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: gas_estimate,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::ZERO,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction should succeed with estimated gas: {gas_estimate}");

    assert!(
        receipt.gas_used() <= gas_estimate,
        "Gas used ({}) should be <= estimate ({})",
        receipt.gas_used(),
        gas_estimate
    );
}

// ============================================================================
// EIP-1559 Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_eip1559_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(500_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "EIP-1559 transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}

// ============================================================================
// Legacy Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_legacy_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(250_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let gas_price = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .with_gas_price(gas_price);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Legacy transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}

// ============================================================================
// TipFeeManager RPC Methods
// ============================================================================

/// `anvil_setFeeToken` sets the fee token for a user address, readable via `userTokens`.
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_fee_token() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let user = Address::random();
    let fee_manager = IFeeManagerRpc::new(TIP_FEE_MANAGER_ADDRESS, &provider);

    // Before: user has no token set (returns zero address)
    let token_before = fee_manager.userTokens(user).call().await.unwrap();
    assert_eq!(token_before, Address::ZERO, "User should have no fee token initially");

    // Set user fee token to ALPHA_USD
    api.anvil_set_fee_token(user, ALPHA_USD).await.unwrap();

    // After: userTokens returns the configured token
    let token_after = fee_manager.userTokens(user).call().await.unwrap();
    assert_eq!(token_after, ALPHA_USD, "User fee token should be set to ALPHA_USD");
}

/// `anvil_setFeeToken` returns an error when Tempo mode is not active.
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_fee_token_non_tempo_fails() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let result = api.anvil_set_fee_token(Address::random(), ALPHA_USD).await;
    assert!(result.is_err(), "anvil_setFeeToken should fail outside of Tempo mode");
}

/// `anvil_setValidatorFeeToken` sets the fee token for a validator, readable via `validatorTokens`.
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_validator_fee_token() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let validator = Address::random();
    let fee_manager = IFeeManagerRpc::new(TIP_FEE_MANAGER_ADDRESS, &provider);

    // Before: validator has no token set (returns zero address)
    let token_before = fee_manager.validatorTokens(validator).call().await.unwrap();
    assert_eq!(token_before, DEFAULT_FEE_TOKEN, "Validator should have no fee token initially");

    // Set validator fee token to BETA_USD
    api.anvil_set_validator_fee_token(validator, BETA_USD).await.unwrap();

    // After: validatorTokens returns the configured token
    let token_after = fee_manager.validatorTokens(validator).call().await.unwrap();
    assert_eq!(token_after, BETA_USD, "Validator fee token should be set to BETA_USD");
}

/// `anvil_setValidatorFeeToken` returns an error when Tempo mode is not active.
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_validator_fee_token_non_tempo_fails() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let result = api.anvil_set_validator_fee_token(Address::random(), BETA_USD).await;
    assert!(result.is_err(), "anvil_setValidatorFeeToken should fail outside of Tempo mode");
}

/// `anvil_setFeeAmmLiquidity` mints AMM liquidity for a token pair,
/// verifiable via `getPool` (non-zero reserves) and `totalSupply` (non-zero LP supply).
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_fee_amm_liquidity() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let fee_manager = IFeeManagerRpc::new(TIP_FEE_MANAGER_ADDRESS, &provider);

    // Genesis pre-seeds all pairs between fee tokens; record current state before adding more
    let pool_before = fee_manager.getPool(PATH_USD, ALPHA_USD).call().await.unwrap();
    let pool_id = fee_manager.getPoolId(PATH_USD, ALPHA_USD).call().await.unwrap();
    let lp_supply_before = fee_manager.totalSupply(pool_id).call().await.unwrap();

    let amount = U256::from(1_000_000u64);
    api.anvil_set_fee_amm_liquidity(PATH_USD, ALPHA_USD, amount).await.unwrap();

    // After minting: reserves and LP supply should have increased
    let pool_after = fee_manager.getPool(PATH_USD, ALPHA_USD).call().await.unwrap();
    let lp_supply_after = fee_manager.totalSupply(pool_id).call().await.unwrap();

    assert!(
        pool_after.reserveValidatorToken > pool_before.reserveValidatorToken,
        "Validator token reserve should increase after minting liquidity"
    );
    assert!(
        lp_supply_after > lp_supply_before,
        "LP token total supply should increase after minting liquidity"
    );
}

/// `anvil_setFeeAmmLiquidity` returns an error when Tempo mode is not active.
#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_fee_amm_liquidity_non_tempo_fails() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let result =
        api.anvil_set_fee_amm_liquidity(PATH_USD, ALPHA_USD, U256::from(1_000_000u64)).await;
    assert!(result.is_err(), "anvil_setFeeAmmLiquidity should fail outside of Tempo mode");
}

/// Pre-T7 Tempo uses a fixed base fee, so mining empty blocks must not drift it (EIP-1559 would).
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_pre_t7_base_fee_stays_fixed() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T1.into()))).await;
    let provider = handle.http_provider();

    for _ in 0..5 {
        api.mine_one().await;
    }

    let latest = provider.get_block(BlockId::latest()).await.unwrap().unwrap().header.number;
    for n in 0..=latest {
        let block = provider.get_block(BlockId::number(n)).await.unwrap().unwrap();
        assert_eq!(
            block.header.base_fee_per_gas,
            Some(TEMPO_T1_BASE_FEE),
            "pre-T7 block {n} base fee should stay fixed"
        );
    }
}

/// `anvil_reset` must restore the Tempo base fee, not fall back to Anvil's Ethereum default.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_base_fee_survives_reset() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T1.into()))).await;
    let provider = handle.http_provider();

    for _ in 0..3 {
        api.mine_one().await;
    }

    api.anvil_reset(None).await.unwrap();
    api.mine_one().await;

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(
        block.header.base_fee_per_gas,
        Some(TEMPO_T1_BASE_FEE),
        "base fee after reset should be the fixed Tempo value, not the Ethereum default"
    );
}

/// T7 Tempo uses the TIP-1067 dynamic controller: empty blocks lower the base fee within the clamp.
#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_t7_base_fee_is_dynamic() {
    let (api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T7.into()))).await;
    let provider = handle.http_provider();

    for _ in 0..8 {
        api.mine_one().await;
    }

    // The genesis block keeps the 20B seed (matching Tempo's T7 genesis).
    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();
    assert_eq!(genesis.header.base_fee_per_gas, Some(TEMPO_T1_BASE_FEE));

    let latest = provider.get_block(BlockId::latest()).await.unwrap().unwrap().header.number;
    let mut fees = Vec::new();
    for n in 1..=latest {
        let block = provider.get_block(BlockId::number(n)).await.unwrap().unwrap();
        fees.push(block.header.base_fee_per_gas.unwrap());
    }

    // Block 1 already clamps the 20B seed down to the TIP-1067 cap; generic EIP-1559 would give
    // ~17.5B instead, so this pins the T7 path. Later empty blocks decay below the cap.
    assert_eq!(fees[0], TEMPO_T7_BASE_FEE_CAP, "block 1 clamps to the T7 cap: {fees:?}");
    assert!(fees[1] < TEMPO_T7_BASE_FEE_CAP, "empty blocks decay below the cap: {fees:?}");
    // Empty blocks never raise the base fee, and it stays within the TIP-1067 clamp.
    assert!(fees.windows(2).all(|w| w[1] <= w[0]), "base fee should not rise: {fees:?}");
    let last = *fees.last().unwrap();
    assert!(
        (TEMPO_T7_BASE_FEE_FLOOR..=TEMPO_T7_BASE_FEE_CAP).contains(&last),
        "base fee should be clamped to the TIP-1067 range: {fees:?}"
    );
}
