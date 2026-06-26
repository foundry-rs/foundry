use crate::{
    CallTrace, CallTraceArena, CallTraceNode, DecodedCallData,
    debug::DebugTraceIdentifier,
    identifier::{IdentifiedAddress, LocalTraceIdentifier, SignaturesIdentifier, TraceIdentifier},
};
use alloy_dyn_abi::{DecodedEvent, DynSolValue, EventExt, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Error, Event, Function, JsonAbi};
use alloy_primitives::{
    Address, B256, LogData, Selector, U256,
    map::{AddressHashMap, HashMap, HashSet},
};
use alloy_sol_types::SolValue;
use foundry_common::{
    ContractsByArtifact, SELECTOR_LEN, abi::get_indexed_event, fmt::format_token,
    get_contract_name, selectors::SelectorKind,
};
use foundry_evm_core::{
    abi::{Vm, console},
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS},
    decode::RevertDecoder,
    precompiles::{
        BLAKE_2F, BLS12_G1ADD, BLS12_G1MSM, BLS12_G2ADD, BLS12_G2MSM, BLS12_MAP_FP_TO_G1,
        BLS12_MAP_FP2_TO_G2, BLS12_PAIRING_CHECK, EC_ADD, EC_MUL, EC_PAIRING, EC_RECOVER, IDENTITY,
        MOD_EXP, P256_VERIFY, POINT_EVALUATION, RIPEMD_160, SHA_256,
    },
};
use foundry_evm_hardforks::TempoHardfork;
use itertools::Itertools;
use revm_inspectors::tracing::types::{DecodedCallLog, DecodedCallTrace};
use std::{collections::BTreeMap, sync::OnceLock};
use tempo_contracts::precompiles::{
    IAccountKeychain, IAddressRegistry, IFeeManager, IReceivePolicyGuard, ISignatureVerifier,
    IStablecoinDEX, ITIP20ChannelReserve, ITIP20Factory, ITIP403Registry, IValidatorConfig,
};
use tempo_precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, ADDRESS_REGISTRY_ADDRESS, NONCE_PRECOMPILE_ADDRESS, PATH_USD_ADDRESS,
    RECEIVE_POLICY_GUARD_ADDRESS, SIGNATURE_VERIFIER_ADDRESS, STABLECOIN_DEX_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS,
    TIP403_REGISTRY_ADDRESS, VALIDATOR_CONFIG_ADDRESS, nonce::INonce, tip20::ITIP20,
};

mod precompiles;

/// Build a new [CallTraceDecoder].
#[derive(Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct CallTraceDecoderBuilder {
    decoder: CallTraceDecoder,
}

impl CallTraceDecoderBuilder {
    /// Create a new builder.
    #[inline]
    pub fn new() -> Self {
        Self { decoder: CallTraceDecoder::new().clone() }
    }

    /// Add known labels to the decoder.
    #[inline]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = (Address, String)>) -> Self {
        self.decoder.labels.extend(labels);
        self
    }

    /// Add known errors to the decoder.
    #[inline]
    pub fn with_abi(mut self, abi: &JsonAbi) -> Self {
        self.decoder.collect_abi(abi, None);
        self
    }

    /// Add known contracts to the decoder.
    #[inline]
    pub fn with_known_contracts(mut self, contracts: &ContractsByArtifact) -> Self {
        trace!(target: "evm::traces", len=contracts.len(), "collecting known contract ABIs");
        for contract in contracts.values() {
            self.decoder.collect_abi(&contract.abi, None);
        }
        self
    }

    /// Add known contracts to the decoder from a `LocalTraceIdentifier`.
    #[inline]
    pub fn with_local_identifier_abis(self, identifier: &LocalTraceIdentifier<'_>) -> Self {
        self.with_known_contracts(identifier.contracts())
    }

    /// Sets the verbosity level of the decoder.
    #[inline]
    pub const fn with_verbosity(mut self, level: u8) -> Self {
        self.decoder.verbosity = level;
        self
    }

    /// Sets the signature identifier for events and functions.
    #[inline]
    pub fn with_signature_identifier(mut self, identifier: SignaturesIdentifier) -> Self {
        self.decoder.signature_identifier = Some(identifier);
        self
    }

    /// Sets the signature identifier for events and functions.
    #[inline]
    pub const fn with_label_disabled(mut self, disable_alias: bool) -> Self {
        self.decoder.disable_labels = disable_alias;
        self
    }

    /// Sets the chain ID for network-specific precompile detection.
    #[inline]
    pub const fn with_chain_id(mut self, chain_id: Option<u64>) -> Self {
        self.decoder.chain_id = chain_id;
        self
    }

    /// Sets the Tempo hardfork for hardfork-specific precompile detection.
    #[inline]
    pub fn with_tempo_hardfork(mut self, hardfork: Option<TempoHardfork>) -> Self {
        self.decoder.tempo_hardfork = hardfork;
        if hardfork.is_some_and(|hardfork| hardfork.is_t5()) {
            self.decoder
                .labels
                .entry(TIP20_CHANNEL_RESERVE_ADDRESS)
                .or_insert_with(|| "TIP20ChannelReserve".to_string());
        }
        if hardfork.is_some_and(|hardfork| hardfork.is_t6()) {
            self.decoder
                .labels
                .entry(RECEIVE_POLICY_GUARD_ADDRESS)
                .or_insert_with(|| "ReceivePolicyGuard".to_string());
        }
        self
    }

    /// Sets the debug identifier for the decoder.
    #[inline]
    pub fn with_debug_identifier(mut self, identifier: DebugTraceIdentifier) -> Self {
        self.decoder.debug_identifier = Some(identifier);
        self
    }

    /// Build the decoder.
    #[inline]
    pub fn build(self) -> CallTraceDecoder {
        self.decoder
    }
}

/// The call trace decoder.
///
/// The decoder collects address labels and ABIs from any number of [TraceIdentifier]s, which it
/// then uses to decode the call trace.
///
/// Note that a call trace decoder is required for each new set of traces, since addresses in
/// different sets might overlap.
#[derive(Clone, Debug, Default)]
pub struct CallTraceDecoder {
    /// Addresses identified to be a specific contract.
    ///
    /// The values are in the form `"<artifact>:<contract>"`.
    pub contracts: HashMap<Address, String>,
    /// Address labels.
    pub labels: HashMap<Address, String>,
    /// Contract addresses that have a receive function.
    pub receive_contracts: HashSet<Address>,
    /// Contract addresses that have fallback functions, mapped to function selectors of that
    /// contract.
    pub fallback_contracts: HashMap<Address, HashSet<Selector>>,
    /// Contract addresses that have do NOT have fallback functions, mapped to function selectors
    /// of that contract.
    pub non_fallback_contracts: HashMap<Address, HashSet<Selector>>,

    /// All known functions.
    pub functions: HashMap<Selector, Vec<Function>>,
    /// Functions identified for a specific contract address.
    pub functions_by_address: HashMap<Address, HashMap<Selector, Vec<Function>>>,
    /// All known events.
    ///
    /// Key is: `(topics[0], topics.len() - 1)`.
    pub events: BTreeMap<(B256, usize), Vec<Event>>,
    /// Revert decoder. Contains all known custom errors.
    pub revert_decoder: RevertDecoder,

    /// A signature identifier for events and functions.
    pub signature_identifier: Option<SignaturesIdentifier>,
    /// Verbosity level
    pub verbosity: u8,

    /// Optional identifier of individual trace steps.
    pub debug_identifier: Option<DebugTraceIdentifier>,

    /// Disable showing of labels.
    pub disable_labels: bool,

    /// The chain ID, used to determine network-specific precompiles.
    pub chain_id: Option<u64>,

    /// The Tempo hardfork, used to determine hardfork-specific precompiles.
    pub tempo_hardfork: Option<TempoHardfork>,
}

impl CallTraceDecoder {
    /// Creates a new call trace decoder.
    ///
    /// The call trace decoder always knows how to decode calls to the cheatcode address, as well
    /// as DSTest-style logs.
    pub fn new() -> &'static Self {
        // If you want to take arguments in this function, assign them to the fields of the cloned
        // lazy instead of removing it
        static INIT: OnceLock<CallTraceDecoder> = OnceLock::new();
        INIT.get_or_init(Self::init)
    }

    #[instrument(name = "CallTraceDecoder::init", level = "debug")]
    fn init() -> Self {
        // Materialized once so the revert decoder can take references below.
        let tempo_abis = [
            IFeeManager::abi::contract(),
            ITIP20::abi::contract(),
            ITIP403Registry::abi::contract(),
            ITIP20Factory::abi::contract(),
            IStablecoinDEX::abi::contract(),
            INonce::abi::contract(),
            IValidatorConfig::abi::contract(),
            IAccountKeychain::abi::contract(),
            IAddressRegistry::abi::contract(),
            ITIP20ChannelReserve::abi::contract(),
            ISignatureVerifier::abi::contract(),
            IReceivePolicyGuard::abi::contract(),
        ];
        Self {
            contracts: Default::default(),
            labels: HashMap::from_iter([
                (CHEATCODE_ADDRESS, "VM".to_string()),
                (HARDHAT_CONSOLE_ADDRESS, "console".to_string()),
                (DEFAULT_CREATE2_DEPLOYER, "Create2Deployer".to_string()),
                (CALLER, "DefaultSender".to_string()),
                (EC_RECOVER, "ECRecover".to_string()),
                (SHA_256, "SHA-256".to_string()),
                (RIPEMD_160, "RIPEMD-160".to_string()),
                (IDENTITY, "Identity".to_string()),
                (MOD_EXP, "ModExp".to_string()),
                (EC_ADD, "ECAdd".to_string()),
                (EC_MUL, "ECMul".to_string()),
                (EC_PAIRING, "ECPairing".to_string()),
                (BLAKE_2F, "Blake2F".to_string()),
                (POINT_EVALUATION, "PointEvaluation".to_string()),
                (BLS12_G1ADD, "BLS12_G1ADD".to_string()),
                (BLS12_G1MSM, "BLS12_G1MSM".to_string()),
                (BLS12_G2ADD, "BLS12_G2ADD".to_string()),
                (BLS12_G2MSM, "BLS12_G2MSM".to_string()),
                (BLS12_PAIRING_CHECK, "BLS12_PAIRING_CHECK".to_string()),
                (BLS12_MAP_FP_TO_G1, "BLS12_MAP_FP_TO_G1".to_string()),
                (BLS12_MAP_FP2_TO_G2, "BLS12_MAP_FP2_TO_G2".to_string()),
                (P256_VERIFY, "P256VERIFY".to_string()),
                // Tempo
                (TIP_FEE_MANAGER_ADDRESS, "FeeManager".to_string()),
                (TIP403_REGISTRY_ADDRESS, "TIP403Registry".to_string()),
                (TIP20_FACTORY_ADDRESS, "TIP20Factory".to_string()),
                (STABLECOIN_DEX_ADDRESS, "StablecoinDex".to_string()),
                (NONCE_PRECOMPILE_ADDRESS, "Nonce".to_string()),
                (VALIDATOR_CONFIG_ADDRESS, "ValidatorConfig".to_string()),
                (ACCOUNT_KEYCHAIN_ADDRESS, "AccountKeychain".to_string()),
                (ADDRESS_REGISTRY_ADDRESS, "AddressRegistry".to_string()),
                (TIP20_CHANNEL_RESERVE_ADDRESS, "TIP20ChannelReserve".to_string()),
                (SIGNATURE_VERIFIER_ADDRESS, "SignatureVerifier".to_string()),
                (RECEIVE_POLICY_GUARD_ADDRESS, "ReceivePolicyGuard".to_string()),
                (PATH_USD_ADDRESS, "PathUSD".to_string()),
            ]),
            receive_contracts: Default::default(),
            fallback_contracts: Default::default(),
            non_fallback_contracts: Default::default(),

            functions: console::hh::abi::functions()
                .into_values()
                .chain(Vm::abi::functions().into_values())
                // Tempo
                .chain(IFeeManager::abi::functions().into_values())
                .chain(ITIP20::abi::functions().into_values())
                .chain(ITIP403Registry::abi::functions().into_values())
                .chain(ITIP20Factory::abi::functions().into_values())
                .chain(IStablecoinDEX::abi::functions().into_values())
                .chain(INonce::abi::functions().into_values())
                .chain(IValidatorConfig::abi::functions().into_values())
                .chain(IAccountKeychain::abi::functions().into_values())
                .chain(IAddressRegistry::abi::functions().into_values())
                .chain(ITIP20ChannelReserve::abi::functions().into_values())
                .chain(ISignatureVerifier::abi::functions().into_values())
                .chain(IReceivePolicyGuard::abi::functions().into_values())
                .flatten()
                .map(|func| (func.selector(), vec![func]))
                .collect(),
            functions_by_address: Default::default(),
            events: console::ds::abi::events()
                .into_values()
                // Tempo
                .chain(IFeeManager::abi::events().into_values())
                .chain(ITIP20::abi::events().into_values())
                .chain(ITIP403Registry::abi::events().into_values())
                .chain(ITIP20Factory::abi::events().into_values())
                .chain(IStablecoinDEX::abi::events().into_values())
                .chain(INonce::abi::events().into_values())
                .chain(IValidatorConfig::abi::events().into_values())
                .chain(IAccountKeychain::abi::events().into_values())
                .chain(IAddressRegistry::abi::events().into_values())
                .chain(ITIP20ChannelReserve::abi::events().into_values())
                .chain(ISignatureVerifier::abi::events().into_values())
                .chain(IReceivePolicyGuard::abi::events().into_values())
                .flatten()
                .map(|event| ((event.selector(), indexed_inputs(&event)), vec![event]))
                .collect(),
            // Decode Tempo precompile custom errors by name in traces.
            revert_decoder: RevertDecoder::new().with_abis(tempo_abis.iter()),

            signature_identifier: None,
            verbosity: 0,

            debug_identifier: None,

            disable_labels: false,

            chain_id: None,

            tempo_hardfork: None,
        }
    }

    /// Clears all known addresses.
    pub fn clear_addresses(&mut self) {
        self.contracts.clear();

        let default_labels = &Self::new().labels;
        if self.labels.len() > default_labels.len() {
            self.labels.clone_from(default_labels);
        }

        self.receive_contracts.clear();
        self.fallback_contracts.clear();
        self.non_fallback_contracts.clear();
        self.functions_by_address.clear();
    }

    /// Returns labels for precompiles active in this decoder's chain context.
    pub fn precompile_labels(&self) -> AddressHashMap<String> {
        self.labels
            .iter()
            .filter(|(address, _)| {
                precompiles::is_known_precompile(**address, self.chain_id, self.tempo_hardfork)
            })
            .map(|(address, label)| (*address, label.clone()))
            .collect()
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    pub fn identify(&mut self, arena: &CallTraceArena, identifier: &mut impl TraceIdentifier) {
        self.collect_identified_addresses(self.identify_addresses(arena, identifier));
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    pub fn identify_addresses<'a>(
        &self,
        arena: &CallTraceArena,
        identifier: &'a mut impl TraceIdentifier,
    ) -> Vec<IdentifiedAddress<'a>> {
        let nodes = arena.nodes().iter().filter(|node| {
            // Skip precompile addresses, they will never resolve externally.
            if node.is_precompile()
                || precompiles::is_known_precompile(
                    node.trace.address,
                    self.chain_id,
                    self.tempo_hardfork,
                )
            {
                return false;
            }
            let address = &node.trace.address;
            !self.labels.contains_key(address) || !self.contracts.contains_key(address)
        });
        identifier.identify_addresses(&nodes.collect::<Vec<_>>())
    }

    /// Adds a single event to the decoder.
    pub fn push_event(&mut self, event: Event) {
        self.events.entry((event.selector(), indexed_inputs(&event))).or_default().push(event);
    }

    /// Adds a single function to the decoder.
    pub fn push_function(&mut self, function: Function) {
        let selector = function.selector();
        let functions = self.functions.entry(selector).or_default();

        if Self::push_function_to(functions, function) && functions.len() > 1 {
            let function = functions.last().expect("function was just inserted");
            let signature = function.signature();
            trace!(target: "evm::traces", %selector, new=%signature, "duplicate function selector");
        }
    }

    /// Adds a single function to the decoder for a specific contract address.
    pub fn push_address_function(&mut self, address: Address, function: Function) {
        let functions = self
            .functions_by_address
            .entry(address)
            .or_default()
            .entry(function.selector())
            .or_default();
        Self::push_function_to(functions, function);
    }

    fn push_function_to(functions: &mut Vec<Function>, function: Function) -> bool {
        if functions.contains(&function) {
            false
        } else {
            functions.push(function);
            true
        }
    }

    fn functions_for_selector(&self, address: Address, selector: &Selector) -> Option<&[Function]> {
        self.functions_by_address
            .get(&address)
            .and_then(|functions| functions.get(selector))
            .or_else(|| self.functions.get(selector))
            .map(Vec::as_slice)
    }

    /// Selects the appropriate function from a list of functions with the same selector by
    /// checking which one decodes the calldata.
    ///
    /// Address-scoped function lookup should happen before this to avoid using ABI metadata from a
    /// different contract when multiple functions have the same input types.
    fn select_contract_function<'a>(
        &self,
        functions: &'a [Function],
        trace: &CallTrace,
    ) -> &'a [Function] {
        // When there are selector collisions, try to decode the calldata with each function
        // to determine which one is actually being called. The correct function should
        // decode successfully while the wrong ones will fail due to parameter type mismatches.
        if functions.len() > 1 {
            for (i, func) in functions.iter().enumerate() {
                if trace.data.len() >= SELECTOR_LEN
                    && func.abi_decode_input(&trace.data[SELECTOR_LEN..]).is_ok()
                {
                    return &functions[i..i + 1];
                }
            }
        }
        functions
    }

    /// Adds a single error to the decoder.
    pub fn push_error(&mut self, error: Error) {
        self.revert_decoder.push_error(error);
    }

    pub const fn without_label(&mut self, disable: bool) {
        self.disable_labels = disable;
    }

    fn collect_identified_addresses(&mut self, mut addrs: Vec<IdentifiedAddress<'_>>) {
        addrs.sort_by_key(|identity| identity.address);
        addrs.dedup_by_key(|identity| identity.address);
        if addrs.is_empty() {
            return;
        }

        trace!(target: "evm::traces", len=addrs.len(), "collecting address identities");
        for IdentifiedAddress { address, label, contract, abi, artifact_id: _ } in addrs {
            let _span = trace_span!(target: "evm::traces", "identity", ?contract, ?label).entered();

            if let Some(contract) = contract {
                self.contracts.entry(address).or_insert(contract);
            }

            if let Some(label) = label.filter(|s| !s.is_empty()) {
                self.labels.entry(address).or_insert(label);
            }

            if let Some(abi) = abi {
                self.collect_abi(&abi, Some(address));
            }
        }
    }

    fn collect_abi(&mut self, abi: &JsonAbi, address: Option<Address>) {
        let len = abi.len();
        if len == 0 {
            return;
        }
        trace!(target: "evm::traces", len, ?address, "collecting ABI");
        for function in abi.functions() {
            if let Some(address) = address {
                self.push_address_function(address, function.clone());
            }
            self.push_function(function.clone());
        }
        for event in abi.events() {
            self.push_event(event.clone());
        }
        for error in abi.errors() {
            self.push_error(error.clone());
        }
        if let Some(address) = address {
            if abi.receive.is_some() {
                self.receive_contracts.insert(address);
            }

            if abi.fallback.is_some() {
                self.fallback_contracts
                    .insert(address, abi.functions().map(|f| f.selector()).collect());
            } else {
                self.non_fallback_contracts
                    .insert(address, abi.functions().map(|f| f.selector()).collect());
            }
        }
    }

    /// Populates the traces with decoded data by mutating the
    /// [CallTrace] in place. See [CallTraceDecoder::decode_function] and
    /// [CallTraceDecoder::decode_event] for more details.
    pub async fn populate_traces(&self, traces: &mut Vec<CallTraceNode>) {
        for node in traces {
            node.trace.decoded = Some(Box::new(self.decode_function(&node.trace).await));
            for log in &mut node.logs {
                log.decoded =
                    Some(Box::new(self.decode_event_with_address(log.address, &log.raw_log).await));
            }

            if let Some(debug) = self.debug_identifier.as_ref()
                && let Some(identified) = self.contracts.get(&node.trace.address)
            {
                debug.identify_node_steps(node, get_contract_name(identified))
            }
        }
    }

    /// Decodes a call trace.
    pub async fn decode_function(&self, trace: &CallTrace) -> DecodedCallTrace {
        let label =
            if self.disable_labels { None } else { self.labels.get(&trace.address).cloned() };

        if trace.kind.is_any_create() {
            return DecodedCallTrace { label, ..Default::default() };
        }

        if let Some(trace) = precompiles::decode(trace, self.chain_id, self.tempo_hardfork) {
            return trace;
        }

        let cdata = &trace.data;
        if trace.address == DEFAULT_CREATE2_DEPLOYER {
            return DecodedCallTrace {
                label,
                call_data: Some(DecodedCallData { signature: "create2".to_string(), args: vec![] }),
                return_data: self.default_return_data(trace),
            };
        }

        if is_abi_call_data(cdata) {
            let selector = Selector::try_from(&cdata[..SELECTOR_LEN]).unwrap();
            let mut identified_functions = Vec::new();
            let functions = match self.functions_for_selector(trace.address, &selector) {
                Some(functions) => functions,
                None => {
                    if let Some(identifier) = &self.signature_identifier
                        && let Some(function) = identifier.identify_function(selector).await
                    {
                        identified_functions.push(function);
                    }
                    &identified_functions
                }
            };

            // Check if unsupported fn selector: calldata dooes NOT point to one of its selectors +
            // non-fallback contract + no receive
            if let Some(contract_selectors) = self.non_fallback_contracts.get(&trace.address)
                && !contract_selectors.contains(&selector)
                && (!cdata.is_empty() || !self.receive_contracts.contains(&trace.address))
            {
                let return_data = if trace.success {
                    None
                } else {
                    let revert_msg = self.revert_decoder.decode(&trace.output, trace.status);

                    if trace.output.is_empty() || revert_msg.contains("EvmError: Revert") {
                        Some(format!(
                            "unrecognized function selector {} for contract {}, which has no fallback function.",
                            selector, trace.address
                        ))
                    } else {
                        Some(revert_msg)
                    }
                };

                return if let Some(func) = functions.first() {
                    DecodedCallTrace {
                        label,
                        call_data: Some(self.decode_function_input(trace, func)),
                        return_data,
                    }
                } else {
                    DecodedCallTrace {
                        label,
                        call_data: self.fallback_call_data(trace),
                        return_data,
                    }
                };
            }

            let contract_functions = self.select_contract_function(functions, trace);
            let [func, ..] = contract_functions else {
                return DecodedCallTrace {
                    label,
                    call_data: self.fallback_call_data(trace),
                    return_data: self.default_return_data(trace),
                };
            };

            // If traced contract is a fallback contract, check if it has the decoded function.
            // If not, then replace call data signature with `fallback`.
            let mut call_data = self.decode_function_input(trace, func);
            if let Some(fallback_functions) = self.fallback_contracts.get(&trace.address)
                && !fallback_functions.contains(&selector)
                && let Some(cd) = self.fallback_call_data(trace)
            {
                call_data.signature = cd.signature;
            }

            DecodedCallTrace {
                label,
                call_data: Some(call_data),
                return_data: self.decode_function_output(trace, contract_functions),
            }
        } else {
            DecodedCallTrace {
                label,
                call_data: self.fallback_call_data(trace),
                return_data: self.default_return_data(trace),
            }
        }
    }

    /// Decodes a function's input into the given trace.
    fn decode_function_input(&self, trace: &CallTrace, func: &Function) -> DecodedCallData {
        let mut args = None;
        if trace.data.len() >= SELECTOR_LEN {
            if trace.address == CHEATCODE_ADDRESS {
                // Try to decode cheatcode inputs in a more custom way
                if let Some(v) = self.decode_cheatcode_inputs(func, &trace.data) {
                    args = Some(v);
                }
            }

            if args.is_none()
                && let Ok(decoded) = func.abi_decode_input(&trace.data[SELECTOR_LEN..])
            {
                args = Some(
                    decoded
                        .iter()
                        .zip(&func.inputs)
                        .map(|(value, input)| {
                            self.format_param_value(
                                Some(trace.address),
                                &func.name,
                                &input.name,
                                &input.ty,
                                value,
                            )
                        })
                        .collect(),
                );
            }
        }

        DecodedCallData { signature: func.signature(), args: args.unwrap_or_default() }
    }

    /// Custom decoding for cheatcode inputs.
    fn decode_cheatcode_inputs(&self, func: &Function, data: &[u8]) -> Option<Vec<String>> {
        match func.name.as_str() {
            "expectRevert" => {
                let decoded = match data.get(SELECTOR_LEN..) {
                    Some(data) => func.abi_decode_input(data).ok(),
                    None => None,
                };
                let Some(decoded) = decoded else {
                    return Some(vec![self.revert_decoder.decode(data, None)]);
                };
                let Some(first) = decoded.first() else {
                    return Some(vec![self.revert_decoder.decode(data, None)]);
                };
                let expected_revert = match first {
                    DynSolValue::Bytes(bytes) => bytes.as_slice(),
                    DynSolValue::FixedBytes(word, size) => &word[..*size],
                    _ => return None,
                };
                Some(
                    std::iter::once(self.revert_decoder.decode(expected_revert, None))
                        .chain(decoded.iter().skip(1).map(|value| self.format_value(value)))
                        .collect(),
                )
            }
            "addr" | "createWallet" | "deriveKey" | "rememberKey" => {
                // Redact private key in all cases
                Some(vec!["<pk>".to_string()])
            }
            "broadcast" | "startBroadcast" => {
                // Redact private key if defined
                // broadcast(uint256) / startBroadcast(uint256)
                (!func.inputs.is_empty() && func.inputs[0].ty == "uint256").then(|| vec!["<pk>".to_string()])
            }
            "getNonce" => {
                // Redact private key if defined
                // getNonce(Wallet)
                (!func.inputs.is_empty() && func.inputs[0].ty == "tuple").then(|| vec!["<pk>".to_string()])
            }
            "sign" | "signP256" => {
                let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..]).ok()?;

                // Redact private key and replace in trace
                // sign(uint256,bytes32) / signP256(uint256,bytes32) / sign(Wallet,bytes32)
                if !decoded.is_empty() &&
                    (func.inputs[0].ty == "uint256" || func.inputs[0].ty == "tuple")
                {
                    decoded[0] = DynSolValue::String("<pk>".to_string());
                }

                Some(decoded.iter().map(format_token).collect())
            }
            "signDelegation" | "signAndAttachDelegation" => {
                let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..]).ok()?;
                // Redact private key and replace in trace for
                // signAndAttachDelegation(address implementation, uint256 privateKey)
                // signDelegation(address implementation, uint256 privateKey)
                decoded[1] = DynSolValue::String("<pk>".to_string());
                Some(decoded.iter().map(format_token).collect())
            }
            "parseJson" |
            "parseJsonUint" |
            "parseJsonUintArray" |
            "parseJsonInt" |
            "parseJsonIntArray" |
            "parseJsonString" |
            "parseJsonStringArray" |
            "parseJsonAddress" |
            "parseJsonAddressArray" |
            "parseJsonBool" |
            "parseJsonBoolArray" |
            "parseJsonBytes" |
            "parseJsonBytesArray" |
            "parseJsonBytes32" |
            "parseJsonBytes32Array" |
            "writeJson" |
            // `keyExists` is being deprecated in favor of `keyExistsJson`. It will be removed in future versions.
            "keyExists" |
            "keyExistsJson" |
            "serializeBool" |
            "serializeUint" |
            "serializeUintToHex" |
            "serializeInt" |
            "serializeAddress" |
            "serializeBytes32" |
            "serializeString" |
            "serializeBytes" => {
                if self.verbosity >= 5 {
                    None
                } else {
                    let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..]).ok()?;
                    let token = if func.name.as_str() == "parseJson" ||
                        // `keyExists` is being deprecated in favor of `keyExistsJson`. It will be removed in future versions.
                        func.name.as_str() == "keyExists" ||
                        func.name.as_str() == "keyExistsJson"
                    {
                        "<JSON file>"
                    } else {
                        "<stringified JSON>"
                    };
                    decoded[0] = DynSolValue::String(token.to_string());
                    Some(decoded.iter().map(format_token).collect())
                }
            }
            s if s.contains("Toml") => {
                if self.verbosity >= 5 {
                    None
                } else {
                    let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..]).ok()?;
                    let token = if func.name.as_str() == "parseToml" ||
                        func.name.as_str() == "keyExistsToml"
                    {
                        "<TOML file>"
                    } else {
                        "<stringified TOML>"
                    };
                    decoded[0] = DynSolValue::String(token.to_string());
                    Some(decoded.iter().map(format_token).collect())
                }
            }
            "createFork" |
            "createSelectFork" |
            "rpc" => {
                let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..]).ok()?;

                // Redact RPC URL except if referenced by an alias
                if !decoded.is_empty() && func.inputs[0].ty == "string" {
                    let url_or_alias = decoded[0].as_str().unwrap_or_default();

                    if url_or_alias.starts_with("http") || url_or_alias.starts_with("ws") {
                        decoded[0] = DynSolValue::String("<rpc url>".to_string());
                    }
                } else {
                    return None;
                }

                Some(decoded.iter().map(format_token).collect())
            }
            _ => None,
        }
    }

    /// Decodes a function's output into the given trace.
    fn decode_function_output(&self, trace: &CallTrace, funcs: &[Function]) -> Option<String> {
        if !trace.success {
            return self.default_return_data(trace);
        }

        if trace.address == CHEATCODE_ADDRESS
            && let Some(decoded) = funcs.iter().find_map(|func| self.decode_cheatcode_outputs(func))
        {
            return Some(decoded);
        }

        if let Some(values) =
            funcs.iter().find_map(|func| func.abi_decode_output(&trace.output).ok())
        {
            // Functions coming from an external database do not have any outputs specified,
            // and will lead to returning an empty list of values.
            if values.is_empty() {
                return None;
            }

            return Some(
                values.iter().map(|value| self.format_value(value)).format(", ").to_string(),
            );
        }

        None
    }

    /// Custom decoding for cheatcode outputs.
    fn decode_cheatcode_outputs(&self, func: &Function) -> Option<String> {
        match func.name.as_str() {
            s if s.starts_with("env") => Some("<env var value>"),
            "createWallet" | "deriveKey" => Some("<pk>"),
            "promptSecret" | "promptSecretUint" => Some("<secret>"),
            "parseJson" if self.verbosity < 5 => Some("<encoded JSON value>"),
            "readFile" if self.verbosity < 5 => Some("<file>"),
            "rpcUrl" | "rpcUrls" | "rpcUrlStructs" => Some("<rpc url>"),
            _ => None,
        }
        .map(Into::into)
    }

    #[track_caller]
    fn fallback_call_data(&self, trace: &CallTrace) -> Option<DecodedCallData> {
        let cdata = &trace.data;
        let signature = if cdata.is_empty() && self.receive_contracts.contains(&trace.address) {
            "receive()"
        } else if self.fallback_contracts.contains_key(&trace.address) {
            "fallback()"
        } else {
            return None;
        }
        .to_string();
        let args = if cdata.is_empty() { Vec::new() } else { vec![cdata.to_string()] };
        Some(DecodedCallData { signature, args })
    }

    /// The default decoded return data for a trace.
    fn default_return_data(&self, trace: &CallTrace) -> Option<String> {
        // For calls with status None or successful status, don't decode revert data
        // This is due to trace.status is derived from the revm_interpreter::InstructionResult in
        // revm-inspectors status will `None` post revm 27, as `InstructionResult::Continue` does
        // not exists anymore.
        if trace.status.is_none_or(|s| s.is_ok()) {
            return None;
        }
        (!trace.success).then(|| self.revert_decoder.decode(&trace.output, trace.status))
    }

    /// Decodes an event.
    pub async fn decode_event(&self, log: &LogData) -> DecodedCallLog {
        self.decode_event_inner(None, log).await
    }

    /// Decodes an event emitted by a known address.
    pub async fn decode_event_with_address(
        &self,
        address: Address,
        log: &LogData,
    ) -> DecodedCallLog {
        self.decode_event_inner(Some(address), log).await
    }

    async fn decode_event_inner(&self, address: Option<Address>, log: &LogData) -> DecodedCallLog {
        let &[t0, ..] = log.topics() else { return DecodedCallLog { name: None, params: None } };

        let mut events = Vec::new();
        let events = match self.events.get(&(t0, log.topics().len() - 1)) {
            Some(es) => es,
            None => {
                if let Some(identifier) = &self.signature_identifier
                    && let Some(event) = identifier.identify_event(t0).await
                {
                    events.push(get_indexed_event(event, log));
                }
                &events
            }
        };
        for event in events {
            if let Ok(decoded) = event.decode_log(log) {
                let params = reconstruct_params(event, &decoded);
                return DecodedCallLog {
                    name: Some(event.name.clone()),
                    params: Some(
                        params
                            .into_iter()
                            .zip(event.inputs.iter())
                            .map(|(param, input)| {
                                // undo patched names
                                let name = input.name.clone();
                                (
                                    name,
                                    self.format_param_value(
                                        address,
                                        &event.name,
                                        &input.name,
                                        &input.ty,
                                        &param,
                                    ),
                                )
                            })
                            .collect(),
                    ),
                };
            }
        }

        DecodedCallLog { name: None, params: None }
    }

    /// Prefetches function and event signatures into the identifier cache
    pub async fn prefetch_signatures(&self, nodes: &[CallTraceNode]) {
        let Some(identifier) = &self.signature_identifier else { return };
        let events = nodes
            .iter()
            .flat_map(|node| {
                node.logs
                    .iter()
                    .map(|log| log.raw_log.topics())
                    .filter(|&topics| {
                        if let Some(&first) = topics.first()
                            && self.events.contains_key(&(first, topics.len() - 1))
                        {
                            return false;
                        }
                        true
                    })
                    .filter_map(|topics| topics.first())
            })
            .copied();
        let functions = nodes
            .iter()
            .filter(|&n| {
                // Ignore known addresses.
                if n.trace.address == DEFAULT_CREATE2_DEPLOYER
                    || n.is_precompile()
                    || precompiles::is_known_precompile(
                        n.trace.address,
                        self.chain_id,
                        self.tempo_hardfork,
                    )
                {
                    return false;
                }
                // Ignore non-ABI calldata.
                if n.trace.kind.is_any_create() || !is_abi_call_data(&n.trace.data) {
                    return false;
                }
                true
            })
            .filter_map(|n| n.trace.data.first_chunk().map(Selector::from))
            .filter(|selector| !self.functions.contains_key(selector));
        let selectors = events
            .map(SelectorKind::Event)
            .chain(functions.map(SelectorKind::Function))
            .unique()
            .collect::<Vec<_>>();
        let _ = identifier.identify(&selectors).await;
    }

    /// Pretty-prints a value.
    fn format_value(&self, value: &DynSolValue) -> String {
        if let DynSolValue::Address(addr) = value
            && let Some(label) = self.labels.get(addr)
        {
            return format!("{label}: [{addr}]");
        }
        format_token(value)
    }

    fn format_param_value(
        &self,
        address: Option<Address>,
        context_name: &str,
        input_name: &str,
        input_ty: &str,
        value: &DynSolValue,
    ) -> String {
        self.format_claim_receipt_bytes(address, context_name, input_name, input_ty, value)
            .map(|value| self.format_value(&value))
            .unwrap_or_else(|| self.format_value(value))
    }

    fn format_claim_receipt_bytes(
        &self,
        address: Option<Address>,
        context_name: &str,
        input_name: &str,
        input_ty: &str,
        value: &DynSolValue,
    ) -> Option<DynSolValue> {
        if !matches!(address, Some(RECEIVE_POLICY_GUARD_ADDRESS | TIP403_REGISTRY_ADDRESS)) {
            return None;
        }
        if input_name != "receipt" || input_ty != "bytes" {
            return None;
        }
        if !matches!(context_name, "balanceOf" | "claim" | "burnBlockedReceipt" | "TransferBlocked")
        {
            return None;
        }
        let DynSolValue::Bytes(bytes) = value else { return None };
        let decoded = IReceivePolicyGuard::ClaimReceiptV1::abi_decode(bytes).ok()?;

        Some(DynSolValue::CustomStruct {
            name: "ClaimReceiptV1".to_string(),
            prop_names: vec![
                "version".to_string(),
                "token".to_string(),
                "recoveryAuthority".to_string(),
                "originator".to_string(),
                "recipient".to_string(),
                "blockedAt".to_string(),
                "blockedNonce".to_string(),
                "blockedReason".to_string(),
                "kind".to_string(),
                "memo".to_string(),
            ],
            tuple: vec![
                DynSolValue::Uint(U256::from(decoded.version), 8),
                DynSolValue::Address(decoded.token),
                DynSolValue::Address(decoded.recoveryAuthority),
                DynSolValue::Address(decoded.originator),
                DynSolValue::Address(decoded.recipient),
                DynSolValue::Uint(U256::from(decoded.blockedAt), 64),
                DynSolValue::Uint(U256::from(decoded.blockedNonce), 64),
                DynSolValue::Uint(U256::from(decoded.blockedReason), 8),
                DynSolValue::Uint(U256::from(decoded.kind as u8), 8),
                DynSolValue::FixedBytes(decoded.memo, 32),
            ],
        })
    }
}

/// Returns `true` if the given function calldata (including function selector) is ABI-encoded.
///
/// This is a simple heuristic to avoid fetching non ABI-encoded selectors.
fn is_abi_call_data(data: &[u8]) -> bool {
    match data.len().cmp(&SELECTOR_LEN) {
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Greater => is_abi_data(&data[SELECTOR_LEN..]),
    }
}

/// Returns `true` if the given data is ABI-encoded.
///
/// See [`is_abi_call_data`] for more details.
fn is_abi_data(data: &[u8]) -> bool {
    let rem = data.len() % 32;
    if rem == 0 || data.is_empty() {
        return true;
    }
    // If the length is not a multiple of 32, also accept when the last remainder bytes are all 0.
    data[data.len() - rem..].iter().all(|byte| *byte == 0)
}

/// Restore the order of the params of a decoded event,
/// as Alloy returns the indexed and unindexed params separately.
fn reconstruct_params(event: &Event, decoded: &DecodedEvent) -> Vec<DynSolValue> {
    let mut indexed = 0;
    let mut unindexed = 0;
    let mut inputs = vec![];
    for input in &event.inputs {
        // Prevent panic of event `Transfer(from, to)` decoded with a signature
        // `Transfer(address indexed from, address indexed to, uint256 indexed tokenId)` by making
        // sure the event inputs is not higher than decoded indexed / un-indexed values.
        if input.indexed && indexed < decoded.indexed.len() {
            inputs.push(decoded.indexed[indexed].clone());
            indexed += 1;
        } else if unindexed < decoded.body.len() {
            inputs.push(decoded.body[unindexed].clone());
            unindexed += 1;
        }
    }

    inputs
}

fn indexed_inputs(event: &Event) -> usize {
    event.inputs.iter().filter(|param| param.indexed).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, aliases::U96, hex};
    use alloy_sol_types::{SolCall, SolEvent};

    #[test]
    fn test_selector_collision_resolution() {
        use alloy_json_abi::Function;
        use alloy_primitives::Address;

        // Create two functions with the same selector but different signatures
        let func1 = Function::parse("transferFrom(address,address,uint256)").unwrap();
        let func2 = Function::parse("gasprice_bit_ether(int128)").unwrap();

        // Verify they have the same selector (this is the collision)
        assert_eq!(func1.selector(), func2.selector());

        let functions = vec![func1, func2];

        // Create a mock trace with calldata that matches func1
        let trace = CallTrace {
            address: Address::from([0x12; 20]),
            data: hex!("23b872dd000000000000000000000000000000000000000000000000000000000000012300000000000000000000000000000000000000000000000000000000000004560000000000000000000000000000000000000000000000000000000000000064").to_vec().into(),
            ..Default::default()
        };

        let decoder = CallTraceDecoder::new();
        let result = decoder.select_contract_function(&functions, &trace);

        // Should return only the function that can decode the calldata (func1)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].signature(), "transferFrom(address,address,uint256)");
    }

    #[test]
    fn test_selector_collision_resolution_second_function() {
        use alloy_json_abi::Function;
        use alloy_primitives::Address;

        // Create two functions with the same selector but different signatures
        let func1 = Function::parse("transferFrom(address,address,uint256)").unwrap();
        let func2 = Function::parse("gasprice_bit_ether(int128)").unwrap();

        let functions = vec![func1, func2];

        // Create a mock trace with calldata that matches func2
        let trace = CallTrace {
            address: Address::from([0x12; 20]),
            data: hex!("23b872dd0000000000000000000000000000000000000000000000000000000000000064")
                .to_vec()
                .into(),
            ..Default::default()
        };

        let decoder = CallTraceDecoder::new();
        let result = decoder.select_contract_function(&functions, &trace);

        // Should return only the function that can decode the calldata (func2)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].signature(), "gasprice_bit_ether(int128)");
    }

    #[test]
    fn test_should_redact() {
        let decoder = CallTraceDecoder::new();

        let expected_revert_bytes4 = vec![0xde, 0xad, 0xbe, 0xef];
        let expect_revert_bytes4_data = Function::parse("expectRevert(bytes4)")
            .unwrap()
            .abi_encode_input(&[DynSolValue::FixedBytes(
                B256::right_padding_from(expected_revert_bytes4.as_slice()),
                4,
            )])
            .unwrap();

        let expected_revert_bytes = hex!(
            "08c379a000000000000000000000000000000000000000000000000000000000\
             0000002000000000000000000000000000000000000000000000000000000000\
             00000004626f6f6d000000000000000000000000000000000000000000000000"
        )
        .to_vec();
        let expect_revert_bytes_data = Function::parse("expectRevert(bytes)")
            .unwrap()
            .abi_encode_input(&[DynSolValue::Bytes(expected_revert_bytes.clone())])
            .unwrap();

        let reverter = Address::from([0x11; 20]);
        let expect_revert_bytes4_address_data = Function::parse("expectRevert(bytes4,address)")
            .unwrap()
            .abi_encode_input(&[
                DynSolValue::FixedBytes(
                    B256::right_padding_from(expected_revert_bytes4.as_slice()),
                    4,
                ),
                DynSolValue::Address(reverter),
            ])
            .unwrap();

        let count = 42_u64;
        let expect_revert_bytes_count_data = Function::parse("expectRevert(bytes,uint64)")
            .unwrap()
            .abi_encode_input(&[
                DynSolValue::Bytes(expected_revert_bytes.clone()),
                DynSolValue::Uint(alloy_primitives::U256::from(count), 64),
            ])
            .unwrap();

        let expect_revert_bytes_address_count_data =
            Function::parse("expectRevert(bytes,address,uint64)")
                .unwrap()
                .abi_encode_input(&[
                    DynSolValue::Bytes(expected_revert_bytes.clone()),
                    DynSolValue::Address(reverter),
                    DynSolValue::Uint(alloy_primitives::U256::from(count), 64),
                ])
                .unwrap();

        let expect_revert_runtime_data = expected_revert_bytes4.clone();

        // [function_signature, data, expected]
        let cheatcode_input_test_cases = vec![
            // Should decode the expected revert payload, not full cheatcode calldata:
            (
                "expectRevert(bytes4)",
                expect_revert_bytes4_data,
                Some(vec![decoder.revert_decoder.decode(expected_revert_bytes4.as_slice(), None)]),
            ),
            (
                "expectRevert(bytes)",
                expect_revert_bytes_data,
                Some(vec![decoder.revert_decoder.decode(expected_revert_bytes.as_slice(), None)]),
            ),
            (
                "expectRevert(bytes4)",
                expect_revert_runtime_data.clone(),
                Some(vec![
                    decoder.revert_decoder.decode(expect_revert_runtime_data.as_slice(), None),
                ]),
            ),
            (
                "expectRevert(bytes4,address)",
                expect_revert_bytes4_address_data,
                Some(vec![
                    decoder.revert_decoder.decode(expected_revert_bytes4.as_slice(), None),
                    decoder.format_value(&DynSolValue::Address(reverter)),
                ]),
            ),
            (
                "expectRevert(bytes,uint64)",
                expect_revert_bytes_count_data,
                Some(vec![
                    decoder.revert_decoder.decode(expected_revert_bytes.as_slice(), None),
                    decoder
                        .format_value(&DynSolValue::Uint(alloy_primitives::U256::from(count), 64)),
                ]),
            ),
            (
                "expectRevert(bytes,address,uint64)",
                expect_revert_bytes_address_count_data,
                Some(vec![
                    decoder.revert_decoder.decode(expected_revert_bytes.as_slice(), None),
                    decoder.format_value(&DynSolValue::Address(reverter)),
                    decoder
                        .format_value(&DynSolValue::Uint(alloy_primitives::U256::from(count), 64)),
                ]),
            ),
            (
                "expectRevert()",
                expect_revert_runtime_data.clone(),
                Some(vec![
                    decoder.revert_decoder.decode(expect_revert_runtime_data.as_slice(), None),
                ]),
            ),
            // Should redact private key from traces in all cases:
            ("addr(uint256)", vec![], Some(vec!["<pk>".to_string()])),
            ("createWallet(string)", vec![], Some(vec!["<pk>".to_string()])),
            ("createWallet(uint256)", vec![], Some(vec!["<pk>".to_string()])),
            ("deriveKey(string,uint32)", vec![], Some(vec!["<pk>".to_string()])),
            ("deriveKey(string,string,uint32)", vec![], Some(vec!["<pk>".to_string()])),
            ("deriveKey(string,uint32,string)", vec![], Some(vec!["<pk>".to_string()])),
            ("deriveKey(string,string,uint32,string)", vec![], Some(vec!["<pk>".to_string()])),
            ("rememberKey(uint256)", vec![], Some(vec!["<pk>".to_string()])),
            //
            // Should redact private key from traces in specific cases with exceptions:
            ("broadcast(uint256)", vec![], Some(vec!["<pk>".to_string()])),
            ("broadcast()", vec![], None), // Ignore: `private key` is not passed.
            ("startBroadcast(uint256)", vec![], Some(vec!["<pk>".to_string()])),
            ("startBroadcast()", vec![], None), // Ignore: `private key` is not passed.
            ("getNonce((address,uint256,uint256,uint256))", vec![], Some(vec!["<pk>".to_string()])),
            ("getNonce(address)", vec![], None), // Ignore: `address` is public.
            //
            // Should redact private key and replace in trace in cases:
            (
                "sign(uint256,bytes32)",
                hex!(
                    "
                    e341eaa4
                    7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6
                    0000000000000000000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"<pk>\"".to_string(),
                    "0x0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                ]),
            ),
            (
                "signP256(uint256,bytes32)",
                hex!(
                    "
                    83211b40
                    7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6
                    0000000000000000000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"<pk>\"".to_string(),
                    "0x0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "createFork(string)" "https://eth-mainnet.g.alchemy.com/v2/api_key"
                "createFork(string)",
                hex!(
                    "
                    31ba3498
                    0000000000000000000000000000000000000000000000000000000000000020
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                    "
                )
                .to_vec(),
                Some(vec!["\"<rpc url>\"".to_string()]),
            ),
            (
                // cast calldata "createFork(string)" "wss://eth-mainnet.g.alchemy.com/v2/api_key"
                "createFork(string)",
                hex!(
                    "
                    31ba3498
                    0000000000000000000000000000000000000000000000000000000000000020
                    000000000000000000000000000000000000000000000000000000000000002a
                    7773733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f6d2f
                    76322f6170695f6b657900000000000000000000000000000000000000000000
                    "
                )
                .to_vec(),
                Some(vec!["\"<rpc url>\"".to_string()]),
            ),
            (
                // cast calldata "createFork(string)" "mainnet"
                "createFork(string)",
                hex!(
                    "
                    31ba3498
                    0000000000000000000000000000000000000000000000000000000000000020
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                    "
                )
                .to_vec(),
                Some(vec!["\"mainnet\"".to_string()]),
            ),
            (
                // cast calldata "createFork(string,uint256)" "https://eth-mainnet.g.alchemy.com/v2/api_key" 1
                "createFork(string,uint256)",
                hex!(
                    "
                    6ba3ba2b
                    0000000000000000000000000000000000000000000000000000000000000040
                    0000000000000000000000000000000000000000000000000000000000000001
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec!["\"<rpc url>\"".to_string(), "1".to_string()]),
            ),
            (
                // cast calldata "createFork(string,uint256)" "mainnet" 1
                "createFork(string,uint256)",
                hex!(
                    "
                    6ba3ba2b
                    0000000000000000000000000000000000000000000000000000000000000040
                    0000000000000000000000000000000000000000000000000000000000000001
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec!["\"mainnet\"".to_string(), "1".to_string()]),
            ),
            (
                // cast calldata "createFork(string,bytes32)" "https://eth-mainnet.g.alchemy.com/v2/api_key" 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                "createFork(string,bytes32)",
                hex!(
                    "
                    7ca29682
                    0000000000000000000000000000000000000000000000000000000000000040
                    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"<rpc url>\"".to_string(),
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "createFork(string,bytes32)" "mainnet"
                // 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                "createFork(string,bytes32)",
                hex!(
                    "
                    7ca29682
                    0000000000000000000000000000000000000000000000000000000000000040
                    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"mainnet\"".to_string(),
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "createSelectFork(string)" "https://eth-mainnet.g.alchemy.com/v2/api_key"
                "createSelectFork(string)",
                hex!(
                    "
                    98680034
                    0000000000000000000000000000000000000000000000000000000000000020
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                    "
                )
                .to_vec(),
                Some(vec!["\"<rpc url>\"".to_string()]),
            ),
            (
                // cast calldata "createSelectFork(string)" "mainnet"
                "createSelectFork(string)",
                hex!(
                    "
                    98680034
                    0000000000000000000000000000000000000000000000000000000000000020
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                    "
                )
                .to_vec(),
                Some(vec!["\"mainnet\"".to_string()]),
            ),
            (
                // cast calldata "createSelectFork(string,uint256)" "https://eth-mainnet.g.alchemy.com/v2/api_key" 1
                "createSelectFork(string,uint256)",
                hex!(
                    "
                    71ee464d
                    0000000000000000000000000000000000000000000000000000000000000040
                    0000000000000000000000000000000000000000000000000000000000000001
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec!["\"<rpc url>\"".to_string(), "1".to_string()]),
            ),
            (
                // cast calldata "createSelectFork(string,uint256)" "mainnet" 1
                "createSelectFork(string,uint256)",
                hex!(
                    "
                    71ee464d
                    0000000000000000000000000000000000000000000000000000000000000040
                    0000000000000000000000000000000000000000000000000000000000000001
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec!["\"mainnet\"".to_string(), "1".to_string()]),
            ),
            (
                // cast calldata "createSelectFork(string,bytes32)" "https://eth-mainnet.g.alchemy.com/v2/api_key" 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                "createSelectFork(string,bytes32)",
                hex!(
                    "
                    84d52b7a
                    0000000000000000000000000000000000000000000000000000000000000040
                    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"<rpc url>\"".to_string(),
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "createSelectFork(string,bytes32)" "mainnet"
                // 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                "createSelectFork(string,bytes32)",
                hex!(
                    "
                    84d52b7a
                    0000000000000000000000000000000000000000000000000000000000000040
                    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"mainnet\"".to_string(),
                    "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "rpc(string,string,string)" "https://eth-mainnet.g.alchemy.com/v2/api_key" "eth_getBalance" "[\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\",\"0x0\"]"
                "rpc(string,string,string)",
                hex!(
                    "
                    0199a220
                    0000000000000000000000000000000000000000000000000000000000000060
                    00000000000000000000000000000000000000000000000000000000000000c0
                    0000000000000000000000000000000000000000000000000000000000000100
                    000000000000000000000000000000000000000000000000000000000000002c
                    68747470733a2f2f6574682d6d61696e6e65742e672e616c6368656d792e636f
                    6d2f76322f6170695f6b65790000000000000000000000000000000000000000
                    000000000000000000000000000000000000000000000000000000000000000e
                    6574685f67657442616c616e6365000000000000000000000000000000000000
                    0000000000000000000000000000000000000000000000000000000000000034
                    5b22307835353165373738343737386566386530343865343935646634396632
                    363134663834613466316463222c22307830225d000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"<rpc url>\"".to_string(),
                    "\"eth_getBalance\"".to_string(),
                    "\"[\\\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\\\",\\\"0x0\\\"]\""
                        .to_string(),
                ]),
            ),
            (
                // cast calldata "rpc(string,string,string)" "mainnet" "eth_getBalance"
                // "[\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\",\"0x0\"]"
                "rpc(string,string,string)",
                hex!(
                    "
                    0199a220
                    0000000000000000000000000000000000000000000000000000000000000060
                    00000000000000000000000000000000000000000000000000000000000000a0
                    00000000000000000000000000000000000000000000000000000000000000e0
                    0000000000000000000000000000000000000000000000000000000000000007
                    6d61696e6e657400000000000000000000000000000000000000000000000000
                    000000000000000000000000000000000000000000000000000000000000000e
                    6574685f67657442616c616e6365000000000000000000000000000000000000
                    0000000000000000000000000000000000000000000000000000000000000034
                    5b22307835353165373738343737386566386530343865343935646634396632
                    363134663834613466316463222c22307830225d000000000000000000000000
                "
                )
                .to_vec(),
                Some(vec![
                    "\"mainnet\"".to_string(),
                    "\"eth_getBalance\"".to_string(),
                    "\"[\\\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\\\",\\\"0x0\\\"]\""
                        .to_string(),
                ]),
            ),
        ];

        // [function_signature, expected]
        let cheatcode_output_test_cases = vec![
            // Should redact private key on output in all cases:
            ("createWallet(string)", Some("<pk>".to_string())),
            ("deriveKey(string,uint32)", Some("<pk>".to_string())),
            // Should redact RPC URL if defined, except if referenced by an alias:
            ("rpcUrl(string)", Some("<rpc url>".to_string())),
            ("rpcUrls()", Some("<rpc url>".to_string())),
            ("rpcUrlStructs()", Some("<rpc url>".to_string())),
        ];

        for (function_signature, data, expected) in cheatcode_input_test_cases {
            let function = Function::parse(function_signature).unwrap();
            let result = decoder.decode_cheatcode_inputs(&function, &data);
            assert_eq!(result, expected, "Input case failed for: {function_signature}");
        }

        for (function_signature, expected) in cheatcode_output_test_cases {
            let function = Function::parse(function_signature).unwrap();
            let result = Some(decoder.decode_cheatcode_outputs(&function).unwrap_or_default());
            assert_eq!(result, expected, "Output case failed for: {function_signature}");
        }
    }

    #[tokio::test]
    async fn test_tempo_decode_preserves_existing_labels() {
        let decoder = CallTraceDecoder::new();
        let trace = CallTrace { address: PATH_USD_ADDRESS, success: true, ..Default::default() };

        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("PathUSD"));
    }

    #[tokio::test]
    async fn test_t5_decode_does_not_synthesize_general_target_label() {
        let mut decoder = CallTraceDecoder::new().clone();
        decoder.chain_id = Some(4217);
        let trace = CallTrace {
            address: address!("0x0000000000000000000000000000000000000123"),
            depth: 0,
            success: true,
            ..Default::default()
        };

        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label, None);
    }

    #[tokio::test]
    async fn test_t5_tip20_logo_uri_calls_and_events_decode() {
        let decoder = CallTraceDecoder::new();

        let call = ITIP20::setLogoURICall { newLogoURI: "https://example.com/logo.png".into() };
        let trace = CallTrace {
            address: PATH_USD_ADDRESS,
            data: call.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        let call_data = decoded.call_data.expect("setLogoURI should decode");
        assert_eq!(call_data.signature, "setLogoURI(string)");
        assert_eq!(call_data.args, vec!["\"https://example.com/logo.png\"".to_string()]);

        let call = ITIP20::logoURICall {};
        let trace = CallTrace {
            address: PATH_USD_ADDRESS,
            data: call.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.call_data.expect("logoURI should decode").signature, "logoURI()");

        let event = ITIP20::LogoURIUpdated {
            updater: address!("0x0000000000000000000000000000000000000abc"),
            newLogoURI: "ipfs://logo".into(),
        };
        let decoded = decoder.decode_event(&event.encode_log_data()).await;
        assert_eq!(decoded.name.as_deref(), Some("LogoURIUpdated"));
        let params = decoded.params.expect("LogoURIUpdated params should decode");
        assert_eq!(params[0].0, "updater");
        assert!(
            params[0].1.to_ascii_lowercase().contains("0000000000000000000000000000000000000abc")
        );
        assert_eq!(params[1], ("newLogoURI".into(), "\"ipfs://logo\"".into()));
    }

    #[tokio::test]
    async fn test_t5_tip20_factory_create_token_with_logo_decodes() {
        let decoder = CallTraceDecoder::new();
        let call = ITIP20Factory::createToken_1Call {
            name: "Example USD".into(),
            symbol: "xUSD".into(),
            currency: "USD".into(),
            quoteToken: PATH_USD_ADDRESS,
            admin: address!("0x0000000000000000000000000000000000000abc"),
            salt: B256::repeat_byte(0x11),
            logoURI: "https://example.com/xusd.png".into(),
        };
        let trace = CallTrace {
            address: TIP20_FACTORY_ADDRESS,
            data: call.abi_encode().into(),
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        let call_data = decoded.call_data.expect("createToken overload should decode");
        assert_eq!(
            call_data.signature,
            "createToken(string,string,string,address,address,bytes32,string)"
        );
        assert_eq!(call_data.args[6], "\"https://example.com/xusd.png\"");
    }

    #[tokio::test]
    async fn test_t5_stablecoin_dex_order_flipped_event_decodes() {
        let decoder = CallTraceDecoder::new();
        let event = IStablecoinDEX::OrderFlipped {
            orderId: 42,
            maker: address!("0x0000000000000000000000000000000000000abc"),
            token: PATH_USD_ADDRESS,
            amount: 1_000_000,
            isBid: false,
            tick: 100,
            flipTick: 100,
        };
        let decoded = decoder.decode_event(&event.encode_log_data()).await;
        assert_eq!(decoded.name.as_deref(), Some("OrderFlipped"));
        let params = decoded.params.expect("OrderFlipped params should decode");
        assert_eq!(params[0], ("orderId".into(), "42".into()));
        assert_eq!(params[4], ("isBid".into(), "false".into()));
        assert_eq!(params[5], ("tick".into(), "100".into()));
        assert_eq!(params[6], ("flipTick".into(), "100".into()));
    }

    #[tokio::test]
    async fn test_t5_channel_reserve_call_and_event_decode() {
        let mut decoder = CallTraceDecoder::new().clone();
        decoder.chain_id = Some(4217);

        let open = ITIP20ChannelReserve::openCall {
            payee: address!("0x0000000000000000000000000000000000000abc"),
            operator: Address::ZERO,
            token: PATH_USD_ADDRESS,
            deposit: U96::from(1_000_000u64),
            salt: B256::repeat_byte(0x22),
            authorizedSigner: Address::ZERO,
        };
        let trace = CallTrace {
            address: TIP20_CHANNEL_RESERVE_ADDRESS,
            data: open.abi_encode().into(),
            depth: 0,
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("TIP20ChannelReserve"));
        assert_eq!(
            decoded.call_data.expect("open should decode").signature,
            "open(address,address,address,uint96,bytes32,address)"
        );

        let transfer = ITIP20::transferCall {
            to: address!("0x0000000000000000000000000000000000000def"),
            amount: U256::from(1_000_000u64),
        };
        let trace = CallTrace {
            address: PATH_USD_ADDRESS,
            data: transfer.abi_encode().into(),
            depth: 0,
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("PathUSD"));
        let json = serde_json::to_string(&decoded).expect("decoded trace serializes");
        assert!(json.contains(r#""label":"PathUSD""#));
        assert!(!json.contains("payment-lane"));

        let balance_of = ITIP20::balanceOfCall {
            account: address!("0x0000000000000000000000000000000000000def"),
        };
        let trace = CallTrace {
            address: PATH_USD_ADDRESS,
            data: balance_of.abi_encode().into(),
            depth: 0,
            success: true,
            ..Default::default()
        };
        let decoded = decoder.decode_function(&trace).await;
        assert_eq!(decoded.label.as_deref(), Some("PathUSD"));

        let event = ITIP20ChannelReserve::ChannelOpened {
            channelId: B256::repeat_byte(0x33),
            payer: address!("0x0000000000000000000000000000000000000123"),
            payee: address!("0x0000000000000000000000000000000000000abc"),
            operator: Address::ZERO,
            token: PATH_USD_ADDRESS,
            authorizedSigner: Address::ZERO,
            salt: B256::repeat_byte(0x22),
            expiringNonceHash: B256::repeat_byte(0x44),
            deposit: U96::from(1_000_000u64),
        };
        let decoded = decoder.decode_event(&event.encode_log_data()).await;
        assert_eq!(decoded.name.as_deref(), Some("ChannelOpened"));
        let params = decoded.params.expect("ChannelOpened params should decode");
        assert_eq!(params[0].0, "channelId");
        assert_eq!(params[8].0, "deposit");
        assert!(params[8].1.starts_with("1000000"));
    }

    // A mock identifier that records which addresses it was asked to identify.
    struct RecordingIdentifier {
        queried: Vec<Address>,
    }
    impl TraceIdentifier for RecordingIdentifier {
        fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
            self.queried.extend(nodes.iter().map(|n| n.trace.address));
            Vec::new()
        }
    }

    #[test]
    fn test_identify_addresses_skips_evm_precompiles() {
        use foundry_evm_core::precompiles::SHA_256;

        let decoder = CallTraceDecoder::new();

        let mut arena = CallTraceArena::default();
        let regular_addr = Address::from([0x42; 20]);
        arena.nodes_mut()[0].trace.address = regular_addr;

        // Standard EVM precompile flagged by the inspector.
        arena.nodes_mut().push(CallTraceNode {
            trace: CallTrace {
                address: SHA_256,
                depth: 1,
                maybe_precompile: Some(true),
                ..Default::default()
            },
            idx: 1,
            ..Default::default()
        });

        // Standard EVM precompile NOT flagged, caught by is_known_precompile.
        arena.nodes_mut().push(CallTraceNode {
            trace: CallTrace {
                address: SHA_256,
                depth: 1,
                maybe_precompile: None,
                ..Default::default()
            },
            idx: 2,
            ..Default::default()
        });

        let mut identifier = RecordingIdentifier { queried: Vec::new() };
        decoder.identify_addresses(&arena, &mut identifier);

        assert_eq!(identifier.queried, vec![regular_addr]);
    }

    #[test]
    fn test_identify_addresses_skips_tempo_precompiles() {
        use foundry_evm_core::tempo::{TEMPO_PRECOMPILE_ADDRESSES, TIP20_CHANNEL_RESERVE_ADDRESS};

        // Decoder with Tempo chain ID (4217).
        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(4217))
            .with_tempo_hardfork(Some(TempoHardfork::T5))
            .build();

        assert_eq!(
            decoder.labels.get(&TIP20_CHANNEL_RESERVE_ADDRESS),
            Some(&"TIP20ChannelReserve".to_string())
        );

        let mut arena = CallTraceArena::default();
        let regular_addr = Address::from([0x42; 20]);
        arena.nodes_mut()[0].trace.address = regular_addr;

        // Tempo precompile — not flagged by inspector, caught by is_known_precompile
        // only when chain_id is a Tempo chain.
        let tempo_precompile = TEMPO_PRECOMPILE_ADDRESSES[0];
        arena.nodes_mut().push(CallTraceNode {
            trace: CallTrace {
                address: tempo_precompile,
                depth: 1,
                maybe_precompile: None,
                ..Default::default()
            },
            idx: 1,
            ..Default::default()
        });

        let mut identifier = RecordingIdentifier { queried: Vec::new() };
        decoder.identify_addresses(&arena, &mut identifier);

        // On a Tempo chain, the Tempo precompile should be filtered out.
        assert_eq!(identifier.queried, vec![regular_addr]);
    }

    #[test]
    fn test_precompile_labels_include_local_tempo_precompiles() {
        let decoder =
            CallTraceDecoderBuilder::new().with_tempo_hardfork(Some(TempoHardfork::T5)).build();

        let labels = decoder.precompile_labels();
        assert_eq!(labels.get(&TIP_FEE_MANAGER_ADDRESS), Some(&"FeeManager".to_string()));
        assert_eq!(
            labels.get(&TIP20_CHANNEL_RESERVE_ADDRESS),
            Some(&"TIP20ChannelReserve".to_string())
        );
        assert!(!labels.contains_key(&RECEIVE_POLICY_GUARD_ADDRESS));
    }

    #[test]
    fn test_precompile_labels_skip_tempo_precompiles_on_other_chains() {
        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(1))
            .with_tempo_hardfork(Some(TempoHardfork::T6))
            .build();

        let labels = decoder.precompile_labels();
        assert!(!labels.contains_key(&TIP_FEE_MANAGER_ADDRESS));
        assert!(!labels.contains_key(&RECEIVE_POLICY_GUARD_ADDRESS));
    }

    #[test]
    fn test_tempo_hardfork_labels_do_not_clobber_user_labels() {
        use foundry_evm_core::tempo::TIP20_CHANNEL_RESERVE_ADDRESS;

        let reserve_label = "UserReserve".to_string();
        let guard_label = "UserGuard".to_string();
        let decoder = CallTraceDecoderBuilder::new()
            .with_labels([
                (TIP20_CHANNEL_RESERVE_ADDRESS, reserve_label.clone()),
                (RECEIVE_POLICY_GUARD_ADDRESS, guard_label.clone()),
            ])
            .with_tempo_hardfork(Some(TempoHardfork::T6))
            .build();

        assert_eq!(decoder.labels.get(&TIP20_CHANNEL_RESERVE_ADDRESS), Some(&reserve_label));
        assert_eq!(decoder.labels.get(&RECEIVE_POLICY_GUARD_ADDRESS), Some(&guard_label));
    }

    #[test]
    fn test_tempo_hardfork_none_does_not_remove_user_reserve_label() {
        use foundry_evm_core::tempo::TIP20_CHANNEL_RESERVE_ADDRESS;

        let reserve_label = "UserReserve".to_string();
        let decoder = CallTraceDecoderBuilder::new()
            .with_labels([(TIP20_CHANNEL_RESERVE_ADDRESS, reserve_label.clone())])
            .with_tempo_hardfork(None)
            .build();

        assert_eq!(decoder.labels.get(&TIP20_CHANNEL_RESERVE_ADDRESS), Some(&reserve_label));
    }

    #[tokio::test]
    async fn test_decode_receive_policy_guard_at_t6() {
        let function = Function::parse("claim(address,bytes)").unwrap();
        let data = function
            .abi_encode_input(&[
                DynSolValue::Address(Address::from([0x11; 20])),
                DynSolValue::Bytes(vec![0x12, 0x34]),
            ])
            .unwrap();
        let trace = CallTrace {
            address: RECEIVE_POLICY_GUARD_ADDRESS,
            data: data.into(),
            success: true,
            ..Default::default()
        };

        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(4217))
            .with_tempo_hardfork(Some(TempoHardfork::T6))
            .build();
        let decoded = decoder.decode_function(&trace).await;

        assert_eq!(decoded.label, Some("ReceivePolicyGuard".to_string()));
        assert_eq!(decoded.call_data.unwrap().signature, "claim(address,bytes)");
    }

    #[tokio::test]
    async fn test_t6_receive_policy_calls_decode() {
        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(4217))
            .with_tempo_hardfork(Some(TempoHardfork::T6))
            .build();

        let set_policy = ITIP403Registry::setReceivePolicyCall {
            senderPolicyId: 7,
            tokenFilterId: 9,
            recoveryAuthority: address!("0x0000000000000000000000000000000000000abc"),
        };
        let decoded = decoder
            .decode_function(&CallTrace {
                address: TIP403_REGISTRY_ADDRESS,
                data: set_policy.abi_encode().into(),
                success: true,
                ..Default::default()
            })
            .await;
        let call_data = decoded.call_data.expect("setReceivePolicy should decode");
        assert_eq!(decoded.label.as_deref(), Some("TIP403Registry"));
        assert_eq!(call_data.signature, "setReceivePolicy(uint64,uint64,address)");
        assert_eq!(call_data.args[0], "7");
        assert_eq!(call_data.args[1], "9");

        let validate = ITIP403Registry::validateReceivePolicyCall {
            token: PATH_USD_ADDRESS,
            sender: address!("0x0000000000000000000000000000000000000def"),
            receiver: address!("0x0000000000000000000000000000000000000123"),
        };
        let decoded = decoder
            .decode_function(&CallTrace {
                address: TIP403_REGISTRY_ADDRESS,
                data: validate.abi_encode().into(),
                success: true,
                ..Default::default()
            })
            .await;
        let call_data = decoded.call_data.expect("validateReceivePolicy should decode");
        assert_eq!(call_data.signature, "validateReceivePolicy(address,address,address)");
        assert!(call_data.args[0].contains("PathUSD"));
    }

    #[tokio::test]
    async fn test_t6_admin_key_calls_decode() {
        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(4217))
            .with_tempo_hardfork(Some(TempoHardfork::T6))
            .build();
        let account = address!("0x0000000000000000000000000000000000000abc");
        let key = address!("0x0000000000000000000000000000000000000def");
        let signature = vec![0x04, 0xaa, 0xbb];

        let cases = [
            (
                ACCOUNT_KEYCHAIN_ADDRESS,
                IAccountKeychain::authorizeAdminKeyCall {
                    keyId: key,
                    signatureType: IAccountKeychain::SignatureType::Secp256k1,
                    witness: B256::repeat_byte(0x11),
                }
                .abi_encode()
                .into(),
                "authorizeAdminKey(address,uint8,bytes32)",
            ),
            (
                ACCOUNT_KEYCHAIN_ADDRESS,
                IAccountKeychain::isAdminKeyCall { account, keyId: key }.abi_encode().into(),
                "isAdminKey(address,address)",
            ),
            (
                SIGNATURE_VERIFIER_ADDRESS,
                ISignatureVerifier::verifyKeychainCall {
                    account,
                    hash: B256::repeat_byte(0x22),
                    signature: signature.clone().into(),
                }
                .abi_encode()
                .into(),
                "verifyKeychain(address,bytes32,bytes)",
            ),
            (
                SIGNATURE_VERIFIER_ADDRESS,
                ISignatureVerifier::verifyKeychainAdminCall {
                    account,
                    hash: B256::repeat_byte(0x33),
                    signature: signature.into(),
                }
                .abi_encode()
                .into(),
                "verifyKeychainAdmin(address,bytes32,bytes)",
            ),
        ];

        for (address, data, signature) in cases {
            let decoded = decoder
                .decode_function(&CallTrace { address, data, success: true, ..Default::default() })
                .await;
            assert_eq!(
                decoded.call_data.expect("T6 keychain call should decode").signature,
                signature
            );
        }
    }

    #[tokio::test]
    async fn test_t6_receive_policy_and_admin_events_decode() {
        let decoder = CallTraceDecoder::new();
        let account = address!("0x0000000000000000000000000000000000000abc");
        let key = address!("0x0000000000000000000000000000000000000def");

        let events = [
            (
                ITIP403Registry::ReceivePolicyUpdated {
                    account,
                    senderPolicyId: 7,
                    tokenFilterId: 9,
                    recoveryAuthority: key,
                }
                .encode_log_data(),
                "ReceivePolicyUpdated",
            ),
            (
                IAccountKeychain::AdminKeyAuthorized { account, publicKey: key }.encode_log_data(),
                "AdminKeyAuthorized",
            ),
            (
                IReceivePolicyGuard::ReceiptClaimed {
                    token: PATH_USD_ADDRESS,
                    receiver: account,
                    blockedNonce: 11,
                    blockedAt: 12,
                    receiptVersion: 1,
                    originator: key,
                    recipient: account,
                    recoveryAuthority: key,
                    caller: key,
                    to: account,
                    amount: U256::from(123),
                }
                .encode_log_data(),
                "ReceiptClaimed",
            ),
            (
                IReceivePolicyGuard::ReceiptBurned {
                    token: PATH_USD_ADDRESS,
                    receiver: account,
                    blockedNonce: 11,
                    blockedAt: 12,
                    receiptVersion: 1,
                    originator: key,
                    recipient: account,
                    recoveryAuthority: key,
                    caller: key,
                    amount: U256::from(123),
                }
                .encode_log_data(),
                "ReceiptBurned",
            ),
        ];

        for (log, expected_name) in events {
            let decoded = decoder.decode_event(&log).await;
            assert_eq!(decoded.name.as_deref(), Some(expected_name));
            assert!(decoded.params.expect("event params should decode").len() >= 2);
        }
    }

    #[tokio::test]
    async fn test_t6_claim_receipt_bytes_decode_in_calls_and_transfer_blocked_event() {
        let decoder = CallTraceDecoder::new();
        let recovery = address!("0x0000000000000000000000000000000000000abc");
        let originator = address!("0x0000000000000000000000000000000000000def");
        let recipient = address!("0x0000000000000000000000000000000000000123");
        let receipt = IReceivePolicyGuard::ClaimReceiptV1::new(
            PATH_USD_ADDRESS,
            recovery,
            originator,
            recipient,
            12,
            34,
            ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8,
            IReceivePolicyGuard::InboundKind::TRANSFER,
            B256::repeat_byte(0x44),
        )
        .abi_encode();

        let claim =
            IReceivePolicyGuard::claimCall { to: recipient, receipt: receipt.clone().into() };
        let decoded = decoder
            .decode_function(&CallTrace {
                address: RECEIVE_POLICY_GUARD_ADDRESS,
                data: claim.abi_encode().into(),
                success: true,
                ..Default::default()
            })
            .await;
        let call_data = decoded.call_data.expect("claim should decode");
        assert_eq!(call_data.signature, "claim(address,bytes)");
        assert!(call_data.args[1].contains("ClaimReceiptV1"));
        assert!(call_data.args[1].contains("blockedNonce: 34"));
        assert!(call_data.args[1].contains(&originator.to_string()));

        let decoded = decoder
            .decode_function(&CallTrace {
                address: Address::from([0x77; 20]),
                data: claim.abi_encode().into(),
                success: true,
                ..Default::default()
            })
            .await;
        let call_data = decoded.call_data.expect("matching claim selector should decode");
        assert_eq!(call_data.signature, "claim(address,bytes)");
        assert!(!call_data.args[1].contains("ClaimReceiptV1"));
        assert!(call_data.args[1].starts_with("0x"));

        let blocked = IReceivePolicyGuard::TransferBlocked {
            token: PATH_USD_ADDRESS,
            receiver: recipient,
            blockedNonce: 34,
            amount: U256::from(123),
            receiptVersion: 1,
            receipt: receipt.into(),
        };
        let blocked_log = blocked.encode_log_data();
        let decoded =
            decoder.decode_event_with_address(RECEIVE_POLICY_GUARD_ADDRESS, &blocked_log).await;
        assert_eq!(decoded.name.as_deref(), Some("TransferBlocked"));
        let params = decoded.params.expect("TransferBlocked params should decode");
        let receipt = params.iter().find(|(name, _)| name == "receipt").unwrap();
        assert!(receipt.1.contains("ClaimReceiptV1"));
        assert!(receipt.1.contains("blockedReason: 2"));
        assert!(receipt.1.contains("kind: 0"));

        let decoded =
            decoder.decode_event_with_address(Address::from([0x77; 20]), &blocked_log).await;
        assert_eq!(decoded.name.as_deref(), Some("TransferBlocked"));
        let params = decoded.params.expect("TransferBlocked params should decode");
        let receipt = params.iter().find(|(name, _)| name == "receipt").unwrap();
        assert!(!receipt.1.contains("ClaimReceiptV1"));
        assert!(receipt.1.starts_with("0x"));
    }

    #[test]
    fn test_identify_addresses_does_not_skip_future_tempo_precompiles() {
        use foundry_evm_core::tempo::TIP20_CHANNEL_RESERVE_ADDRESS;

        let decoder = CallTraceDecoderBuilder::new()
            .with_chain_id(Some(4217))
            .with_tempo_hardfork(Some(TempoHardfork::T4))
            .build();

        let mut arena = CallTraceArena::default();
        let regular_addr = Address::from([0x42; 20]);
        arena.nodes_mut()[0].trace.address = regular_addr;

        arena.nodes_mut().push(CallTraceNode {
            trace: CallTrace {
                address: TIP20_CHANNEL_RESERVE_ADDRESS,
                depth: 1,
                maybe_precompile: None,
                ..Default::default()
            },
            idx: 1,
            ..Default::default()
        });

        let mut identifier = RecordingIdentifier { queried: Vec::new() };
        decoder.identify_addresses(&arena, &mut identifier);

        assert_eq!(identifier.queried, vec![regular_addr, TIP20_CHANNEL_RESERVE_ADDRESS]);
    }

    #[test]
    fn test_identify_addresses_does_not_skip_tempo_precompiles_on_other_chains() {
        use foundry_evm_core::tempo::TEMPO_PRECOMPILE_ADDRESSES;

        // Decoder with Ethereum mainnet chain ID (1).
        let mut decoder = CallTraceDecoder::new().clone();
        decoder.chain_id = Some(1);

        let mut arena = CallTraceArena::default();
        let regular_addr = Address::from([0x42; 20]);
        arena.nodes_mut()[0].trace.address = regular_addr;

        let tempo_precompile = TEMPO_PRECOMPILE_ADDRESSES[0];
        arena.nodes_mut().push(CallTraceNode {
            trace: CallTrace {
                address: tempo_precompile,
                depth: 1,
                maybe_precompile: None,
                ..Default::default()
            },
            idx: 1,
            ..Default::default()
        });

        let mut identifier = RecordingIdentifier { queried: Vec::new() };
        decoder.identify_addresses(&arena, &mut identifier);

        // On Ethereum, Tempo precompile addresses are regular contracts — should NOT be filtered.
        assert_eq!(identifier.queried, vec![regular_addr, tempo_precompile]);
    }
}
