//! Cheatcode state shared by both execution engines.

use crate::{
    CheatcodeAnalysis, CheatsConfig, ExpectedCallTracker, Wallets,
    evm::mock::{MockCallDataContext, MockCallReturnData},
    inspector::ArbitraryStorage,
};
use alloy_primitives::{
    Address, Bytes, U256,
    map::{AddressHashMap, HashMap},
};
use alloy_rpc_types::AccessList;
use alloy_signer_local::PrivateKeySigner;
use foundry_common::mapping_slots::MappingSlots;
use foundry_wallets::wallet_multi::MultiWallet;
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use rand::Rng;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::Arc,
};

/// Cheatcode state that does not depend on the execution engine.
#[derive(Clone, Debug)]
pub struct CheatcodesCore {
    /// Additional, user configurable context this Inspector has access to when inspecting a call.
    pub config: Arc<CheatsConfig>,
    /// Solar compiler instance, to grant syntactic and semantic analysis capabilities
    pub analysis: Option<CheatcodeAnalysis>,
    /// Address labels
    pub labels: AddressHashMap<String>,
    /// Test-scoped context holding data that needs to be reset every test run
    pub test_context: TestContext,
    /// Whether to commit FS changes such as file creations, writes and deletes.
    /// Used to prevent duplicate changes file executing non-committing calls.
    pub fs_commit: bool,
    /// Serialized JSON values.
    // **Note**: both must a BTreeMap to ensure the order of the keys is deterministic.
    pub serialized_jsons: BTreeMap<String, BTreeMap<String, Value>>,
    /// Optional cheatcodes `TestRunner`. Used for generating random values from uint and int
    /// strategies.
    test_runner: Option<TestRunner>,
    /// Unlocked wallets used in scripts and testing of scripts.
    pub wallets: Option<Wallets>,
    /// Parsed secp256k1 private-key signers for repeated `vm.addr` / `vm.sign` calls.
    pub private_key_signers: HashMap<U256, PrivateKeySigner>,
    /// Mocked calls
    // **Note**: inner must a BTreeMap because of special `Ord` impl for `MockCallDataContext`
    pub mocked_calls: HashMap<Address, BTreeMap<MockCallDataContext, VecDeque<MockCallReturnData>>>,
    /// Mocked functions. Maps target address to be mocked to pair of (calldata, mock address).
    pub mocked_functions: HashMap<Address, HashMap<Bytes, Address>>,
    /// Expected calls
    pub expected_calls: ExpectedCallTracker,
    /// Mapping slots.
    pub mapping_slots: Option<AddressHashMap<MappingSlots>>,
    /// Recorded logs
    pub recorded_logs: Option<Vec<crate::Vm::Log>>,
    /// Bumped by `recordLogs` and `getRecordedLogs*` so open frame checkpoints know the buffer
    /// was reset.
    pub recorded_logs_generation: u64,
    /// Current EIP-2930 access lists.
    pub access_list: Option<AccessList>,
    /// Whether the next contract creation should be intercepted to return its initcode.
    pub intercept_next_create_call: bool,
    /// Addresses with arbitrary storage.
    pub arbitrary_storage: Option<ArbitraryStorage>,
}

impl Default for CheatcodesCore {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl CheatcodesCore {
    /// Creates a new `CheatcodesCore` with the given settings.
    pub fn new(config: Arc<CheatsConfig>) -> Self {
        Self {
            analysis: None,
            labels: config.labels.clone(),
            config,
            test_context: Default::default(),
            fs_commit: true,
            serialized_jsons: Default::default(),
            test_runner: Default::default(),
            wallets: Default::default(),
            private_key_signers: Default::default(),
            mocked_calls: Default::default(),
            mocked_functions: Default::default(),
            expected_calls: Default::default(),
            mapping_slots: Default::default(),
            recorded_logs: Default::default(),
            recorded_logs_generation: Default::default(),
            access_list: Default::default(),
            intercept_next_create_call: Default::default(),
            arbitrary_storage: Default::default(),
        }
    }

    /// Enables cheatcode analysis capabilities by providing a solar compiler instance.
    pub fn set_analysis(&mut self, analysis: CheatcodeAnalysis) {
        self.analysis = Some(analysis);
    }

    /// Returns the configured wallets if available, else creates a new instance.
    pub fn wallets(&mut self) -> &Wallets {
        self.wallets.get_or_insert_with(|| Wallets::new(MultiWallet::default(), None))
    }

    /// Sets the unlocked wallets.
    pub fn set_wallets(&mut self, wallets: Wallets) {
        self.wallets = Some(wallets);
    }

    pub fn rng(&mut self) -> &mut impl Rng {
        self.test_runner().rng()
    }

    pub fn test_runner(&mut self) -> &mut TestRunner {
        self.test_runner.get_or_insert_with(|| match self.config.seed {
            Some(seed) => TestRunner::new_with_rng(
                proptest::test_runner::Config::default(),
                TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>()),
            ),
            None => TestRunner::new(proptest::test_runner::Config::default()),
        })
    }

    pub fn set_seed(&mut self, seed: U256) {
        self.test_runner = Some(TestRunner::new_with_rng(
            proptest::test_runner::Config::default(),
            TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>()),
        ));
    }

    /// Returns struct definitions from the analysis, if available.
    pub fn struct_defs(&self) -> Option<&foundry_common::fmt::StructDefinitions> {
        self.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok())
    }

    /// Returns existing or set a default `ArbitraryStorage` option.
    /// Used by `setArbitraryStorage` cheatcode to track addresses with arbitrary storage.
    pub fn arbitrary_storage(&mut self) -> &mut ArbitraryStorage {
        self.arbitrary_storage.get_or_insert_with(ArbitraryStorage::default)
    }
}

/// Contains additional, test specific resources that should be kept for the duration of the test
#[derive(Debug, Default)]
pub struct TestContext {
    /// Buffered readers for files opened for reading (path => BufReader mapping)
    pub opened_read_files: HashMap<PathBuf, BufReader<File>>,
}

/// Every time we clone `Context`, we want it to be empty
impl Clone for TestContext {
    fn clone(&self) -> Self {
        Default::default()
    }
}

impl TestContext {
    /// Clears the context.
    pub fn clear(&mut self) {
        self.opened_read_files.clear();
    }
}
