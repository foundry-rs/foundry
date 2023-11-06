use crate::{
    identifier::{AddressIdentity, SingleSignaturesIdentifier, TraceIdentifier},
    node::CallTraceNode,
    utils, CallTraceArena, RawOrDecodedLog, TraceCallData, TraceRetData,
};
use alloy_dyn_abi::{DecodedEvent, DynSolValue, EventExt};
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
            // Set contract name
            if let Some(contract) = self.contracts.get(&node.trace.address) {
                node.trace.contract = Some(contract.clone());
            }

            // Set label
            if let Some(label) = self.labels.get(&node.trace.address) {
                node.trace.label = Some(label.clone());
            }

            // Decode events
            self.decode_events(node).await;

            // Decode call
            // TODO: chain ID argument
            if precompiles::decode(&mut node.trace, 1) {
                return
            }

            if let TraceCallData::Raw(bytes) = &node.trace.data {
                if bytes.len() >= SELECTOR_LEN {
                    if let Some(funcs) = self.functions.get(&bytes[..SELECTOR_LEN]) {
                        node.decode_function(funcs, &self.labels, &self.errors, self.verbosity);
                    } else if node.trace.address == DEFAULT_CREATE2_DEPLOYER {
                        node.trace.data = TraceCallData::Decoded {
                            signature: "create2".to_string(),
                            args: vec![],
                        };
                    } else if let Some(identifier) = &self.signature_identifier {
                        if let Some(function) =
                            identifier.write().await.identify_function(&bytes[..SELECTOR_LEN]).await
                        {
                            node.decode_function(
                                &[function],
                                &self.labels,
                                &self.errors,
                                self.verbosity,
                            );
                        }
                    }
                } else {
                    let has_receive =
                        self.receive_contracts.get(&node.trace.address).copied().unwrap_or(false);
                    let signature =
                        if bytes.is_empty() && has_receive { "receive()" } else { "fallback()" }
                            .into();
                    node.trace.data = TraceCallData::Decoded { signature, args: Vec::new() };

                    if let TraceRetData::Raw(bytes) = &node.trace.output {
                        if !node.trace.success {
                            node.trace.output = TraceRetData::Decoded(decode::decode_revert(
                                bytes,
                                Some(&self.errors),
                                Some(node.trace.status),
                            ));
                        }
                    }
                }
            }
        }
    }

    async fn decode_events(&self, node: &mut CallTraceNode) {
        for log in node.logs.iter_mut() {
            self.decode_event(log).await;
        }
    }

    async fn decode_event(&self, log: &mut RawOrDecodedLog) {
        if let RawOrDecodedLog::Raw(raw_log) = log {
            // do not attempt decoding if no topics
            if raw_log.topics().is_empty() {
                return
            }

            let mut events = vec![];
            if let Some(evs) = self.events.get(&(raw_log.topics()[0], raw_log.topics().len() - 1)) {
                events = evs.clone();
            } else if let Some(identifier) = &self.signature_identifier {
                if let Some(event) =
                    identifier.write().await.identify_event(&raw_log.topics()[0].0).await
                {
                    events.push(get_indexed_event(event, raw_log));
                }
            }

            for event in events {
                if let Ok(decoded) = event.decode_log(raw_log, false) {
                    let params = reconstruct_params(&event, &decoded);
                    *log = RawOrDecodedLog::Decoded(
                        event.name,
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
