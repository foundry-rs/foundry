use crate::{
    decode,
    executor::CHEATCODE_ADDRESS,
    trace::{
        utils, utils::decode_cheatcode_outputs, CallTrace, LogCallOrder, RawOrDecodedCall,
        RawOrDecodedLog, RawOrDecodedReturnData,
    },
    CallKind,
};
use ethers::{
    abi::{Abi, Function},
    types::{Action, Address, Call, CallResult, Create, CreateResult, Res, Suicide},
};
use foundry_common::SELECTOR_LEN;
use revm::Return;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A node in the arena
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Returns the kind of call the trace belongs to
    pub fn kind(&self) -> CallKind {
        self.trace.kind
    }

    /// Returns the status of the call
    pub fn status(&self) -> Return {
        self.trace.status
    }

    /// Returns the `Res` for a parity trace
    pub fn parity_result(&self) -> Res {
        match self.kind() {
            CallKind::Call | CallKind::StaticCall | CallKind::CallCode | CallKind::DelegateCall => {
                Res::Call(CallResult {
                    gas_used: self.trace.gas_cost.into(),
                    output: self.trace.output.to_raw().into(),
                })
            }
            CallKind::Create | CallKind::Create2 => Res::Create(CreateResult {
                gas_used: self.trace.gas_cost.into(),
                code: self.trace.output.to_raw().into(),
                address: self.trace.address,
            }),
        }
    }

    /// Returns the `Action` for a parity trace
    pub fn parity_action(&self) -> Action {
        if self.status() == Return::SelfDestruct {
            return Action::Suicide(Suicide {
                address: self.trace.address,
                // TODO deserialize from calldata here?
                refund_address: Default::default(),
                balance: self.trace.value,
            })
        }
        match self.kind() {
            CallKind::Call | CallKind::StaticCall | CallKind::CallCode | CallKind::DelegateCall => {
                Action::Call(Call {
                    from: self.trace.caller,
                    to: self.trace.address,
                    value: self.trace.value,
                    gas: self.trace.gas_cost.into(),
                    input: self.trace.data.to_raw().into(),
                    call_type: self.kind().into(),
                })
            }
            CallKind::Create | CallKind::Create2 => Action::Create(Create {
                from: self.trace.caller,
                value: self.trace.value,
                gas: self.trace.gas_cost.into(),
                init: self.trace.data.to_raw().into(),
            }),
        }
    }

    /// Decode a regular function
    pub fn decode_function(
        &mut self,
        funcs: &[Function],
        labels: &HashMap<Address, String>,
        errors: &Abi,
        verbosity: u8,
    ) {
        debug_assert!(!funcs.is_empty(), "requires at least 1 func");
        // This is safe because (1) we would not have an entry for the given
        // selector if no functions with that selector were added and (2) the
        // same selector implies the function has
        // the same name and inputs.
        let func = &funcs[0];

        if let RawOrDecodedCall::Raw(ref bytes) = self.trace.data {
            let inputs = if bytes.len() >= SELECTOR_LEN {
                if self.trace.address == CHEATCODE_ADDRESS {
                    // Try to decode cheatcode inputs in a more custom way
                    utils::decode_cheatcode_inputs(func, bytes, errors, verbosity).unwrap_or_else(
                        || {
                            func.decode_input(&bytes[SELECTOR_LEN..])
                                .expect("bad function input decode")
                                .iter()
                                .map(|token| utils::label(token, labels))
                                .collect()
                        },
                    )
                } else {
                    match func.decode_input(&bytes[SELECTOR_LEN..]) {
                        Ok(v) => v.iter().map(|token| utils::label(token, labels)).collect(),
                        Err(_) => Vec::new(),
                    }
                }
            } else {
                Vec::new()
            };

            // add signature to decoded calls for better calls filtering
            self.trace.data =
                RawOrDecodedCall::Decoded(func.name.clone(), func.signature(), inputs);

            if let RawOrDecodedReturnData::Raw(bytes) = &self.trace.output {
                if !bytes.is_empty() && self.trace.success {
                    if self.trace.address == CHEATCODE_ADDRESS {
                        if let Some(decoded) = funcs
                            .iter()
                            .find_map(|func| decode_cheatcode_outputs(func, bytes, verbosity))
                        {
                            self.trace.output = RawOrDecodedReturnData::Decoded(decoded);
                            return
                        }
                    }

                    if let Some(tokens) =
                        funcs.iter().find_map(|func| func.decode_output(bytes).ok())
                    {
                        // Functions coming from an external database do not have any outputs
                        // specified, and will lead to returning an empty list of tokens.
                        if !tokens.is_empty() {
                            self.trace.output = RawOrDecodedReturnData::Decoded(
                                tokens
                                    .iter()
                                    .map(|token| utils::label(token, labels))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                        }
                    }
                } else if let Ok(decoded_error) =
                    decode::decode_revert(bytes, Some(errors), Some(self.trace.status))
                {
                    self.trace.output =
                        RawOrDecodedReturnData::Decoded(format!(r#""{decoded_error}""#));
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
                precompile_fn.signature(),
                precompile_fn.decode_input(bytes).map_or_else(
                    |_| vec![hex::encode(bytes)],
                    |tokens| tokens.iter().map(|token| utils::label(token, labels)).collect(),
                ),
            );

            if let RawOrDecodedReturnData::Raw(ref bytes) = self.trace.output {
                self.trace.output = RawOrDecodedReturnData::Decoded(
                    precompile_fn.decode_output(bytes).map_or_else(
                        |_| hex::encode(bytes),
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
