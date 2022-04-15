use crate::{
    executor::CHEATCODE_ADDRESS,
    trace::{
        utils, CallTrace, LogCallOrder, RawOrDecodedCall, RawOrDecodedLog, RawOrDecodedReturnData,
    },
};
use ethers::{
    abi::{Abi, Function},
    types::Address,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A node in the arena
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct CallTraceNode {
    /// Parent node index in the arena
    pub parent: Option<usize>,
    /// Children node indexes in the arena
    pub children: Vec<usize>,
    /// This node's index in the arena
    pub idx: usize,
    /// The call trace
    pub trace: CallTrace,
    /// Logs
    #[serde(skip)]
    pub logs: Vec<RawOrDecodedLog>,
    /// Ordering of child calls and logs
    pub ordering: Vec<LogCallOrder>,
}

impl CallTraceNode {
    /// Decode a regular function
    pub fn decode_function(
        &mut self,
        funcs: &[Function],
        labels: &HashMap<Address, String>,
        errors: &Abi,
    ) {
        debug_assert!(!funcs.is_empty(), "requires at least 1 func");
        // This is safe because (1) we would not have an entry for the given
        // selector if no functions with that selector were added and (2) the
        // same selector implies the function has
        // the same name and inputs.
        let func = &funcs[0];

        if let RawOrDecodedCall::Raw(ref bytes) = self.trace.data {
            let inputs = if !bytes[4..].is_empty() {
                if self.trace.address == CHEATCODE_ADDRESS {
                    // Try to decode cheatcode inputs in a more custom way
                    utils::decode_cheatcode_inputs(func, bytes, errors).unwrap_or_else(|| {
                        func.decode_input(&bytes[4..])
                            .expect("bad function input decode")
                            .iter()
                            .map(|token| utils::label(token, labels))
                            .collect()
                    })
                } else {
                    match func.decode_input(&bytes[4..]) {
                        Ok(v) => v.iter().map(|token| utils::label(token, labels)).collect(),
                        Err(_) => Vec::new(),
                    }
                }
            } else {
                Vec::new()
            };
            self.trace.data = RawOrDecodedCall::Decoded(func.name.clone(), inputs);

            if let RawOrDecodedReturnData::Raw(bytes) = &self.trace.output {
                if !bytes.is_empty() {
                    if self.trace.success {
                        if let Some(tokens) =
                            funcs.iter().find_map(|func| func.decode_output(&bytes[..]).ok())
                        {
                            self.trace.output = RawOrDecodedReturnData::Decoded(
                                tokens
                                    .iter()
                                    .map(|token| utils::label(token, labels))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                        }
                    } else if let Ok(decoded_error) =
                        foundry_utils::decode_revert(&bytes[..], Some(errors))
                    {
                        self.trace.output =
                            RawOrDecodedReturnData::Decoded(format!(r#""{}""#, decoded_error));
                    }
                }
            }
        }
    }

    /// Decode the node's tracing data for the given precompile function
    pub fn decode_precompile(
        &mut self,
        precompile_fn: &Function,
        labels: &HashMap<Address, String>,
    ) {
        if let RawOrDecodedCall::Raw(ref bytes) = self.trace.data {
            self.trace.label = Some("PRECOMPILE".to_string());
            self.trace.data = RawOrDecodedCall::Decoded(
                precompile_fn.name.clone(),
                precompile_fn.decode_input(&bytes[..]).map_or_else(
                    |_| vec![hex::encode(&bytes)],
                    |tokens| tokens.iter().map(|token| utils::label(token, labels)).collect(),
                ),
            );

            if let RawOrDecodedReturnData::Raw(ref bytes) = self.trace.output {
                self.trace.output = RawOrDecodedReturnData::Decoded(
                    precompile_fn.decode_output(&bytes[..]).map_or_else(
                        |_| hex::encode(&bytes),
                        |tokens| {
                            tokens
                                .iter()
                                .map(|token| utils::label(token, labels))
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                    ),
                );
            }
        }
    }
}
