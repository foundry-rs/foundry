use crate::{
    utils, utils::decode_cheatcode_outputs, CallTrace, LogCallOrder, RawOrDecodedLog,
    TraceCallData, TraceRetData,
};
use alloy_dyn_abi::{FunctionExt, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi as Abi};
use alloy_primitives::Address;
use ethers::types::{Action, Call, CallResult, Create, CreateResult, Res, Suicide};
use foundry_common::SELECTOR_LEN;
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, decode, utils::CallKind};
use foundry_utils::types::ToEthers;
use revm::interpreter::InstructionResult;
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
    pub fn status(&self) -> InstructionResult {
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
                address: self.trace.address.to_ethers(),
            }),
        }
    }

    /// Returns the `Action` for a parity trace
    pub fn parity_action(&self) -> Action {
        if self.status() == InstructionResult::SelfDestruct {
            return Action::Suicide(Suicide {
                address: self.trace.address.to_ethers(),
                // TODO deserialize from calldata here?
                refund_address: Default::default(),
                balance: self.trace.value.to_ethers(),
            })
        }
        match self.kind() {
            CallKind::Call | CallKind::StaticCall | CallKind::CallCode | CallKind::DelegateCall => {
                Action::Call(Call {
                    from: self.trace.caller.to_ethers(),
                    to: self.trace.address.to_ethers(),
                    value: self.trace.value.to_ethers(),
                    gas: self.trace.gas_cost.into(),
                    input: self.trace.data.as_bytes().to_vec().into(),
                    call_type: self.kind().into(),
                })
            }
            CallKind::Create | CallKind::Create2 => Action::Create(Create {
                from: self.trace.caller.to_ethers(),
                value: self.trace.value.to_ethers(),
                gas: self.trace.gas_cost.into(),
                init: self.trace.data.as_bytes().to_vec().into(),
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

        if let TraceCallData::Raw(ref bytes) = self.trace.data {
            let args = if bytes.len() >= SELECTOR_LEN {
                if self.trace.address == CHEATCODE_ADDRESS {
                    // Try to decode cheatcode inputs in a more custom way
                    utils::decode_cheatcode_inputs(func, bytes, errors, verbosity).unwrap_or_else(
                        || {
                            func.abi_decode_input(&bytes[SELECTOR_LEN..], false)
                                .expect("bad function input decode")
                                .iter()
                                .map(|token| utils::label(token, labels))
                                .collect()
                        },
                    )
                } else {
                    match func.abi_decode_input(&bytes[SELECTOR_LEN..], false) {
                        Ok(v) => v.iter().map(|token| utils::label(token, labels)).collect(),
                        Err(_) => Vec::new(),
                    }
                }
            } else {
                Vec::new()
            };

            // add signature to decoded calls for better calls filtering
            self.trace.data = TraceCallData::Decoded { signature: func.signature(), args };

            if let TraceRetData::Raw(bytes) = &self.trace.output {
                if self.trace.success {
                    if self.trace.address == CHEATCODE_ADDRESS {
                        if let Some(decoded) = funcs
                            .iter()
                            .find_map(|func| decode_cheatcode_outputs(func, bytes, verbosity))
                        {
                            self.trace.output = TraceRetData::Decoded(decoded);
                            return
                        }
                    }

                    if let Some(tokens) =
                        funcs.iter().find_map(|func| func.abi_decode_output(bytes, false).ok())
                    {
                        // Functions coming from an external database do not have any outputs
                        // specified, and will lead to returning an empty list of tokens.
                        if !tokens.is_empty() {
                            self.trace.output = TraceRetData::Decoded(
                                tokens
                                    .iter()
                                    .map(|token| utils::label(token, labels))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                        }
                    }
                } else {
                    self.trace.output = TraceRetData::Decoded(decode::decode_revert(
                        bytes,
                        Some(errors),
                        Some(self.trace.status),
                    ));
                }
            }
        }
    }
}
