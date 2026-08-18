//! Base precompile ABIs for trace decoding.
//!
//! `base-common-precompiles` declares these interfaces without `#[sol(abi)]`, so it exposes no
//! [`JsonAbi`](alloy_json_abi::JsonAbi) for `register_address_abi` to consume. These declarations
//! mirror the canonical ones so traces name Base precompile calls, events, and errors, following
//! the same approach `super::monad` takes for its staking interfaces.
//!
//! The `abi_matches_canonical_selectors` tests below pin every selector against the canonical
//! types, so an interface change in Base fails here instead of silently mis-decoding traces.

use alloy_sol_types::sol;

sol! {
    /// Mirror of `base_common_precompiles::IActivationRegistry`.
    #[sol(abi)]
    interface IActivationRegistry {
        event FeatureActivated(bytes32 indexed feature, address indexed caller);
        event FeatureDeactivated(bytes32 indexed feature, address indexed caller);
        event AdminChanged(
            address indexed previousAdmin,
            address indexed newAdmin,
            address indexed caller
        );

        error Unauthorized(address caller);
        error AlreadyActivated(bytes32 feature);
        error FeatureNotActivated(bytes32 feature);
        error DelegateCallNotAllowed();
        error StaticCallNotAllowed();
        error AdminStorageNotEnabled();
        error ZeroAdminAddress();

        function isActivated(bytes32 feature) external view returns (bool);
        function checkActivated(bytes32 feature) external view;
        function admin() external view returns (address);
        function setAdmin(address newAdmin) external;
        function activate(bytes32 feature) external;
        function deactivate(bytes32 feature) external;
    }
}

sol! {
    /// Mirror of `base_common_precompiles::INonceManager`.
    #[sol(abi)]
    interface INonceManager {
        error DelegateCallNotAllowed();
        error NonPayable();
        error ProtocolNonceNotSupported();
        error InvalidNonceKey();
        error NonceOverflow();
        error InvalidExpiringNonceExpiry();
        error ExpiringNonceReplay();
        error ExpiringNonceSetFull();

        event NonceIncremented(address indexed account, uint256 indexed nonceKey, uint64 newNonce);

        function getNonce(address account, uint256 nonceKey) external view returns (uint64);
    }
}

sol! {
    /// Mirror of `base_common_precompiles::ITransactionContext`.
    #[sol(abi)]
    interface ITransactionContext {
        error DelegateCallNotAllowed();
        error NonPayable();

        function getTransactionSender() external view returns (address);
        function getTransactionPayer() external view returns (address);
        function getTransactionSenderActorId() external view returns (bytes32);
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, U256};
    use alloy_sol_types::{SolCall, SolError, SolEvent};
    use base_common_precompiles::{
        self as canonical, ActivationRegistryStorage, NonceManagerStorage, TxContextStorage,
    };
    use foundry_evm_hardforks::BaseUpgrade;
    use foundry_evm_networks::NetworkConfigs;

    use crate::{CallTrace, CallTraceDecoderBuilder};

    #[tokio::test]
    async fn registered_abis_decode_base_precompile_calls() {
        let decoder = CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_base())
            .with_base_upgrade(Some(BaseUpgrade::Cobalt))
            .build();

        let activate =
            super::IActivationRegistry::activateCall { feature: B256::repeat_byte(0x11) };
        let trace = CallTrace {
            address: ActivationRegistryStorage::ADDRESS,
            data: activate.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("ActivationRegistry"));
        assert_eq!(
            decoded.call_data.expect("activate should decode").signature,
            "activate(bytes32)"
        );

        let get_nonce = super::INonceManager::getNonceCall {
            account: Address::repeat_byte(0x22),
            nonceKey: U256::from(7),
        };
        let trace = CallTrace {
            address: NonceManagerStorage::ADDRESS,
            data: get_nonce.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(
            decoded.call_data.expect("getNonce should decode").signature,
            "getNonce(address,uint256)"
        );
    }

    /// The transaction-context precompile only exists from Cobalt, so a Beryl decoder must not
    /// name its calls. This uses a Base-unique selector on purpose: generic names like
    /// `getNonce(address,uint256)` also exist in Tempo's globally registered nonce ABI and would
    /// decode regardless of the Base upgrade.
    #[tokio::test]
    async fn pre_cobalt_decoder_does_not_register_eip8130_surfaces() {
        let call = super::ITransactionContext::getTransactionSenderActorIdCall {};
        let trace = CallTrace {
            address: TxContextStorage::ADDRESS,
            data: call.abi_encode().into(),
            success: true,
            ..Default::default()
        };

        let beryl = CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_base())
            .with_base_upgrade(Some(BaseUpgrade::Beryl))
            .build();
        assert!(
            beryl.decode_function(&trace).await.call_data.is_none(),
            "Beryl must not decode a Cobalt-only precompile"
        );

        let cobalt = CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_base())
            .with_base_upgrade(Some(BaseUpgrade::Cobalt))
            .build();
        assert_eq!(
            cobalt.decode_function(&trace).await.call_data.expect("Cobalt should decode").signature,
            "getTransactionSenderActorId()"
        );
    }

    #[test]
    fn activation_registry_abi_matches_canonical_selectors() {
        assert_eq!(
            super::IActivationRegistry::activateCall::SELECTOR,
            canonical::IActivationRegistry::activateCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::deactivateCall::SELECTOR,
            canonical::IActivationRegistry::deactivateCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::isActivatedCall::SELECTOR,
            canonical::IActivationRegistry::isActivatedCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::checkActivatedCall::SELECTOR,
            canonical::IActivationRegistry::checkActivatedCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::adminCall::SELECTOR,
            canonical::IActivationRegistry::adminCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::setAdminCall::SELECTOR,
            canonical::IActivationRegistry::setAdminCall::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::Unauthorized::SELECTOR,
            canonical::IActivationRegistry::Unauthorized::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::StaticCallNotAllowed::SELECTOR,
            canonical::IActivationRegistry::StaticCallNotAllowed::SELECTOR
        );
        assert_eq!(
            super::IActivationRegistry::FeatureActivated::SIGNATURE_HASH,
            canonical::IActivationRegistry::FeatureActivated::SIGNATURE_HASH
        );
        assert_eq!(
            super::IActivationRegistry::AdminChanged::SIGNATURE_HASH,
            canonical::IActivationRegistry::AdminChanged::SIGNATURE_HASH
        );
    }

    #[test]
    fn nonce_manager_abi_matches_canonical_selectors() {
        assert_eq!(
            super::INonceManager::getNonceCall::SELECTOR,
            canonical::INonceManager::getNonceCall::SELECTOR
        );
        assert_eq!(
            super::INonceManager::ExpiringNonceReplay::SELECTOR,
            canonical::INonceManager::ExpiringNonceReplay::SELECTOR
        );
        assert_eq!(
            super::INonceManager::InvalidExpiringNonceExpiry::SELECTOR,
            canonical::INonceManager::InvalidExpiringNonceExpiry::SELECTOR
        );
        assert_eq!(
            super::INonceManager::NonceIncremented::SIGNATURE_HASH,
            canonical::INonceManager::NonceIncremented::SIGNATURE_HASH
        );
    }

    #[test]
    fn transaction_context_abi_matches_canonical_selectors() {
        assert_eq!(
            super::ITransactionContext::getTransactionSenderCall::SELECTOR,
            canonical::ITransactionContext::getTransactionSenderCall::SELECTOR
        );
        assert_eq!(
            super::ITransactionContext::getTransactionPayerCall::SELECTOR,
            canonical::ITransactionContext::getTransactionPayerCall::SELECTOR
        );
        assert_eq!(
            super::ITransactionContext::getTransactionSenderActorIdCall::SELECTOR,
            canonical::ITransactionContext::getTransactionSenderActorIdCall::SELECTOR
        );
    }
}
