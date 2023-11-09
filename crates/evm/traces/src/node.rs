use crate::{CallTrace, LogCallOrder, TraceLog};
use ethers::types::{Action, Call, CallResult, Create, CreateResult, Res, Suicide};
use foundry_evm_core::utils::CallKind;
use foundry_utils::types::ToEthers;
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};

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
    pub logs: Vec<TraceLog>,
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
}
