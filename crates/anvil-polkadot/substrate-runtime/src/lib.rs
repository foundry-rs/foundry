#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::disallowed_macros)]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

extern crate alloc;

use crate::sp_runtime::ConsensusEngineId;
use alloc::{vec, vec::Vec};
use currency::*;
use frame_support::weights::{
    Weight,
    constants::{BlockExecutionWeight, ExtrinsicBaseWeight, WEIGHT_REF_TIME_PER_SECOND},
};
use frame_system::limits::BlockWeights;
use pallet_revive::{AccountId32Mapper, evm::runtime::EthExtra};
use pallet_transaction_payment::{FeeDetails, RuntimeDispatchInfo};
use polkadot_sdk::{
    parachains_common::{AccountId, BlockNumber, Hash as CommonHash, Header, Nonce, Signature},
    polkadot_sdk_frame::{
        deps::sp_genesis_builder,
        runtime::{apis, prelude::*},
        traits::FindAuthor,
    },
    sp_consensus_aura::{
        self, SlotDuration, ed25519::AuthorityId as AuraId, runtime_decl_for_aura_api::AuraApiV1,
    },
    *,
};

pub use polkadot_sdk::parachains_common::Balance;
use sp_weights::{ConstantMultiplier, IdentityFee};

pub mod currency {
    use super::Balance;
    pub const MILLICENTS: Balance = 1_000_000_000;
    pub const CENTS: Balance = 1_000 * MILLICENTS;
    pub const DOLLARS: Balance = 100 * CENTS;
}

/// Provides getters for genesis configuration presets.
pub mod genesis_config_presets {
    use super::*;
    use crate::{
        Balance, BalancesConfig, RuntimeGenesisConfig, SudoConfig, currency::DOLLARS,
        sp_keyring::Sr25519Keyring,
    };

    use alloc::{vec, vec::Vec};
    use serde_json::Value;

    pub const ENDOWMENT: Balance = 1_001 * DOLLARS;

    fn well_known_accounts() -> Vec<AccountId> {
        Sr25519Keyring::well_known()
            .map(|k| k.to_account_id())
            .chain([
                // subxt_signer::eth::dev::alith()
                array_bytes::hex_n_into_unchecked(
                    "f24ff3a9cf04c71dbc94d0b566f7a27b94566caceeeeeeeeeeeeeeeeeeeeeeee",
                ),
                // subxt_signer::eth::dev::baltathar()
                array_bytes::hex_n_into_unchecked(
                    "3cd0a705a2dc65e5b1e1205896baa2be8a07c6e0eeeeeeeeeeeeeeeeeeeeeeee",
                ),
            ])
            .collect::<Vec<_>>()
    }

    /// Returns a development genesis config preset.
    pub fn development_config_genesis() -> Value {
        frame_support::build_struct_json_patch!(RuntimeGenesisConfig {
            balances: BalancesConfig {
                balances: well_known_accounts()
                    .into_iter()
                    .map(|id| (id, ENDOWMENT))
                    .collect::<Vec<_>>(),
            },
            sudo: SudoConfig { key: Some(Sr25519Keyring::Alice.to_account_id()) },
        })
    }

    /// Get the set of the available genesis config presets.
    pub fn get_preset(id: &PresetId) -> Option<Vec<u8>> {
        let patch = match id.as_ref() {
            sp_genesis_builder::DEV_RUNTIME_PRESET => development_config_genesis(),
            _ => return None,
        };
        Some(
            serde_json::to_string(&patch)
                .expect("serialization to json is expected to work. qed.")
                .into_bytes(),
        )
    }

    /// List of supported presets.
    pub fn preset_names() -> Vec<PresetId> {
        vec![PresetId::from(sp_genesis_builder::DEV_RUNTIME_PRESET)]
    }
}

/// The runtime version.
#[runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: alloc::borrow::Cow::Borrowed("revive-dev-runtime"),
    impl_name: alloc::borrow::Cow::Borrowed("revive-dev-runtime"),
    authoring_version: 1,
    spec_version: 0,
    impl_version: 1,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 1,
    system_version: 1,
};

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
    NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

/// The address format for describing accounts.
pub type Address = sp_runtime::MultiAddress<AccountId, ()>;
/// Block type as expected by this runtime.
pub type Block = sp_runtime::generic::Block<Header, UncheckedExtrinsic>;
/// Hash type used by the runtime.
pub type Hash = <Runtime as frame_system::Config>::Hash;
/// The transaction extensions that are added to the runtime.
type TxExtension = (
    // Checks that the sender is not the zero address.
    frame_system::CheckNonZeroSender<Runtime>,
    // Checks that the runtime version is correct.
    frame_system::CheckSpecVersion<Runtime>,
    // Checks that the transaction version is correct.
    frame_system::CheckTxVersion<Runtime>,
    // Checks that the genesis hash is correct.
    frame_system::CheckGenesis<Runtime>,
    // Checks that the era is valid.
    frame_system::CheckEra<Runtime>,
    // Checks that the nonce is valid.
    frame_system::CheckNonce<Runtime>,
    // Checks that the weight is valid.
    frame_system::CheckWeight<Runtime>,
    // Ensures that the sender has enough funds to pay for the transaction
    // and deducts the fee from the sender's account.
    pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
    // Reclaim the unused weight from the block using post dispatch information.
    // It must be last in the pipeline in order to catch the refund in previous transaction
    // extensions
    frame_system::WeightReclaim<Runtime>,
);

/// Default extensions applied to Ethereum transactions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EthExtraImpl;

impl EthExtra for EthExtraImpl {
    type Config = Runtime;
    type Extension = TxExtension;

    fn get_eth_extension(nonce: u32, tip: Balance) -> Self::Extension {
        (
            frame_system::CheckNonZeroSender::<Runtime>::new(),
            frame_system::CheckSpecVersion::<Runtime>::new(),
            frame_system::CheckTxVersion::<Runtime>::new(),
            frame_system::CheckGenesis::<Runtime>::new(),
            frame_system::CheckMortality::from(sp_runtime::generic::Era::Immortal),
            frame_system::CheckNonce::<Runtime>::from(nonce),
            frame_system::CheckWeight::<Runtime>::new(),
            pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(tip),
            frame_system::WeightReclaim::<Runtime>::new(),
        )
    }
}

pub type UncheckedExtrinsic =
    pallet_revive::evm::runtime::UncheckedExtrinsic<Address, Signature, EthExtraImpl>;

type Executive = frame_executive::Executive<
    Runtime,
    Block,
    frame_system::ChainContext<Runtime>,
    Runtime,
    AllPalletsWithSystem,
>;

#[frame_construct_runtime]
// Composes the runtime by adding all the used pallets and deriving necessary types.
mod runtime {
    /// The main runtime type.
    #[runtime::runtime]
    #[runtime::derive(
        RuntimeCall,
        RuntimeEvent,
        RuntimeError,
        RuntimeOrigin,
        RuntimeFreezeReason,
        RuntimeHoldReason,
        RuntimeSlashReason,
        RuntimeLockId,
        RuntimeTask,
        RuntimeViewFunction
    )]
    pub struct Runtime;

    /// Mandatory system pallet that should always be included in a FRAME runtime.
    #[runtime::pallet_index(0)]
    pub type System = frame_system::Pallet<Runtime>;

    /// Provides a way for consensus systems to set and check the onchain time.
    #[runtime::pallet_index(1)]
    pub type Timestamp = pallet_timestamp::Pallet<Runtime>;

    /// Provides the ability to keep track of balances.
    #[runtime::pallet_index(2)]
    pub type Balances = pallet_balances::Pallet<Runtime>;

    /// Provides a way to execute privileged functions.
    #[runtime::pallet_index(3)]
    pub type Sudo = pallet_sudo::Pallet<Runtime>;

    /// Provides the ability to charge for extrinsic execution.
    #[runtime::pallet_index(4)]
    pub type TransactionPayment = pallet_transaction_payment::Pallet<Runtime>;

    /// Provides the ability to execute Smart Contracts.
    #[runtime::pallet_index(5)]
    pub type Revive = pallet_revive::Pallet<Runtime>;

    /// Provides the ability to determine AURA authorities for block building.
    #[runtime::pallet_index(6)]
    pub type Aura = pallet_aura::Pallet<Runtime>;
}

impl pallet_aura::Config for Runtime {
    type AuthorityId = AuraId;
    type DisabledValidators = ();
    type MaxAuthorities = ConstU32<1>;
    type AllowMultipleBlocksPerSlot = ConstBool<true>;
    // Not relevant in general since the node digest
    // will refer to slot 0 always.
    type SlotDuration = ConstU64<6000>;
}

/// We assume that ~10% of the block weight is consumed by `on_initialize` handlers.
/// This is used to limit the maximal weight of a single extrinsic.
const AVERAGE_ON_INITIALIZE_RATIO: Perbill = Perbill::from_percent(10);
/// We allow `Normal` extrinsics to fill up the block up to 75%, the rest can be used
/// by  Operational  extrinsics.
const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);
/// We allow for 2 seconds of compute with a 6 second average block time, with maximum proof size.
const MAXIMUM_BLOCK_WEIGHT: Weight =
    Weight::from_parts(WEIGHT_REF_TIME_PER_SECOND.saturating_mul(2), u64::MAX);

parameter_types! {
    pub const Version: RuntimeVersion = VERSION;
    pub RuntimeBlockWeights: BlockWeights = BlockWeights::builder()
        .base_block(BlockExecutionWeight::get())
        .for_class(DispatchClass::all(), |weights| {
            weights.base_extrinsic = ExtrinsicBaseWeight::get();
        })
        .for_class(DispatchClass::Normal, |weights| {
            weights.max_total = Some(NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT);
        })
        .for_class(DispatchClass::Operational, |weights| {
            weights.max_total = Some(MAXIMUM_BLOCK_WEIGHT);
            // Operational transactions have some extra reserved space, so that they
            // are included even if block reached `MAXIMUM_BLOCK_WEIGHT`.
            weights.reserved = Some(
                MAXIMUM_BLOCK_WEIGHT - NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT
            );
        })
        .avg_block_initialization(AVERAGE_ON_INITIALIZE_RATIO)
        .build_or_panic();
}

/// Implements the types required for the system pallet.
#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
    type Block = Block;
    type Version = Version;
    type AccountId = AccountId;
    type Hash = CommonHash;
    type Nonce = Nonce;
    type AccountData = pallet_balances::AccountData<<Runtime as pallet_balances::Config>::Balance>;
}

parameter_types! {
    pub const ExistentialDeposit: Balance = DOLLARS;
}

// Implements the types required for the balances pallet.
#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Runtime {
    type AccountStore = System;
    type Balance = Balance;
    type ExistentialDeposit = ExistentialDeposit;
}

// Implements the types required for the sudo pallet.
#[derive_impl(pallet_sudo::config_preludes::TestDefaultConfig)]
impl pallet_sudo::Config for Runtime {}

// Implements the types required for the sudo pallet.
#[derive_impl(pallet_timestamp::config_preludes::TestDefaultConfig)]
impl pallet_timestamp::Config for Runtime {}

parameter_types! {
    pub const TransactionByteFee: Balance = 10 * MILLICENTS;
}

// Implements the types required for the transaction payment pallet.
#[derive_impl(pallet_transaction_payment::config_preludes::TestDefaultConfig)]
impl pallet_transaction_payment::Config for Runtime {
    type OnChargeTransaction = pallet_transaction_payment::FungibleAdapter<Balances, ()>;
    type WeightToFee = IdentityFee<Balance>;
    type LengthToFee = ConstantMultiplier<Balance, TransactionByteFee>;
}

parameter_types! {
    pub CodeHashLockupDepositPercent: Perbill = Perbill::from_percent(30);
    pub storage ChainId: u64 = 420_420_420;
    pub storage Author: [u8; 32] = [0xEE;32];
}

impl FindAuthor<AccountId> for Author {
    fn find_author<'a, I>(_digests: I) -> Option<AccountId>
    where
        I: 'a + IntoIterator<Item = (ConsensusEngineId, &'a [u8])>,
    {
        let authorities = Runtime::authorities();
        let key = authorities[0].clone().into_inner();
        Some(key.into())
    }
}

#[derive_impl(pallet_revive::config_preludes::TestDefaultConfig)]
impl pallet_revive::Config for Runtime {
    type AddressMapper = AccountId32Mapper<Self>;
    type ChainId = ChainId;
    type CodeHashLockupDepositPercent = CodeHashLockupDepositPercent;
    type Currency = Balances;
    type FindAuthor = Author;
    type NativeToEthRatio = ConstU32<1_000_000>;
    type UploadOrigin = EnsureSigned<Self::AccountId>;
    type InstantiateOrigin = EnsureSigned<Self::AccountId>;
    type Time = Timestamp;
}

pallet_revive::impl_runtime_apis_plus_revive!(
    Runtime,
    Executive,
    EthExtraImpl,

    impl apis::Core<Block> for Runtime {
        fn version() -> RuntimeVersion {
            VERSION
        }

        fn execute_block(block: Block) {
            Executive::execute_block(block)
        }

        fn initialize_block(header: &Header) -> ExtrinsicInclusionMode {
            Executive::initialize_block(header)
        }
    }

    impl apis::Metadata<Block> for Runtime {
        fn metadata() -> OpaqueMetadata {
            OpaqueMetadata::new(Runtime::metadata().into())
        }

        fn metadata_at_version(version: u32) -> Option<OpaqueMetadata> {
            Runtime::metadata_at_version(version)
        }

        fn metadata_versions() -> Vec<u32> {
            Runtime::metadata_versions()
        }
    }

    impl apis::BlockBuilder<Block> for Runtime {
        fn apply_extrinsic(extrinsic: ExtrinsicFor<Runtime>) -> ApplyExtrinsicResult {
            Executive::apply_extrinsic(extrinsic)
        }

        fn finalize_block() -> HeaderFor<Runtime> {
            Executive::finalize_block()
        }

        fn inherent_extrinsics(data: InherentData) -> Vec<ExtrinsicFor<Runtime>> {
            data.create_extrinsics()
        }

        fn check_inherents(
            block: Block,
            data: InherentData,
        ) -> CheckInherentsResult {
            data.check_extrinsics(&block)
        }
    }

    impl apis::TaggedTransactionQueue<Block> for Runtime {
        fn validate_transaction(
            source: TransactionSource,
            tx: ExtrinsicFor<Runtime>,
            block_hash: <Runtime as frame_system::Config>::Hash,
        ) -> TransactionValidity {
            Executive::validate_transaction(source, tx, block_hash)
        }
    }

    impl apis::OffchainWorkerApi<Block> for Runtime {
        fn offchain_worker(header: &HeaderFor<Runtime>) {
            Executive::offchain_worker(header)
        }
    }

    impl apis::SessionKeys<Block> for Runtime {
        fn generate_session_keys(_seed: Option<Vec<u8>>) -> Vec<u8> {
            Default::default()
        }

        fn decode_session_keys(
            _encoded: Vec<u8>,
        ) -> Option<Vec<(Vec<u8>, apis::KeyTypeId)>> {
            Default::default()
        }
    }

    impl apis::AccountNonceApi<Block, AccountId, Nonce> for Runtime {
        fn account_nonce(account: AccountId) -> Nonce {
            System::account_nonce(account)
        }
    }

    impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<
        Block,
        Balance,
    > for Runtime {
        fn query_info(uxt: ExtrinsicFor<Runtime>, len: u32) -> RuntimeDispatchInfo<Balance> {
            TransactionPayment::query_info(uxt, len)
        }
        fn query_fee_details(uxt: ExtrinsicFor<Runtime>, len: u32) -> FeeDetails<Balance> {
            TransactionPayment::query_fee_details(uxt, len)
        }
        fn query_weight_to_fee(weight: Weight) -> Balance {
            TransactionPayment::weight_to_fee(weight)
        }
        fn query_length_to_fee(length: u32) -> Balance {
            TransactionPayment::length_to_fee(length)
        }
    }

    impl apis::GenesisBuilder<Block> for Runtime {
        fn build_state(config: Vec<u8>) -> sp_genesis_builder::Result {
            build_state::<RuntimeGenesisConfig>(config)
        }

        fn get_preset(id: &Option<PresetId>) -> Option<Vec<u8>> {
            get_preset::<RuntimeGenesisConfig>(id, self::genesis_config_presets::get_preset)
        }

        fn preset_names() -> Vec<PresetId> {
            self::genesis_config_presets::preset_names()
        }
    }

    impl sp_consensus_aura::AuraApi<Block, AuraId> for Runtime {
        fn slot_duration() -> SlotDuration {
            // This is not relevant when considering a manual-seal
            // driven node. The slot duration is used by Aura to determine
            // the authority, but anvil-polkadot will provide same slot and
            // not use this API to determine the correct slot.
            SlotDuration::from_millis(6000)
        }

        fn authorities() -> Vec<AuraId> {
            pallet_aura::Authorities::<Runtime>::get().into_inner()
        }
    }

);
