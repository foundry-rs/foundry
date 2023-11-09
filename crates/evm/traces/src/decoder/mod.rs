use crate::{
    identifier::{AddressIdentity, SingleSignaturesIdentifier, TraceIdentifier},
    utils::{self, decode_cheatcode_outputs},
    CallTrace, CallTraceArena, TraceCallData, TraceLog, TraceRetData,
};
use alloy_dyn_abi::{DecodedEvent, DynSolValue, EventExt, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Event, Function, JsonAbi as Abi};
use alloy_primitives::{Address, Selector, B256};
use foundry_common::{abi::get_indexed_event, SELECTOR_LEN};
use foundry_evm_core::{
    abi::{CONSOLE_ABI, HARDHAT_CONSOLE_ABI, HEVM_ABI},
    constants::{
        CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS,
        TEST_CONTRACT_ADDRESS,
    },
    decode,
};
use foundry_utils::types::ToAlloy;
use once_cell::sync::OnceCell;
use std::collections::{BTreeMap, HashMap};

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

    /// Add known events to the decoder.
    #[inline]
    pub fn with_events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        for event in events {
            self.decoder
                .events
                .entry((event.selector(), indexed_inputs(&event)))
                .or_default()
                .push(event);
        }
        self
    }

    /// Sets the verbosity level of the decoder.
    #[inline]
    pub fn with_verbosity(mut self, level: u8) -> Self {
        self.decoder.verbosity = level;
        self
    }

    /// Sets the signature identifier for events and functions.
    #[inline]
    pub fn with_signature_identifier(mut self, identifier: SingleSignaturesIdentifier) -> Self {
        self.decoder.signature_identifier = Some(identifier);
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
#[derive(Clone, Default, Debug)]
pub struct CallTraceDecoder {
    /// Addresses identified to be a specific contract.
    ///
    /// The values are in the form `"<artifact>:<contract>"`.
    pub contracts: HashMap<Address, String>,
    /// Address labels
    pub labels: HashMap<Address, String>,
    /// Information whether the contract address has a receive function
    pub receive_contracts: HashMap<Address, bool>,
    /// A mapping of signatures to their known functions
    pub functions: BTreeMap<Selector, Vec<Function>>,
    /// All known events
    pub events: BTreeMap<(B256, usize), Vec<Event>>,
    /// All known errors
    pub errors: Abi,
    /// A signature identifier for events and functions.
    pub signature_identifier: Option<SingleSignaturesIdentifier>,
    /// Verbosity level
    pub verbosity: u8,
}

impl CallTraceDecoder {
    /// Creates a new call trace decoder.
    ///
    /// The call trace decoder always knows how to decode calls to the cheatcode address, as well
    /// as DSTest-style logs.
    pub fn new() -> &'static Self {
        // If you want to take arguments in this function, assign them to the fields of the cloned
        // lazy instead of removing it
        static INIT: OnceCell<CallTraceDecoder> = OnceCell::new();
        INIT.get_or_init(Self::init)
    }

    fn init() -> Self {
        Self {
            contracts: Default::default(),

            labels: [
                (CHEATCODE_ADDRESS, "VM".to_string()),
                (HARDHAT_CONSOLE_ADDRESS, "console".to_string()),
                (DEFAULT_CREATE2_DEPLOYER, "Create2Deployer".to_string()),
                (CALLER, "DefaultSender".to_string()),
                (TEST_CONTRACT_ADDRESS, "DefaultTestContract".to_string()),
            ]
            .into(),

            functions: HARDHAT_CONSOLE_ABI
                .functions()
                .chain(HEVM_ABI.functions())
                .map(|func| {
                    let func = func.clone().to_alloy();
                    (func.selector(), vec![func])
                })
                .collect(),

            events: CONSOLE_ABI
                .events()
                .map(|event| {
                    let event = event.clone().to_alloy();
                    ((event.selector(), indexed_inputs(&event)), vec![event])
                })
                .collect(),

            errors: Default::default(),
            signature_identifier: None,
            receive_contracts: Default::default(),
            verbosity: 0,
        }
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    #[inline]
    pub fn identify(&mut self, trace: &CallTraceArena, identifier: &mut impl TraceIdentifier) {
        self.collect_identities(identifier.identify_addresses(self.addresses(trace)));
    }

    #[inline(always)]
    fn addresses<'a>(
        &'a self,
        trace: &'a CallTraceArena,
    ) -> impl Iterator<Item = (&'a Address, Option<&'a [u8]>)> + 'a {
        trace.addresses().into_iter().filter(|&(address, _)| {
            !self.labels.contains_key(address) || !self.contracts.contains_key(address)
        })
    }

    fn collect_identities(&mut self, identities: Vec<AddressIdentity<'_>>) {
        for identity in identities {
            let address = identity.address;

            if let Some(contract) = &identity.contract {
                self.contracts.entry(address).or_insert_with(|| contract.to_string());
            }

            if let Some(label) = &identity.label {
                self.labels.entry(address).or_insert_with(|| label.to_string());
            }

            if let Some(abi) = &identity.abi {
                // Store known functions for the address
                for function in abi.functions() {
                    self.functions.entry(function.selector()).or_default().push(function.clone())
                }

                // Flatten events from all ABIs
                for event in abi.events() {
                    let sig = (event.selector(), indexed_inputs(event));
                    self.events.entry(sig).or_default().push(event.clone());
                }

                // Flatten errors from all ABIs
                for error in abi.errors() {
                    self.errors.errors.entry(error.name.clone()).or_default().push(error.clone());
                }

                self.receive_contracts.entry(address).or_insert(abi.receive.is_some());
            }
        }
    }

    /// Decodes all nodes in the specified call trace.
    pub async fn decode(&self, traces: &mut CallTraceArena) {
        for node in &mut traces.arena {
            self.decode_function(&mut node.trace).await;
            for log in node.logs.iter_mut() {
                self.decode_event(log).await;
            }
        }
    }

    async fn decode_function(&self, trace: &mut CallTrace) {
        // Decode precompile
        if precompiles::decode(trace, 1) {
            return
        }

        // Set label
        if trace.label.is_none() {
            if let Some(label) = self.labels.get(&trace.address) {
                trace.label = Some(label.clone());
            }
        }

        // Set contract name
        if trace.contract.is_none() {
            if let Some(contract) = self.contracts.get(&trace.address) {
                trace.contract = Some(contract.clone());
            }
        }

        let TraceCallData::Raw(cdata) = &trace.data else { return };

        if trace.address == DEFAULT_CREATE2_DEPLOYER {
            trace.data = TraceCallData::Decoded { signature: "create2".to_string(), args: vec![] };
            return
        }

        if cdata.len() >= SELECTOR_LEN {
            let selector = &cdata[..SELECTOR_LEN];
            let mut functions = Vec::new();
            let functions = match self.functions.get(selector) {
                Some(fs) => fs,
                None => {
                    if let Some(identifier) = &self.signature_identifier {
                        if let Some(function) =
                            identifier.write().await.identify_function(selector).await
                        {
                            functions.push(function);
                        }
                    }
                    &functions
                }
            };
            let [func, ..] = &functions[..] else { return };
            self.decode_function_input(trace, func);
            self.decode_function_output(trace, functions);
        } else {
            let has_receive = self.receive_contracts.get(&trace.address).copied().unwrap_or(false);
            let signature =
                if cdata.is_empty() && has_receive { "receive()" } else { "fallback()" }.into();
            trace.data = TraceCallData::Decoded { signature, args: vec![cdata.to_string()] };

            if let TraceRetData::Raw(bytes) = &trace.output {
                if !trace.success {
                    trace.output = TraceRetData::Decoded(decode::decode_revert(
                        bytes,
                        Some(&self.errors),
                        Some(trace.status),
                    ));
                }
            }
        }
    }

    fn decode_function_input(&self, trace: &mut CallTrace, func: &Function) {
        let TraceCallData::Raw(data) = &trace.data else { return };
        let args = if data.len() >= SELECTOR_LEN {
            if trace.address == CHEATCODE_ADDRESS {
                // Try to decode cheatcode inputs in a more custom way
                utils::decode_cheatcode_inputs(func, data, &self.errors, self.verbosity)
                    .unwrap_or_else(|| {
                        func.abi_decode_input(&data[SELECTOR_LEN..], false)
                            .expect("bad function input decode")
                            .iter()
                            .map(|token| utils::label(token, &self.labels))
                            .collect()
                    })
            } else {
                match func.abi_decode_input(&data[SELECTOR_LEN..], false) {
                    Ok(v) => v.iter().map(|token| utils::label(token, &self.labels)).collect(),
                    Err(_) => Vec::new(),
                }
            }
        } else {
            Vec::new()
        };
        trace.data = TraceCallData::Decoded { signature: func.signature(), args };
    }

    fn decode_function_output(&self, trace: &mut CallTrace, funcs: &[Function]) {
        let TraceRetData::Raw(data) = &trace.output else { return };
        if trace.success {
            if trace.address == CHEATCODE_ADDRESS {
                if let Some(decoded) = funcs
                    .iter()
                    .find_map(|func| decode_cheatcode_outputs(func, data, self.verbosity))
                {
                    trace.output = TraceRetData::Decoded(decoded);
                    return
                }
            }

            if let Some(tokens) = funcs
                .iter()
                .filter(|f| !f.inputs.is_empty())
                .find_map(|func| func.abi_decode_output(data, false).ok())
            {
                trace.output = TraceRetData::Decoded(
                    tokens
                        .iter()
                        .map(|token| utils::label(token, &self.labels))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
        } else {
            trace.output = TraceRetData::Decoded(decode::decode_revert(
                data,
                Some(&self.errors),
                Some(trace.status),
            ));
        }
    }

    async fn decode_event(&self, log: &mut TraceLog) {
        let TraceLog::Raw(raw_log) = log else { return };
        let &[t0, ..] = raw_log.topics() else { return };

        let mut events = Vec::new();
        let events = match self.events.get(&(t0, raw_log.topics().len())) {
            Some(es) => es,
            None => {
                if let Some(identifier) = &self.signature_identifier {
                    if let Some(event) = identifier.write().await.identify_event(&t0[..]).await {
                        events.push(get_indexed_event(event, raw_log));
                    }
                }
                &events
            }
        };
        for event in events {
            if let Ok(decoded) = event.decode_log(raw_log, false) {
                let params = reconstruct_params(&event, &decoded);
                *log = TraceLog::Decoded(
                    event.name.clone(),
                    params
                        .into_iter()
                        .zip(event.inputs.iter())
                        .map(|(param, input)| {
                            // undo patched names
                            let name = input.name.clone();
                            (name, self.apply_label(&param))
                        })
                        .collect(),
                );
                break
            }
        }
    }

    fn apply_label(&self, token: &DynSolValue) -> String {
        utils::label(token, &self.labels)
    }
}

/// Restore the order of the params of a decoded event,
/// as Alloy returns the indexed and unindexed params separately.
fn reconstruct_params(event: &Event, decoded: &DecodedEvent) -> Vec<DynSolValue> {
    let mut indexed = 0;
    let mut unindexed = 0;
    let mut inputs = vec![];
    for input in event.inputs.iter() {
        if input.indexed {
            inputs.push(decoded.indexed[indexed].clone());
            indexed += 1;
        } else {
            inputs.push(decoded.body[unindexed].clone());
            unindexed += 1;
        }
    }

    inputs
}

fn indexed_inputs(event: &Event) -> usize {
    event.inputs.iter().filter(|param| param.indexed).count()
}
