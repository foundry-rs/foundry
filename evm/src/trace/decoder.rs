use super::{
    CallTraceArena, RawOrDecodedCall, RawOrDecodedLog, RawOrDecodedReturnData, TraceIdentifier,
};
use crate::abi::{CHEATCODE_ADDRESS, CONSOLE_ABI, HEVM_ABI};
use ethers::{
    abi::{Abi, Address, Event, Function, Param, ParamType, Token},
    types::H256,
};
use foundry_utils::format_token;
use std::collections::BTreeMap;

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
    pub precompiles: BTreeMap<Address, Function>,
    /// Addresses identified to be a specific contract.
    ///
    /// The values are in the form `"<artifact>:<contract>"`.
    pub contracts: BTreeMap<Address, String>,
    /// Address labels
    pub labels: BTreeMap<Address, String>,
    /// A mapping of addresses to their known functions
    pub functions: BTreeMap<[u8; 4], Vec<Function>>,
    /// All known events
    pub events: BTreeMap<(H256, usize), Vec<Event>>,
    /// All known errors
    pub errors: Abi,
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
            contracts: BTreeMap::new(),
            labels: [(CHEATCODE_ADDRESS, "VM".to_string())].into(),
            functions: HEVM_ABI
                .functions()
                .map(|func| (func.short_signature(), vec![func.clone()]))
                .collect::<BTreeMap<[u8; 4], Vec<Function>>>(),
            events: CONSOLE_ABI
                .events()
                .map(|event| ((event.signature(), indexed_inputs(event)), vec![event.clone()]))
                .collect::<BTreeMap<(H256, usize), Vec<Event>>>(),
            errors: Abi::default(),
        }
    }

    /// Creates a new call trace decoder with predetermined address labels.
    pub fn new_with_labels(labels: BTreeMap<Address, String>) -> Self {
        let mut info = Self::new();
        for (address, label) in labels.into_iter() {
            info.labels.insert(address, label);
        }
        info
    }

    /// Identify unknown addresses in the specified call trace using the specified identifier.
    ///
    /// Unknown contracts are contracts that either lack a label or an ABI.
    pub fn identify(&mut self, trace: &CallTraceArena, identifier: &impl TraceIdentifier) {
        trace.addresses_iter().for_each(|(address, code)| {
            // We only try to identify addresses with missing data
            if self.labels.contains_key(address) && self.contracts.contains_key(address) {
                return
            }

            let (contract, label, abi) = identifier.identify_address(address, code);
            if let Some(contract) = contract {
                self.contracts.entry(*address).or_insert(contract);
            }

            if let Some(label) = label {
                self.labels.entry(*address).or_insert(label);
            }

            if let Some(abi) = abi {
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
            }
        });
    }

    pub fn decode(&self, traces: &mut CallTraceArena) {
        for node in traces.arena.iter_mut() {
            // Set contract name
            if let Some(contract) = self.contracts.get(&node.trace.address) {
                node.trace.contract = Some(contract.clone());
            }

            // Set label
            if let Some(label) = self.labels.get(&node.trace.address) {
                node.trace.label = Some(label.clone());
            }

            // Decode call
            if let RawOrDecodedCall::Raw(bytes) = &node.trace.data {
                if let Some(precompile_fn) = self.precompiles.get(&node.trace.address) {
                    node.trace.label = Some("PRECOMPILE".to_string());
                    node.trace.data = RawOrDecodedCall::Decoded(
                        precompile_fn.name.clone(),
                        precompile_fn.decode_input(&bytes[..]).map_or_else(
                            |_| vec![hex::encode(&bytes)],
                            |tokens| tokens.iter().map(|token| self.apply_label(token)).collect(),
                        ),
                    );

                    if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                        node.trace.output = RawOrDecodedReturnData::Decoded(
                            precompile_fn.decode_output(&bytes[..]).map_or_else(
                                |_| hex::encode(&bytes),
                                |tokens| {
                                    tokens
                                        .iter()
                                        .map(|token| self.apply_label(token))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                },
                            ),
                        );
                    }
                } else if bytes.len() >= 4 {
                    if let Some(funcs) = self.functions.get(&bytes[0..4]) {
                        // This is safe because (1) we would not have an entry for the given
                        // selector if no functions with that selector were added and (2) the same
                        // selector implies the function has the same name and inputs.
                        let func = &funcs[0];

                        // Decode inputs
                        let inputs = if !bytes[4..].is_empty() {
                            if node.trace.address == CHEATCODE_ADDRESS {
                                // Try to decode cheatcode inputs in a more custom way
                                self.decode_cheatcode_inputs(func, bytes).unwrap_or_else(|| {
                                    func.decode_input(&bytes[4..])
                                        .expect("bad function input decode")
                                        .iter()
                                        .map(|token| self.apply_label(token))
                                        .collect()
                                })
                            } else {
                                match func.decode_input(&bytes[4..]) {
                                    Ok(v) => {
                                        v.iter().map(|token| self.apply_label(token)).collect()
                                    }
                                    Err(_) => Vec::new(),
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        node.trace.data = RawOrDecodedCall::Decoded(func.name.clone(), inputs);

                        if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                            if !bytes.is_empty() {
                                if node.trace.success {
                                    if let Some(tokens) = funcs
                                        .iter()
                                        .find_map(|func| func.decode_output(&bytes[..]).ok())
                                    {
                                        node.trace.output = RawOrDecodedReturnData::Decoded(
                                            tokens
                                                .iter()
                                                .map(|token| self.apply_label(token))
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                        );
                                    }
                                } else if let Ok(decoded_error) =
                                    foundry_utils::decode_revert(&bytes[..], Some(&self.errors))
                                {
                                    node.trace.output = RawOrDecodedReturnData::Decoded(format!(
                                        r#""{}""#,
                                        decoded_error
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    node.trace.data = RawOrDecodedCall::Decoded("fallback".to_string(), Vec::new());

                    if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                        if !node.trace.success {
                            if let Ok(decoded_error) =
                                foundry_utils::decode_revert(&bytes[..], Some(&self.errors))
                            {
                                node.trace.output = RawOrDecodedReturnData::Decoded(format!(
                                    r#""{}""#,
                                    decoded_error
                                ));
                            }
                        }
                    }
                }
            }

            // Decode events
            node.logs.iter_mut().for_each(|log| {
                if let RawOrDecodedLog::Raw(raw_log) = log {
                    if let Some(events) =
                        self.events.get(&(raw_log.topics[0], raw_log.topics.len() - 1))
                    {
                        for event in events {
                            if let Ok(decoded) = event.parse_log(raw_log.clone()) {
                                *log = RawOrDecodedLog::Decoded(
                                    event.name.clone(),
                                    decoded
                                        .params
                                        .into_iter()
                                        .map(|param| (param.name, self.apply_label(&param.value)))
                                        .collect(),
                                );
                                break
                            }
                        }
                    }
                }
            });
        }
    }

    fn apply_label(&self, token: &Token) -> String {
        match token {
            Token::Address(addr) => {
                if let Some(label) = self.labels.get(addr) {
                    format!("{}: [{:?}]", label, addr)
                } else {
                    format_token(token)
                }
            }
            _ => format_token(token),
        }
    }

    fn decode_cheatcode_inputs(&self, func: &Function, data: &[u8]) -> Option<Vec<String>> {
        match func.name.as_str() {
            "expectRevert" => foundry_utils::decode_revert(data, Some(&self.errors))
                .ok()
                .map(|decoded| vec![decoded]),
            _ => None,
        }
    }
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
