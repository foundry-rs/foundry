use super::{
    identifier::{SingleSignaturesIdentifier, TraceIdentifier},
    CallTraceArena, RawOrDecodedCall, RawOrDecodedLog, RawOrDecodedReturnData,
};
use crate::{
    abi::{CHEATCODE_ADDRESS, CONSOLE_ABI, HARDHAT_CONSOLE_ABI, HARDHAT_CONSOLE_ADDRESS, HEVM_ABI},
    decode,
    executor::inspector::DEFAULT_CREATE2_DEPLOYER,
    trace::{node::CallTraceNode, utils},
};
use ethers::{
    abi::{Abi, Address, Event, Function, Param, ParamType, Token},
    types::H256,
};
use foundry_common::{abi::get_indexed_event, SELECTOR_LEN};
use hashbrown::HashSet;
use std::collections::{BTreeMap, HashMap};

/// Build a new [CallTraceDecoder].
#[derive(Default)]
pub struct CallTraceDecoderBuilder {
    decoder: CallTraceDecoder,
}

impl CallTraceDecoderBuilder {
    pub fn new() -> Self {
        Self { decoder: CallTraceDecoder::new() }
    }

    /// Add known labels to the decoder.
    pub fn with_labels(mut self, labels: BTreeMap<Address, String>) -> Self {
        for (address, label) in labels.into_iter() {
            self.decoder.labels.insert(address, label);
        }
        self
    }

    /// Add known events to the decoder.
    pub fn with_events(mut self, events: Vec<Event>) -> Self {
        events
            .into_iter()
            .map(|event| ((event.signature(), indexed_inputs(&event)), event))
            .for_each(|(sig, event)| {
                self.decoder.events.entry(sig).or_default().push(event);
            });
        self
    }

    pub fn with_verbosity(mut self, level: u8) -> Self {
        self.decoder.verbosity = level;
        self
    }

    /// Build the decoder.
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
#[derive(Default, Debug)]
pub struct CallTraceDecoder {
    /// Information for decoding precompile calls.
    pub precompiles: HashMap<Address, Function>,
    /// Addresses identified to be a specific contract.
    ///
    /// The values are in the form `"<artifact>:<contract>"`.
    pub contracts: HashMap<Address, String>,
    /// Address labels
    pub labels: HashMap<Address, String>,
    /// Information whether the contract address has a receive function
    pub receive_contracts: HashMap<Address, bool>,
    /// A mapping of signatures to their known functions
    pub functions: BTreeMap<[u8; 4], Vec<Function>>,
    /// All known events
    pub events: BTreeMap<(H256, usize), Vec<Event>>,
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
    pub fn new() -> Self {
        let functions = HARDHAT_CONSOLE_ABI
            .functions()
            .map(|func| (func.short_signature(), vec![func.clone()]))
            .chain(HEVM_ABI.functions().map(|func| (func.short_signature(), vec![func.clone()])))
            .collect::<BTreeMap<[u8; 4], Vec<Function>>>();

        Self {
            // TODO: These are the Ethereum precompiles. We should add a way to support precompiles
            // for other networks, too.
            precompiles: [
                precompile(
                    1,
                    "ecrecover",
                    [
                        ParamType::FixedBytes(32),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                    ],
                    [ParamType::Address],
                ),
                precompile(2, "keccak", [ParamType::Bytes], [ParamType::FixedBytes(32)]),
                precompile(3, "ripemd", [ParamType::Bytes], [ParamType::FixedBytes(32)]),
                precompile(4, "identity", [ParamType::Bytes], [ParamType::Bytes]),
                precompile(
                    5,
                    "modexp",
                    [
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Bytes,
                    ],
                    [ParamType::Bytes],
                ),
                precompile(
                    6,
                    "ecadd",
                    [
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                    ],
                    [ParamType::Uint(256), ParamType::Uint(256)],
                ),
                precompile(
                    7,
                    "ecmul",
                    [ParamType::Uint(256), ParamType::Uint(256), ParamType::Uint(256)],
                    [ParamType::Uint(256), ParamType::Uint(256)],
                ),
                precompile(
                    8,
                    "ecpairing",
                    [
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                        ParamType::Uint(256),
                    ],
                    [ParamType::Uint(256)],
                ),
                precompile(
                    9,
                    "blake2f",
                    [
                        ParamType::Uint(4),
                        ParamType::FixedBytes(64),
                        ParamType::FixedBytes(128),
                        ParamType::FixedBytes(16),
                        ParamType::FixedBytes(1),
                    ],
                    [ParamType::FixedBytes(64)],
                ),
            ]
            .into(),
            contracts: Default::default(),
            labels: [
                (CHEATCODE_ADDRESS, "VM".to_string()),
                (HARDHAT_CONSOLE_ADDRESS, "console".to_string()),
                (DEFAULT_CREATE2_DEPLOYER, "Create2Deployer".to_string()),
            ]
            .into(),
            functions,
            events: CONSOLE_ABI
                .events()
                .map(|event| ((event.signature(), indexed_inputs(event)), vec![event.clone()]))
                .collect::<BTreeMap<(H256, usize), Vec<Event>>>(),
            errors: Abi::default(),
            signature_identifier: None,
            receive_contracts: Default::default(),
            verbosity: u8::default(),
        }
    }

    pub fn add_signature_identifier(&mut self, identifier: SingleSignaturesIdentifier) {
        self.signature_identifier = Some(identifier);
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    pub fn identify(&mut self, trace: &CallTraceArena, identifier: &mut impl TraceIdentifier) {
        let unidentified_addresses = trace
            .addresses()
            .into_iter()
            .filter(|(address, _)| {
                !self.labels.contains_key(address) || !self.contracts.contains_key(address)
            })
            .collect();

        identifier.identify_addresses(unidentified_addresses).iter().for_each(|identity| {
            let address = identity.address;

            if let Some(contract) = &identity.contract {
                self.contracts.entry(address).or_insert_with(|| contract.to_string());
            }

            if let Some(label) = &identity.label {
                self.labels.entry(address).or_insert_with(|| label.to_string());
            }

            if let Some(abi) = &identity.abi {
                // Store known functions for the address
                abi.functions()
                    .map(|func| (func.short_signature(), func.clone()))
                    .for_each(|(sig, func)| self.functions.entry(sig).or_default().push(func));

                // Flatten events from all ABIs
                abi.events()
                    .map(|event| ((event.signature(), indexed_inputs(event)), event.clone()))
                    .for_each(|(sig, event)| {
                        self.events.entry(sig).or_default().push(event);
                    });

                // Flatten errors from all ABIs
                abi.errors().for_each(|error| {
                    let entry = self
                        .errors
                        .errors
                        .entry(error.name.clone())
                        .or_insert_with(Default::default);
                    entry.push(error.clone());
                });

                self.receive_contracts.entry(address).or_insert(abi.receive);
            }
        });
    }

    pub async fn decode(&self, traces: &mut CallTraceArena) {
        for node in traces.arena.iter_mut() {
            // Set contract name
            if let Some(contract) = self.contracts.get(&node.trace.address).cloned() {
                node.trace.contract = Some(contract);
            }

            // Set label
            if let Some(label) = self.labels.get(&node.trace.address).cloned() {
                node.trace.label = Some(label);
            }

            // Decode call
            if let Some(precompile_fn) = self.precompiles.get(&node.trace.address) {
                node.decode_precompile(precompile_fn, &self.labels);
            } else if let RawOrDecodedCall::Raw(ref bytes) = node.trace.data {
                if bytes.len() >= 4 {
                    if let Some(funcs) = self.functions.get(&bytes[..SELECTOR_LEN]) {
                        node.decode_function(funcs, &self.labels, &self.errors, self.verbosity);
                    } else if node.trace.address == DEFAULT_CREATE2_DEPLOYER {
                        node.trace.data =
                            RawOrDecodedCall::Decoded("create2".to_string(), String::new(), vec![]);
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
                    let has_receive = self
                        .receive_contracts
                        .get(&node.trace.address)
                        .copied()
                        .unwrap_or_default();
                    let func_name =
                        if bytes.is_empty() && has_receive { "receive" } else { "fallback" };

                    node.trace.data =
                        RawOrDecodedCall::Decoded(func_name.to_string(), String::new(), Vec::new());

                    if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                        if !node.trace.success {
                            if let Ok(decoded_error) = decode::decode_revert(
                                &bytes[..],
                                Some(&self.errors),
                                Some(node.trace.status),
                            ) {
                                node.trace.output = RawOrDecodedReturnData::Decoded(format!(
                                    r#""{decoded_error}""#
                                ));
                            }
                        }
                    }
                }
            }

            // Decode events
            self.decode_events(node).await;
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
            if raw_log.topics.is_empty() {
                return
            }

            let mut events = vec![];
            if let Some(evs) = self.events.get(&(raw_log.topics[0], raw_log.topics.len() - 1)) {
                events = evs.clone();
            } else if let Some(identifier) = &self.signature_identifier {
                if let Some(event) =
                    identifier.write().await.identify_event(&raw_log.topics[0].0).await
                {
                    events.push(get_indexed_event(event, raw_log));
                }
            }

            for mut event in events {
                // ensure all params are named, otherwise this will cause issues with decoding: See also <https://github.com/rust-ethereum/ethabi/issues/206>
                let empty_params = patch_nameless_params(&mut event);
                if let Ok(decoded) = event.parse_log(raw_log.clone()) {
                    *log = RawOrDecodedLog::Decoded(
                        event.name,
                        decoded
                            .params
                            .into_iter()
                            .map(|param| {
                                // undo patched names
                                let name = if empty_params.contains(&param.name) {
                                    "".to_string()
                                } else {
                                    param.name
                                };
                                (name, self.apply_label(&param.value))
                            })
                            .collect(),
                    );
                    break
                }
            }
        }
    }

    fn apply_label(&self, token: &Token) -> String {
        utils::label(token, &self.labels)
    }
}

/// This is a bit horrible but due to <https://github.com/rust-ethereum/ethabi/issues/206> we need to patch nameless (valid) params before decoding a logs, otherwise [`Event::parse_log()`] will result in wrong results since they're identified by name.
///
/// Returns a set of patched param names, that originally were empty.
fn patch_nameless_params(event: &mut Event) -> HashSet<String> {
    let mut patches = HashSet::new();
    if event.inputs.iter().filter(|input| input.name.is_empty()).count() > 1 {
        for (idx, param) in event.inputs.iter_mut().enumerate() {
            // this is an illegal arg name, which ensures patched identifiers are unique
            param.name = format!("<patched {idx}>");
            patches.insert(param.name.clone());
        }
    }
    patches
}

fn precompile<I, O>(number: u8, name: impl ToString, inputs: I, outputs: O) -> (Address, Function)
where
    I: IntoIterator<Item = ParamType>,
    O: IntoIterator<Item = ParamType>,
{
    (
        Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, number]),
        #[allow(deprecated)]
        Function {
            name: name.to_string(),
            inputs: inputs
                .into_iter()
                .map(|kind| Param { name: "".to_string(), kind, internal_type: None })
                .collect(),
            outputs: outputs
                .into_iter()
                .map(|kind| Param { name: "".to_string(), kind, internal_type: None })
                .collect(),
            constant: None,
            state_mutability: ethers::abi::StateMutability::Pure,
        },
    )
}

fn indexed_inputs(event: &Event) -> usize {
    event.inputs.iter().filter(|param| param.indexed).count()
}
