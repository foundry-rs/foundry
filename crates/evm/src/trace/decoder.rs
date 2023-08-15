use super::{
    identifier::{SingleSignaturesIdentifier, TraceIdentifier},
    CallTraceArena, RawOrDecodedCall, RawOrDecodedLog, RawOrDecodedReturnData,
};
use crate::{
    abi::{CHEATCODE_ADDRESS, CONSOLE_ABI, HARDHAT_CONSOLE_ABI, HARDHAT_CONSOLE_ADDRESS, HEVM_ABI},
    decode,
    executor::inspector::DEFAULT_CREATE2_DEPLOYER,
    trace::{node::CallTraceNode, utils},
    CALLER, TEST_CONTRACT_ADDRESS,
};
use ethers::{
    abi::{Abi, Address, Event, Function, Param, ParamType, Token},
    types::{H160, H256},
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
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = (Address, String)>) -> Self {
        self.decoder.labels.extend(labels);
        self
    }

    /// Add known events to the decoder.
    pub fn with_events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        for event in events {
            self.decoder
                .events
                .entry((event.signature(), indexed_inputs(&event)))
                .or_default()
                .push(event);
        }
        self
    }

    /// Sets the verbosity level of the decoder.
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

/// Returns an expression of the type `[(Address, Function); N]`
macro_rules! precompiles {
    ($($number:literal : $name:ident($( $name_in:ident : $in:expr ),* $(,)?) -> ($( $name_out:ident : $out:expr ),* $(,)?)),+ $(,)?) => {{
        use std::string::String as RustString;
        use ParamType::*;
        [$(
            (
                H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, $number]),
                #[allow(deprecated)]
                Function {
                    name: RustString::from(stringify!($name)),
                    inputs: vec![$(Param { name: RustString::from(stringify!($name_in)), kind: $in, internal_type: None, }),*],
                    outputs: vec![$(Param { name: RustString::from(stringify!($name_out)), kind: $out, internal_type: None, }),*],
                    constant: None,
                    state_mutability: ethers::abi::StateMutability::Pure,
                },
            ),
        )+]
    }};
}

impl CallTraceDecoder {
    /// Creates a new call trace decoder.
    ///
    /// The call trace decoder always knows how to decode calls to the cheatcode address, as well
    /// as DSTest-style logs.
    pub fn new() -> Self {
        Self {
            // TODO: These are the Ethereum precompiles. We should add a way to support precompiles
            // for other networks, too.
            precompiles: precompiles!(
                0x01: ecrecover(hash: FixedBytes(32), v: Uint(256), r: Uint(256), s: Uint(256)) -> (publicAddress: Address),
                0x02: sha256(data: Bytes) -> (hash: FixedBytes(32)),
                0x03: ripemd(data: Bytes) -> (hash: FixedBytes(32)),
                0x04: identity(data: Bytes) -> (data: Bytes),
                0x05: modexp(Bsize: Uint(256), Esize: Uint(256), Msize: Uint(256), BEM: Bytes) -> (value: Bytes),
                0x06: ecadd(x1: Uint(256), y1: Uint(256), x2: Uint(256), y2: Uint(256)) -> (x: Uint(256), y: Uint(256)),
                0x07: ecmul(x1: Uint(256), y1: Uint(256), s: Uint(256)) -> (x: Uint(256), y: Uint(256)),
                0x08: ecpairing(x1: Uint(256), y1: Uint(256), x2: Uint(256), y2: Uint(256), x3: Uint(256), y3: Uint(256)) -> (success: Uint(256)),
                0x09: blake2f(rounds: Uint(4), h: FixedBytes(64), m: FixedBytes(128), t: FixedBytes(16), f: FixedBytes(1)) -> (h: FixedBytes(64)),
            ).into(),

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
                .map(|func| (func.short_signature(), vec![func.clone()]))
                .collect(),

            events: CONSOLE_ABI
                .events()
                .map(|event| ((event.signature(), indexed_inputs(event)), vec![event.clone()]))
                .collect(),

            errors: Default::default(),
            signature_identifier: None,
            receive_contracts: Default::default(),
            verbosity: 0,
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
                    let entry = self.errors.errors.entry(error.name.clone()).or_default();
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

fn indexed_inputs(event: &Event) -> usize {
    event.inputs.iter().filter(|param| param.indexed).count()
}
