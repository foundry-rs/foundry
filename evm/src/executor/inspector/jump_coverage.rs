use crate::{
    debug::{DebugArena, DebugNode, DebugStep, Instruction},
    executor::{
        inspector::utils::{gas_used, get_create_address},
        CHEATCODE_ADDRESS,
    },
};
use bytes::Bytes;
use ethers::types::Address;
use revm::{
    opcode, spec_opcode_gas, CallInputs, CreateInputs, Database, EVMData, Gas, Inspector,
    Interpreter, Memory, Return, SpecId,
};
use std::collections::BTreeMap;

/// An inspector that collects debug nodes on every step of the interpreter.
#[derive(Default, Debug)]
pub struct JumpRecorder {
    // mapping for each address to a mapping from JUMPDEST to whether it has been touched
    pub jump_blocks: BTreeMap<Address, BTreeMap<usize, bool>>
}

impl JumpRecorder {
    /// Builds the instruction counter map for the given bytecode.
    // TODO: Some of the same logic is performed in REVM, but then later discarded. We should
    // investigate if we can reuse it
    pub fn build_jump_map(&mut self, spec: SpecId, address: &Address, code: &Bytes) {
        let opcode_infos = spec_opcode_gas(spec);
        let mut jump_map: BTreeMap<usize, bool> = BTreeMap::new();

        let mut i = 0;
        let mut cumulative_push_size = 0;
        while i < code.len() {
            let op = code[i];
            if  op  ==  opcode::JUMPDEST as u8 {
                jump_map.insert(i, false);  
            }
            if opcode_infos[op as usize].is_push {
                // Skip the push bytes.
                //
                // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
                i += (op - opcode::PUSH1 + 1) as usize;
            }
            i += 1;
        }

        self.jump_blocks.insert(*address, jump_map);
    }
}

impl<DB> Inspector<DB> for Debugger
where
    DB: Database,
{
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.enter(data.subroutine.depth() as usize, call.contract, false);
        if call.contract == CHEATCODE_ADDRESS {
            self.arena.arena[self.head].steps.push(DebugStep {
                memory: Memory::new(),
                instruction: Instruction::Cheatcode(
                    call.input[0..4].try_into().expect("malformed cheatcode call"),
                ),
                ..Default::default()
            });
        }

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> Return {
        // TODO: This is rebuilt for all contracts every time. We should only run this if the IC
        // map for a given address does not exist, *but* we need to account for the fact that the
        // code given by the interpreter may either be the contract init code, or the runtime code.
        self.build_ic_map(
            data.env.cfg.spec_id,
            &interp.contract().address,
            &interp.contract().code,
        );
        self.previous_gas_block = interp.contract.first_gas_block();
        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        let pc = interpreter.program_counter();
        let op = interpreter.contract.code[pc];

        // Get opcode information
        let opcode_infos = spec_opcode_gas(data.env.cfg.spec_id);
        let opcode_info = &opcode_infos[op as usize];

        // Extract the push bytes
        let push_size = if opcode_info.is_push { (op - opcode::PUSH1 + 1) as usize } else { 0 };
        let push_bytes = match push_size {
            0 => None,
            n => {
                let start = pc + 1;
                let end = start + n;
                Some(interpreter.contract.code[start..end].to_vec())
            }
        };

        // Calculate the current amount of gas used
        let gas = interpreter.gas();
        let total_gas_spent = gas.spend() - self.previous_gas_block + self.current_gas_block;
        if opcode_info.gas_block_end {
            self.previous_gas_block = interpreter.contract.gas_block(pc);
            self.current_gas_block = 0;
        } else {
            self.current_gas_block += opcode_info.gas;
        }

        self.arena.arena[self.head].steps.push(DebugStep {
            pc,
            stack: interpreter.stack().data().clone(),
            memory: interpreter.memory.clone(),
            instruction: Instruction::OpCode(op),
            push_bytes,
            ic: self
                .ic_map
                .get(&interpreter.contract().address)
                .expect("no instruction counter map")
                .get(&pc)
                .copied(),
            total_gas_used: gas_used(data.env.cfg.spec_id, total_gas_spent, gas.refunded() as u64),
        });

        Return::Continue
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.exit();

        (status, gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // TODO: Does this increase gas cost?
        data.subroutine.load_account(call.caller, data.db);
        let nonce = data.subroutine.account(call.caller).info.nonce;
        self.enter(data.subroutine.depth() as usize, get_create_address(call, nonce), true);

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        self.exit();

        (status, address, gas, retdata)
    }
}
