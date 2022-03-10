use super::{
    CallTraceArena, RawOrDecodedCall, RawOrDecodedLog, RawOrDecodedReturnData, TraceIdentifier,
};
use crate::abi::{CHEATCODE_ADDRESS, CONSOLE_ABI, HEVM_ABI};
use ethers::{
    abi::{Abi, Address, Event, Function, Token},
    types::H256,
};
use foundry_utils::format_token;
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct CallTraceDecoder {
    /// Address labels
    labels: BTreeMap<Address, String>,
    /// All known functions
    functions: BTreeMap<[u8; 4], Function>,
    /// All known events
    events: BTreeMap<H256, Event>,
    /// All known errors
    errors: Abi,
}

impl CallTraceDecoder {
    /// Creates a new call trace decoder.
    ///
    /// The call trace decoder always knows how to decode calls to the cheatcode address, as well
    /// as DSTest-style logs.
    pub fn new() -> Self {
        Self {
            labels: [(*CHEATCODE_ADDRESS, "VM".to_string())].into(),
            functions: HEVM_ABI
                .functions()
                .map(|func| (func.short_signature(), func.clone()))
                .collect::<BTreeMap<[u8; 4], Function>>(),
            events: CONSOLE_ABI
                .events()
                .map(|event| (event.signature(), event.clone()))
                .collect::<BTreeMap<H256, Event>>(),
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

    // TODO: Identify using multiple identifiers
    pub fn identify(&mut self, trace: &CallTraceArena, identifier: &impl TraceIdentifier) {
        trace.addresses_iter().for_each(|(address, code)| {
            let (label, abi) = identifier.identify_address(address, code);

            if let Some(label) = label {
                self.labels.entry(*address).or_insert(label);
            }

            if let Some(abi) = abi {
                // Flatten functions from all ABIs
                abi.functions().map(|func| (func.short_signature(), func.clone())).for_each(
                    |(sig, func)| {
                        self.functions.insert(sig, func);
                    },
                );

                // Flatten events from all ABIs
                abi.events().map(|event| (event.signature(), event.clone())).for_each(
                    |(sig, event)| {
                        self.events.insert(sig, event);
                    },
                );

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
            // Set label
            if let Some(label) = self.labels.get(&node.trace.address) {
                node.trace.label = Some(label.clone());
            }

            // Decode call
            if let RawOrDecodedCall::Raw(bytes) = &node.trace.data {
                if bytes.len() >= 4 {
                    if let Some(func) = self.functions.get(&bytes[0..4]) {
                        // Decode inputs
                        let inputs = if !bytes[4..].is_empty() {
                            if node.trace.address == *CHEATCODE_ADDRESS {
                                // Try to decode cheatcode inputs in a more custom way
                                self.decode_cheatcode_inputs(func, bytes).unwrap_or_else(|| {
                                    func.decode_input(&bytes[4..])
                                        .expect("bad function input decode")
                                        .iter()
                                        .map(|token| self.apply_label(token))
                                        .collect()
                                })
                            } else {
                                func.decode_input(&bytes[4..])
                                    .expect("bad function input decode")
                                    .iter()
                                    .map(|token| self.apply_label(token))
                                    .collect()
                            }
                        } else {
                            Vec::new()
                        };

                        node.trace.data = RawOrDecodedCall::Decoded(func.name.clone(), inputs);

                        if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                            if !bytes.is_empty() {
                                if node.trace.success {
                                    if let Ok(tokens) = func.decode_output(&bytes[..]) {
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
                    if let Some(event) = self.events.get(&raw_log.topics[0]) {
                        if let Ok(decoded) = event.parse_log(raw_log.clone()) {
                            *log = RawOrDecodedLog::Decoded(
                                event.name.clone(),
                                decoded
                                    .params
                                    .into_iter()
                                    .map(|param| (param.name, self.apply_label(&param.value)))
                                    .collect(),
                            )
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
