use crate::Evm;

use ethers::{
    abi::{Detokenize, Function, Tokenize},
    prelude::{decode_function_data, encode_function_data},
    types::{Address, Bytes, U256},
    utils::CompiledContract,
};

use evmodin::{tracing::Tracer, AnalyzedCode, CallKind, Host, Message, Revision, StatusCode};

use std::collections::BTreeMap;

use eyre::Result;

// pub type MemoryState = BTreeMap<Address, MemoryAccount>;

// TODO: Check if we can implement this as the base layer of an ethers-provider
// Middleware stack instead of doing RPC calls.
pub struct EvmOdin<S, T> {
    pub host: S,
    pub gas_limit: u64,
    pub revision: Revision,
    /// The contract must be set in order to be able to instantiate the EVMOdin
    /// analyzer
    pub contract: Option<CompiledContract>,
    pub tracer: T,
}

impl<S: Host, T: Tracer> EvmOdin<S, T> {
    /// Given a gas limit, vm revision, and initialized host state
    pub fn new(host: S, gas_limit: u64, revision: Revision, tracer: T) -> Self {
        Self { host, gas_limit, revision, contract: None, tracer }
    }
}

impl<S: Host, Tr: Tracer> Evm<S> for EvmOdin<S, Tr> {
    type ReturnReason = StatusCode;

    fn load_contract_info(&mut self, contract: CompiledContract) {
        self.contract = Some(contract);
    }

    fn reset(&mut self, state: S) {
        unimplemented!()
        // let state_ = self.executor.state_mut();
        // *state_ = state;
    }

    fn init_state(&self) -> S
    where
        S: Clone,
    {
        unimplemented!()
        // self.executor.state().clone()
    }

    fn check_success(
        &mut self,
        address: Address,
        result: Self::ReturnReason,
        should_fail: bool,
    ) -> bool {
        if should_fail {
            match result {
                // If the function call reverted, we're good.
                StatusCode::Revert => true,
                // If the function call was successful in an expected fail case,
                // we make a call to the `failed()` function inherited from DS-Test
                StatusCode::Success => self.failed(address).unwrap_or(false),
                err => {
                    tracing::error!(?err);
                    false
                }
            }
        } else {
            matches!(result, StatusCode::Success)
        }
    }

    /// Runs the selected function
    fn call<D: Detokenize, T: Tokenize>(
        &mut self,
        from: Address,
        to: Address,
        func: &Function,
        args: T, // derive arbitrary for Tokenize?
        value: U256,
    ) -> Result<(D, Self::ReturnReason, u64)> {
        //
        let calldata = encode_function_data(func, args)?;

        // For the `func.constant` field usage
        #[allow(deprecated)]
        let message = Message {
            sender: from,
            destination: to,
            // What should this be?
            depth: 0,
            // I think? Should this be configured at the VM constructor?
            kind: CallKind::Call,
            input_data: calldata.0.into(),
            value,
            gas: self.gas_limit as i64,
            is_static: func.constant ||
                matches!(
                    func.state_mutability,
                    ethers::abi::StateMutability::View | ethers::abi::StateMutability::Pure
                ),
        };

        // None is state_modifier, we may want to use it in the future for cheat codes in a
        // non-invasive way?
        let bytecode = self
            .contract
            .clone()
            .map(|c| c.runtime_bytecode)
            .ok_or_else(|| eyre::eyre!("no bytecode set for evm contract execution"))?;
        let bytecode = AnalyzedCode::analyze(bytecode.as_ref());
        let output =
            bytecode.execute(&mut self.host, &mut self.tracer, None, message, self.revision);

        // let gas = dapp_utils::remove_extra_costs(gas_before - gas_after, calldata.as_ref());

        let retdata = decode_function_data(func, output.output_data, false)?;
        let gas = U256::from(0);

        Ok((retdata, output.status_code, gas.as_u64()))
    }
}

#[cfg(test)]
// TODO: Check that the simple unit test passes for evmodin
mod tests {
    use super::*;
    use crate::test_helpers::{can_call_vm_directly, COMPILED};
    use evmodin::{tracing::NoopTracer, util::mocked_host::MockedHost};

    #[test]
    fn evmodin_can_call_vm_directly() {
        let revision = Revision::Istanbul;
        let compiled = COMPILED.get("Greeter").expect("could not find contract");

        let addr: Address = "0x1000000000000000000000000000000000000000".parse().unwrap();
        // let state = initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);

        let host = MockedHost::default();
        let gas_limit = 12_000_000;
        let mut evm = EvmOdin::new(host, gas_limit, revision, NoopTracer);
        evm.load_contract_info(compiled.clone());

        let results = can_call_vm_directly(evm, addr);
        assert_eq!(results, vec![StatusCode::Success; 2]);
        // assert_eq!(results[0], ExitReason::Succeed(ExitSucceed::Stopped));
        // assert_eq!(results[1], ExitReason::Succeed(ExitSucceed::Returned));
    }
}

// /// given an iterator of contract address to contract bytecode, initializes
// /// the state with the contract deployed at the specified address
// pub fn initialize_contracts<T: IntoIterator<Item = (Address, Bytes)>>(contracts: T) ->
// MemoryState {     contracts
//         .into_iter()
//         .map(|(address, bytecode)| {
//             (
//                 address,
//                 MemoryAccount {
//                     nonce: U256::one(),
//                     balance: U256::zero(),
//                     storage: BTreeMap::new(),
//                     code: bytecode.to_vec(),
//                 },
//             )
//         })
//         .collect::<BTreeMap<_, _>>()
// }
