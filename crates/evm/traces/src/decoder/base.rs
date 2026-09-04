//! Base precompile ABIs for trace decoding.
//!
//! `base-common-precompiles` declares these interfaces without `#[sol(abi)]`, so it exposes no
//! [`JsonAbi`](alloy_json_abi::JsonAbi) for `register_address_abi` to consume. These declarations
//! mirror the canonical ones so traces name Base precompile calls, events, and errors, following
//! the same approach `super::monad` takes for its staking interfaces.
//!
//! The `abi_matches_canonical_selectors` tests below pin every selector against the canonical
//! types, so an interface change in Base fails here instead of silently decoding traces wrong.

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
    /// Mirror of `base_common_precompiles::IB20Factory`, the surface frozen at Beryl.
    #[sol(abi)]
    interface IB20Factory {
        enum B20Variant {
            ASSET,
            STABLECOIN
        }

        struct B20StablecoinCreateParams {
            uint8 version;
            string name;
            string symbol;
            address initialAdmin;
            string currency;
        }

        struct B20AssetCreateParams {
            uint8 version;
            string name;
            string symbol;
            address initialAdmin;
            uint8 decimals;
        }

        error NonPayable();
        error TokenAlreadyExists(address token);
        error InvalidVariant();
        error UnsupportedVersion(uint8 version, B20Variant variant);
        error MissingRequiredField(string field);
        error InvalidCurrency(string code);
        error InvalidDecimals(uint8 decimals);
        error InitCallFailed(uint256 index);

        event B20Created(
            address indexed token,
            B20Variant indexed variant,
            string name,
            string symbol,
            uint8 decimals,
            bytes variantParams
        );

        struct B20StablecoinEventParams {
            uint8 version;
            string currency;
        }

        function createB20(
            B20Variant variant,
            bytes32 salt,
            bytes calldata params,
            bytes[] calldata initCalls
        ) external returns (address token);
        function getB20Address(B20Variant variant, address sender, bytes32 salt) external view returns (address);
        function isB20(address token) external view returns (bool);
        function isB20Initialized(address token) external view returns (bool);
    }
}

sol! {
    /// **Deliberately partial** mirror of `base_common_precompiles::IB20`, the B-20 token surface.
    ///
    /// B-20 tokens are created by the factory at derived addresses, so they cannot be registered
    /// per address like the fixed precompiles. Following Tempo's `ITIP20`, these are registered
    /// globally by selector instead.
    ///
    /// Only Base-specific members belong here. The ERC-20, EIP-2612 and AccessControl portions are
    /// omitted on purpose: the global function map is not network-scoped, so adding competing
    /// candidates for `transfer`, `balanceOf`, `approve` or `permit` could change how ordinary
    /// token traces decode in any Base-enabled build. Foundry already has to order
    /// `IStorageCredits` against `ITIP20` to keep `balanceOf`'s return type stable, which is the
    /// same hazard. `b20_surface_excludes_erc20_members` pins this boundary.
    #[sol(abi)]
    interface IB20Extensions {
        enum PausableFeature {
            TRANSFER,
            MINT,
            BURN,
            SEIZE
        }

        event Memo(address indexed caller, bytes32 indexed memo);
        event BurnedBlocked(address indexed caller, address indexed from, uint256 amount);
        event Seized(address indexed caller, address indexed from, address indexed to, uint256 amount);
        event LastAdminRenounced(address indexed previousAdmin);
        event Paused(address indexed updater, PausableFeature[] features);
        event Unpaused(address indexed updater, PausableFeature[] features);
        event PolicyUpdated(bytes32 indexed policyScope, uint64 oldPolicyId, uint64 newPolicyId);

        function mintWithMemo(address to, uint256 amount, bytes32 memo) external;
        function burnWithMemo(uint256 amount, bytes32 memo) external;
        function burnBlocked(address from, uint256 amount) external;
        function seizeWithMemo(address from, address to, uint256 amount, bytes32 memo) external;
        function pausedFeatures() external view returns (PausableFeature[] memory);
        function isPaused(PausableFeature feature) external view returns (bool);
        function pause(PausableFeature[] calldata features) external;
        function unpause(PausableFeature[] calldata features) external;
        function policyId(bytes32 policyScope) external view returns (uint64);
        function updatePolicy(bytes32 policyScope, uint64 newPolicyId) external;
        function contractURI() external view returns (string);
        function updateContractURI(string calldata newURI) external;
    }
}

sol! {
    /// Mirror of `base_common_precompiles::IPolicyRegistry`, the canonical Cobalt surface.
    ///
    /// Only this surface is mirrored: its call set is a strict superset of Beryl's V1, so it also
    /// names Beryl-era policy traces. `policy_registry_covers_v1_selectors` pins that.
    #[sol(abi)]
    interface IPolicyRegistry {
        enum PolicyType {
            BLOCKLIST,
            ALLOWLIST,
            UNION,
            INTERSECT
        }

        error NonPayable();
        error Unauthorized();
        error PolicyNotFound();
        error IncompatiblePolicyType();
        error ZeroAddress();
        error BatchSizeTooLarge(uint256 maxBatchSize);
        error NoPendingAdmin();
        error ChildPoliciesOutsideOfRange();
        error InvalidChildPolicy(uint64 childPolicyId);

        event PolicyCreated(uint64 indexed policyId, address indexed creator, PolicyType policyType);
        event PolicyAdminStaged(uint64 indexed policyId, address indexed currentAdmin, address indexed pendingAdmin);
        event PolicyAdminUpdated(uint64 indexed policyId, address indexed previousAdmin, address indexed newAdmin);
        event AllowlistUpdated(uint64 indexed policyId, address indexed updater, bool allowed, address[] accounts);
        event BlocklistUpdated(uint64 indexed policyId, address indexed updater, bool blocked, address[] accounts);
        event CompositePolicyUpdated(uint64 indexed policyId, address indexed updater, uint64[] childPolicyIds);

        function createPolicy(address admin, PolicyType policyType) external returns (uint64);
        function createPolicyWithAccounts(address admin, PolicyType policyType, address[] calldata accounts) external returns (uint64);
        function createCompositePolicy(address admin, PolicyType policyType, uint64[] calldata childPolicyIds) external returns (uint64);
        function updateComposite(uint64 policyId, uint64[] calldata childPolicyIds) external;
        function stageUpdateAdmin(uint64 policyId, address newAdmin) external;
        function finalizeUpdateAdmin(uint64 policyId) external;
        function renounceAdmin(uint64 policyId) external;
        function updateAllowlist(uint64 policyId, bool allowed, address[] calldata accounts) external;
        function updateBlocklist(uint64 policyId, bool blocked, address[] calldata accounts) external;
        function isAuthorized(uint64 policyId, address account) external view returns (bool);
        function MIN_COMPOSITE_CHILD_POLICIES() external view returns (uint256);
        function MAX_COMPOSITE_CHILD_POLICIES() external view returns (uint256);
        function policyExists(uint64 policyId) external view returns (bool);
        function policyAdmin(uint64 policyId) external view returns (address);
        function pendingPolicyAdmin(uint64 policyId) external view returns (address);
        function compositePolicyChildIds(uint64 policyId) external view returns (uint64[] memory);
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
        self as canonical, ActivationRegistryStorage, B20FactoryStorage, NonceManagerStorage,
        TxContextStorage,
    };
    use foundry_evm_hardforks::{BaseUpgrade, FoundryHardfork};
    use foundry_evm_networks::NetworkConfigs;

    use crate::{CallTrace, CallTraceDecoderBuilder};

    #[tokio::test]
    async fn registered_abis_decode_base_precompile_calls() {
        let decoder = CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_base())
            .with_hardfork(Some(FoundryHardfork::Base(BaseUpgrade::Cobalt)))
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

        let create = super::IB20Factory::createB20Call {
            variant: super::IB20Factory::B20Variant::ASSET,
            salt: B256::repeat_byte(0x33),
            params: alloy_primitives::Bytes::new(),
            initCalls: Vec::new(),
        };
        let trace = CallTrace {
            address: B20FactoryStorage::ADDRESS,
            data: create.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("B20Factory"));
        let signature = decoded.call_data.expect("createB20 should decode").signature;
        assert!(signature.starts_with("createB20("), "{signature}");
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
            .with_hardfork(Some(FoundryHardfork::Base(BaseUpgrade::Beryl)))
            .build();
        assert!(
            beryl.decode_function(&trace).await.call_data.is_none(),
            "Beryl must not decode a Cobalt-only precompile"
        );

        let cobalt = CallTraceDecoderBuilder::new()
            .with_networks(NetworkConfigs::with_base())
            .with_hardfork(Some(FoundryHardfork::Base(BaseUpgrade::Cobalt)))
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

    /// Base pins this surface with an `AbiFingerprint`, but that type is only exported under its
    /// `test-utils` feature, so this compares against the canonical types directly instead.
    /// `B20Variant`'s ordinals are asserted explicitly because they are the one thing selectors
    /// cannot catch: Solidity encodes enums as `uint8`, so a reorder moves no signature, yet the
    /// ordinal is byte `[10]` of every token address the factory deploys.
    #[test]
    fn b20_factory_abi_matches_canonical_surface() {
        use alloy_sol_types::SolEnum;

        assert_eq!(
            super::IB20Factory::createB20Call::SELECTOR,
            canonical::IB20Factory::createB20Call::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::getB20AddressCall::SELECTOR,
            canonical::IB20Factory::getB20AddressCall::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::isB20Call::SELECTOR,
            canonical::IB20Factory::isB20Call::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::isB20InitializedCall::SELECTOR,
            canonical::IB20Factory::isB20InitializedCall::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::TokenAlreadyExists::SELECTOR,
            canonical::IB20Factory::TokenAlreadyExists::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::UnsupportedVersion::SELECTOR,
            canonical::IB20Factory::UnsupportedVersion::SELECTOR
        );
        assert_eq!(
            super::IB20Factory::B20Created::SIGNATURE_HASH,
            canonical::IB20Factory::B20Created::SIGNATURE_HASH
        );

        assert_eq!(
            super::IB20Factory::B20Variant::COUNT,
            canonical::IB20Factory::B20Variant::COUNT
        );
        assert_eq!(
            super::IB20Factory::B20Variant::ASSET as u8,
            canonical::IB20Factory::B20Variant::ASSET as u8
        );
        assert_eq!(
            super::IB20Factory::B20Variant::STABLECOIN as u8,
            canonical::IB20Factory::B20Variant::STABLECOIN as u8
        );
    }

    /// `PolicyType` ordinals ride the top byte of every policy ID, so they are asserted alongside
    /// the selectors for the same reason as `B20Variant`.
    #[test]
    fn policy_registry_abi_matches_canonical_surface() {
        use alloy_sol_types::SolEnum;

        assert_eq!(
            super::IPolicyRegistry::createPolicyCall::SELECTOR,
            canonical::IPolicyRegistry::createPolicyCall::SELECTOR
        );
        assert_eq!(
            super::IPolicyRegistry::createCompositePolicyCall::SELECTOR,
            canonical::IPolicyRegistry::createCompositePolicyCall::SELECTOR
        );
        assert_eq!(
            super::IPolicyRegistry::isAuthorizedCall::SELECTOR,
            canonical::IPolicyRegistry::isAuthorizedCall::SELECTOR
        );
        assert_eq!(
            super::IPolicyRegistry::updateAllowlistCall::SELECTOR,
            canonical::IPolicyRegistry::updateAllowlistCall::SELECTOR
        );
        assert_eq!(
            super::IPolicyRegistry::InvalidChildPolicy::SELECTOR,
            canonical::IPolicyRegistry::InvalidChildPolicy::SELECTOR
        );
        assert_eq!(
            super::IPolicyRegistry::PolicyCreated::SIGNATURE_HASH,
            canonical::IPolicyRegistry::PolicyCreated::SIGNATURE_HASH
        );
        assert_eq!(
            super::IPolicyRegistry::CompositePolicyUpdated::SIGNATURE_HASH,
            canonical::IPolicyRegistry::CompositePolicyUpdated::SIGNATURE_HASH
        );

        assert_eq!(
            super::IPolicyRegistry::PolicyType::COUNT,
            canonical::IPolicyRegistry::PolicyType::COUNT
        );
        for (mirror, expected) in [
            (
                super::IPolicyRegistry::PolicyType::BLOCKLIST as u8,
                canonical::IPolicyRegistry::PolicyType::BLOCKLIST as u8,
            ),
            (
                super::IPolicyRegistry::PolicyType::ALLOWLIST as u8,
                canonical::IPolicyRegistry::PolicyType::ALLOWLIST as u8,
            ),
            (
                super::IPolicyRegistry::PolicyType::UNION as u8,
                canonical::IPolicyRegistry::PolicyType::UNION as u8,
            ),
            (
                super::IPolicyRegistry::PolicyType::INTERSECT as u8,
                canonical::IPolicyRegistry::PolicyType::INTERSECT as u8,
            ),
        ] {
            assert_eq!(mirror, expected);
        }
    }

    /// Mirroring only the Cobalt surface is safe because it is a strict superset of Beryl's. If
    /// Base ever diverges a shared signature, this fails and V1 needs its own mirror.
    #[test]
    fn policy_registry_covers_v1_selectors() {
        use alloy_sol_types::SolInterface;

        let mirrored: Vec<[u8; 4]> =
            super::IPolicyRegistry::IPolicyRegistryCalls::selectors().collect();
        for selector in canonical::IPolicyRegistryV1::IPolicyRegistryCalls::selectors() {
            assert!(
                mirrored.contains(&selector),
                "Beryl selector {selector:?} is absent from the mirrored Cobalt surface"
            );
        }
    }

    #[test]
    fn b20_extensions_abi_matches_canonical_surface() {
        use alloy_sol_types::SolEnum;

        assert_eq!(
            super::IB20Extensions::mintWithMemoCall::SELECTOR,
            canonical::IB20::mintWithMemoCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::burnWithMemoCall::SELECTOR,
            canonical::IB20::burnWithMemoCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::burnBlockedCall::SELECTOR,
            canonical::IB20::burnBlockedCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::seizeWithMemoCall::SELECTOR,
            canonical::IB20::seizeWithMemoCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::policyIdCall::SELECTOR,
            canonical::IB20::policyIdCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::pauseCall::SELECTOR,
            canonical::IB20::pauseCall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::contractURICall::SELECTOR,
            canonical::IB20::contractURICall::SELECTOR
        );
        assert_eq!(
            super::IB20Extensions::Seized::SIGNATURE_HASH,
            canonical::IB20::Seized::SIGNATURE_HASH
        );
        assert_eq!(
            super::IB20Extensions::Memo::SIGNATURE_HASH,
            canonical::IB20::Memo::SIGNATURE_HASH
        );
        assert_eq!(
            super::IB20Extensions::PolicyUpdated::SIGNATURE_HASH,
            canonical::IB20::PolicyUpdated::SIGNATURE_HASH
        );

        // Pausable ordinals are load-bearing in `pause`/`unpause` calldata.
        assert_eq!(
            super::IB20Extensions::PausableFeature::COUNT,
            canonical::IB20::PausableFeature::COUNT
        );
        assert_eq!(
            super::IB20Extensions::PausableFeature::SEIZE as u8,
            canonical::IB20::PausableFeature::SEIZE as u8
        );
    }

    /// This surface is registered globally rather than per address, so it must never carry the
    /// ERC-20, EIP-2612 or AccessControl members: a competing candidate for those selectors could
    /// change how ordinary token traces decode on every network in a Base-enabled build.
    #[test]
    fn b20_surface_excludes_erc20_members() {
        use alloy_sol_types::SolInterface;

        let mirrored: Vec<[u8; 4]> =
            super::IB20Extensions::IB20ExtensionsCalls::selectors().collect();
        for excluded in [
            canonical::IB20::transferCall::SELECTOR,
            canonical::IB20::transferFromCall::SELECTOR,
            canonical::IB20::approveCall::SELECTOR,
            canonical::IB20::balanceOfCall::SELECTOR,
            canonical::IB20::allowanceCall::SELECTOR,
            canonical::IB20::permitCall::SELECTOR,
            canonical::IB20::hasRoleCall::SELECTOR,
            canonical::IB20::grantRoleCall::SELECTOR,
        ] {
            assert!(
                !mirrored.contains(&excluded),
                "standard selector {excluded:?} must stay out of the global map"
            );
        }
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
