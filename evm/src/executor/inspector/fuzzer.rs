use crate::{
    fuzz::{invariant::RandomCallGenerator, strategies::EvmFuzzState},
    utils,
};
use bytes::Bytes;
use ethers::prelude::H160;
use proptest::strategy::Strategy;
use revm::{db::Database, CallInputs, CallScheme, EVMData, Gas, Inspector, Interpreter, Return};

/// An inspector that can fuzz and collect data for that effect.
#[derive(Clone, Debug)]
pub struct Fuzzer {
    pub generator: RandomCallGenerator,
    pub fuzz_state: EvmFuzzState,
    pub collect: bool,
}

impl<DB> Inspector<DB> for Fuzzer
where
    DB: Database,
{
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if data.env.tx.caller != call.context.caller {
            self.reentrancy(call);
        }

        self.collect = true;

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        // We only collect before and after calls
        if self.collect {
            self.collect_data(interpreter);
            self.collect = false;
        }
        Return::Continue
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.generator.used = false;
        self.collect = true;

        (status, remaining_gas, retdata)
    }
}

impl Fuzzer {
    fn collect_data(&mut self, interpreter: &mut Interpreter) {
        let mut state = self.fuzz_state.write().unwrap();

        for slot in interpreter.stack().data() {
            state.insert(utils::u256_to_h256_be(*slot).into());
        }

        for index in 0..interpreter.memory.len() / 32 {
            let mut slot = [0u8; 32];
            slot.clone_from_slice(interpreter.memory.get_slice(index * 32, 32));

            state.insert(slot);
        }
    }

    fn reentrancy(&mut self, call: &mut CallInputs) {
        // We only override external calls which are not coming from the test contract
        if call.context.caller !=
            H160([
                180, 199, 157, 171, 143, 37, 156, 122, 238, 110, 91, 42, 167, 41, 130, 24, 100, 34,
                126, 132,
            ]) &&
            call.context.scheme == CallScheme::Call &&
            !self.generator.used
        {
            let (sender, (contract, input)) = if !self.generator.replay {
                let mut testrunner = self.generator.runner.write().unwrap();
                let mut random_call;
                loop {
                    let mut reentrant_call =
                        self.generator.strat.new_tree(&mut testrunner).unwrap().current();
                    random_call = reentrant_call.pop().unwrap().1;

                    // Only accepting calls made to the one who called us.
                    if random_call.0 == call.context.caller {
                        break
                    }
                }

                self.generator
                    .last_sequence
                    .write()
                    .unwrap()
                    .push((call.context.caller, (random_call.0, random_call.1.clone())));

                (call.contract, random_call)
            } else {
                self.generator.last_sequence.write().unwrap().pop().unwrap()
            };

            call.input = input.0;
            call.context.caller = sender;
            call.contract = contract;
            call.context.code_address = contract;
            call.context.address = contract;

            self.generator.used = true;
        }
    }
}
