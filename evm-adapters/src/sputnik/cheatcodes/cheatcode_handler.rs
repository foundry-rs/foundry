//! Hooks to EVM execution
use super::{
    backend::CheatcodeBackend, memory_stackstate_owned::MemoryStackStateOwned, ConsoleCalls,
    HEVMCalls,
};
use crate::{
    call_tracing::CallTrace,
    sputnik::{cheatcodes::memory_stackstate_owned::ExpectedEmit, Executor},
    Evm,
};
use std::collections::BTreeMap;

use std::{fs::File, io::Read, path::Path};

use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor, StackState, StackSubstateMetadata},
    Capture, Config, Context, CreateScheme, ExitReason, ExitSucceed, Handler, Memory, Opcode,
    Runtime, Transfer,
};
use std::{process::Command, rc::Rc};

use ethers::{
    abi::Token,
    core::{abi::AbiDecode, k256::ecdsa::SigningKey, utils},
    signers::{LocalWallet, Signer},
    solc::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, H160, U256},
};

use std::convert::Infallible;

use crate::sputnik::cheatcodes::{
    debugger::{CheatOp, DebugNode, DebugStep, OpCode},
    memory_stackstate_owned::Prank,
    patch_hardhat_console_log_selector,
};
use once_cell::sync::Lazy;

use crate::sputnik::{
    common::{ExecutionHandler, ExecutionHandlerWrapper, RuntimeExecutionHandler},
    utils::{evm_error, revert_return_evm, ExpectRevertReturn},
};
use ethers::abi::Tokenize;

// This is now getting us the right hash? Also tried [..20]
// Lazy::new(|| Address::from_slice(&keccak256("hevm cheat code")[12..]));
/// Address where the Vm cheatcodes contract lives
pub static CHEATCODE_ADDRESS: Lazy<Address> = Lazy::new(|| {
    Address::from_slice(&hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap())
});

// This is the address used by console.sol, vendored by nomiclabs/hardhat:
// https://github.com/nomiclabs/hardhat/blob/master/packages/hardhat-core/console.sol
pub static CONSOLE_ADDRESS: Lazy<Address> = Lazy::new(|| {
    Address::from_slice(&hex::decode("000000000000000000636F6e736F6c652e6c6f67").unwrap())
});

/// A [`MemoryStackStateOwned`] state instantiated over a [`CheatcodeBackend`]
pub type CheatcodeStackState<'a, B> = MemoryStackStateOwned<'a, CheatcodeBackend<B>>;

/// A [`CheatcodeHandler`] which uses a [`CheatcodeStackState`] to store its state and a
/// [`StackExecutor`] for executing transactions.
pub type CheatcodeStackExecutor<'a, 'b, B, P> =
    CheatcodeHandler<StackExecutor<'a, 'b, CheatcodeStackState<'a, B>, P>>;

/// The wrapper type that takes the `CheatcodeStackExecutor` and implements all `SputnikExecutor`
/// functions
pub type CheatcodeExecutionHandler<'a, 'b, Back, Precom> = ExecutionHandlerWrapper<
    'a,
    'b,
    Back,
    Precom,
    CheatcodeStackState<'a, Back>,
    CheatcodeStackExecutor<'a, 'b, Back, Precom>,
>;

/// Hooks on live EVM execution and forwards everything else to a Sputnik [`Handler`].
///
/// It allows:
/// 1. Logging of values for debugging
/// 2. Modifying chain state live with cheatcodes
///
/// The `call_inner` and `create_inner` functions are copy-pasted from upstream, so that
/// it can hook in the runtime. They may eventually be removed if Sputnik allows bringing in your
/// own runtime handler.
#[derive(Clone, Debug)]
// here, e.g. hardhat console.log-style, or dapptools logs, some ad-hoc method for tracing
// etc.
pub struct CheatcodeHandler<H> {
    handler: H,
    enable_ffi: bool,
    enable_trace: bool,
    console_logs: Vec<String>,
}

impl<'a, 'b, B: Backend, P: PrecompileSet + 'b>
    CheatcodeHandler<StackExecutor<'a, 'b, MemoryStackStateOwned<'a, CheatcodeBackend<B>>, P>>
{
}

impl<'a, 'b, Back: Backend, Pre: PrecompileSet + 'b> CheatcodeStackExecutor<'a, 'b, Back, Pre> {
    /// This allows has to turn this mutable ref into a type that implements `sputnik::Handler` to
    /// execute runtime operations
    fn as_handler<'handler>(
        &'handler mut self,
    ) -> RuntimeExecutionHandler<
        'handler,
        'a,
        'b,
        Back,
        Pre,
        CheatcodeStackState<'a, Back>,
        Self,
        CheatcodeBackend<Back>,
    > {
        RuntimeExecutionHandler::new(self)
    }

    fn state(&self) -> &CheatcodeStackState<'a, Back> {
        self.handler.state()
    }

    fn state_mut(&mut self) -> &mut CheatcodeStackState<'a, Back> {
        self.handler.state_mut()
    }

    /// Checks whether the provided call reverted with an expected revert reason.
    fn expected_revert(
        &mut self,
        res: ExpectRevertReturn,
        expected_revert: Option<Vec<u8>>,
    ) -> ExpectRevertReturn {
        // return early if there was no revert expected
        let expected_revert = match expected_revert {
            Some(inner) => inner,
            None => return res,
        };

        let call = res.is_call();

        // If the call was successful (i.e. did not revert) return
        // an error. Otherwise, get the return data
        let data = match res {
            ExpectRevertReturn::Create(Capture::Exit((ExitReason::Revert(_e), None, revdata))) => {
                Some(revdata)
            }
            ExpectRevertReturn::Call(Capture::Exit((ExitReason::Revert(_e), revdata))) => {
                Some(revdata)
            }
            _ => return revert_return_evm(call, None, || "Expected revert did not revert"),
        };

        // if there was no revert data return an error
        let data = match data {
            Some(inner) => inner,
            None => {
                return revert_return_evm(call, None, || "Expected revert did not revert with data")
            }
        };

        // do the actual check
        if data.len() >= 4 && data[0..4] == [8, 195, 121, 160] {
            // its a revert string
            let decoded_data = ethers::abi::decode(&[ethers::abi::ParamType::Bytes], &data[4..])
                .expect("String error code, but not actual string");

            let decoded_data =
                decoded_data[0].clone().into_bytes().expect("Can never fail because it is bytes");

            let err = || {
                format!(
                    "Error != expected error: '{}' != '{}'",
                    String::from_utf8_lossy(&decoded_data[..]),
                    String::from_utf8_lossy(&expected_revert)
                )
            };
            revert_return_evm(call, Some((&decoded_data, &expected_revert)), err)
        } else {
            let err = || {
                format!(
                    "Error data != expected error data: 0x{} != 0x{}",
                    hex::encode(&data),
                    hex::encode(&expected_revert)
                )
            };
            revert_return_evm(call, Some((&data, &expected_revert)), err)
        }
    }

    /// Given a transaction's calldata, it tries to parse it a console call and print the call
    fn console_log(&mut self, input: Vec<u8>) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        // replacing hardhat style selectors (`uint`) with abigen style (`uint256`)
        let input = patch_hardhat_console_log_selector(input);
        let decoded = match ConsoleCalls::decode(&input) {
            Ok(inner) => inner,
            Err(err) => return evm_error(&err.to_string()),
        };
        self.console_logs.push(decoded.to_string());
        Capture::Exit((ExitReason::Succeed(ExitSucceed::Stopped), vec![]))
    }

    /// Adds CheatOp to the latest DebugArena
    fn add_debug(&mut self, cheatop: CheatOp) {
        if self.state().debug_enabled {
            let depth =
                if let Some(depth) = self.state().metadata().depth() { depth + 1 } else { 0 };
            self.state_mut().debug_mut().push_node(
                0,
                DebugNode {
                    address: *CHEATCODE_ADDRESS,
                    depth,
                    steps: vec![DebugStep {
                        op: OpCode::from(cheatop),
                        memory: Memory::new(0),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
        }
    }

    fn prank(
        &mut self,
        single_call: bool,
        msg_sender: Address,
        caller: Address,
        origin: Option<Address>,
    ) -> Result<(), Capture<(ExitReason, Vec<u8>), Infallible>> {
        let curr_depth =
            if let Some(depth) = self.state().metadata().depth() { depth + 1 } else { 0 };

        let prank = Prank {
            prank_caller: msg_sender,
            new_caller: caller,
            new_origin: origin,
            depth: curr_depth,
        };
        if single_call {
            if self.state().next_prank.is_some() {
                return Err(evm_error("You have an active `prank` call already. Use either `prank` or `startPrank`, not both"));
            }
            self.state_mut().next_prank = Some(prank);
        } else {
            // startPrank works by using frame depth to determine whether to overwrite
            // msg.sender if we set a prank caller at a particular depth, it
            // will continue to use the prank caller for any subsequent calls
            // until stopPrank is called.
            //
            // We additionally have to store the original message sender of the cheatcode caller
            // so that we dont apply it to any other addresses when depth ==
            // prank_depth
            if let Some(Prank { depth, prank_caller, .. }) = self.state().prank {
                if curr_depth == depth && caller == prank_caller {
                    return Err(evm_error("You have an active `startPrank` at this frame depth already. Use either `prank` or `startPrank`, not both"));
                }
            }
            self.state_mut().prank = Some(prank);
        }
        Ok(())
    }

    fn expect_revert(
        &mut self,
        inner: Vec<u8>,
    ) -> Result<(), Capture<(ExitReason, Vec<u8>), Infallible>> {
        self.add_debug(CheatOp::EXPECTREVERT);
        if self.state().expected_revert.is_some() {
            return Err(evm_error(
                "You must call another function prior to expecting a second revert.",
            ))
        } else {
            self.state_mut().expected_revert = Some(inner);
        }
        Ok(())
    }

    /// Given a transaction's calldata, it tries to parse it as an [`HEVM cheatcode`](super::HEVM)
    /// call and modify the state accordingly.
    fn apply_cheatcode(
        &mut self,
        input: Vec<u8>,
        msg_sender: H160,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        let mut res = vec![];
        let pre_index = self.state().trace_index;
        let trace = self.start_trace(*CHEATCODE_ADDRESS, input.clone(), 0.into(), false);
        // Get a mutable ref to the state so we can apply the cheats
        let decoded = match HEVMCalls::decode(&input) {
            Ok(inner) => inner,
            Err(err) => return evm_error(&err.to_string()),
        };

        match decoded {
            HEVMCalls::Warp(inner) => {
                self.add_debug(CheatOp::WARP);
                self.state_mut().backend.cheats.block_timestamp = Some(inner.0);
            }
            HEVMCalls::Roll(inner) => {
                self.add_debug(CheatOp::ROLL);
                self.state_mut().backend.cheats.block_number = Some(inner.0);
            }
            HEVMCalls::Fee(inner) => {
                self.add_debug(CheatOp::FEE);
                self.state_mut().backend.cheats.block_base_fee_per_gas = Some(inner.0);
            }
            HEVMCalls::Store(inner) => {
                self.add_debug(CheatOp::STORE);
                self.state_mut().set_storage(inner.0, inner.1.into(), inner.2.into());
            }
            HEVMCalls::Load(inner) => {
                self.add_debug(CheatOp::LOAD);
                res = self.state_mut().storage(inner.0, inner.1.into()).0.to_vec();
            }
            HEVMCalls::Ffi(inner) => {
                self.add_debug(CheatOp::FFI);
                let args = inner.0;
                // if FFI is not explicitly enabled at runtime, do not let this be called
                // (we could have an FFI cheatcode executor instead but feels like
                // over engineering)
                if !self.enable_ffi {
                    return evm_error(
                        "ffi disabled: run again with --ffi if you want to allow tests to call external scripts",
                    );
                }

                // execute the command & get the stdout
                let output = match Command::new(&args[0]).args(&args[1..]).output() {
                    Ok(res) => res.stdout,
                    Err(err) => return evm_error(&err.to_string()),
                };

                // get the hex string & decode it
                let output = unsafe { std::str::from_utf8_unchecked(&output) };
                let decoded = match hex::decode(&output.trim()[2..]) {
                    Ok(res) => res,
                    Err(err) => return evm_error(&err.to_string()),
                };

                // encode the data as Bytes
                res = ethers::abi::encode(&[Token::Bytes(decoded.to_vec())]);
            }
            HEVMCalls::GetCode(inner) => {
                self.add_debug(CheatOp::GETCODE);

                let path = if inner.0.ends_with(".json") {
                    Path::new(&inner.0).to_path_buf()
                } else {
                    let parts = inner.0.split(':').collect::<Vec<&str>>();
                    let contract_file = parts[0];
                    let contract_name = if parts.len() == 1 {
                        parts[0].replace(".sol", "")
                    } else {
                        parts[1].to_string()
                    };

                    let outdir = ProjectPathsConfig::find_artifacts_dir(Path::new("./"));
                    outdir.join(format!("{}/{}.json", contract_file, contract_name))
                };

                let mut data = String::new();
                match File::open(path) {
                    Ok(mut file) => match file.read_to_string(&mut data) {
                        Ok(_) => {}
                        Err(e) => return evm_error(&e.to_string()),
                    },
                    Err(e) => return evm_error(&e.to_string()),
                }

                match serde_json::from_str::<CompactContractBytecode>(&data) {
                    Ok(contract_file) => {
                        if let Some(bin) =
                            contract_file.bytecode.and_then(|bcode| bcode.object.into_bytes())
                        {
                            res = ethers::abi::encode(&[Token::Bytes(bin.to_vec())]);
                        } else {
                            return evm_error(
                                "No bytecode for contract. is it abstract or unlinked?",
                            )
                        }
                    }
                    Err(e) => return evm_error(&e.to_string()),
                }
            }
            HEVMCalls::Addr(inner) => {
                self.add_debug(CheatOp::ADDR);
                let sk = inner.0;
                if sk.is_zero() {
                    return evm_error("Bad Cheat Code. Private Key cannot be 0.")
                }
                // 256 bit priv key -> 32 byte slice
                let mut bs: [u8; 32] = [0; 32];
                sk.to_big_endian(&mut bs);
                let xsk = match SigningKey::from_bytes(&bs) {
                    Ok(xsk) => xsk,
                    Err(err) => return evm_error(&err.to_string()),
                };
                let addr = utils::secret_key_to_address(&xsk);
                res = ethers::abi::encode(&[Token::Address(addr)]);
            }
            HEVMCalls::Sign(inner) => {
                self.add_debug(CheatOp::SIGN);
                let sk = inner.0;
                let digest = inner.1;
                if sk.is_zero() {
                    return evm_error("Bad Cheat Code. Private Key cannot be 0.")
                }
                // 256 bit priv key -> 32 byte slice
                let mut bs: [u8; 32] = [0; 32];
                sk.to_big_endian(&mut bs);

                let xsk = match SigningKey::from_bytes(&bs) {
                    Ok(xsk) => xsk,
                    Err(err) => return evm_error(&err.to_string()),
                };
                let wallet = LocalWallet::from(xsk).with_chain_id(self.handler.chain_id().as_u64());

                // The EVM precompile does not use EIP-155
                let sig = wallet.sign_hash(digest.into(), false);

                let recovered = match sig.recover(digest) {
                    Ok(rec) => rec,
                    Err(e) => return evm_error(&e.to_string()),
                };

                assert_eq!(recovered, wallet.address());

                let mut r_bytes = [0u8; 32];
                let mut s_bytes = [0u8; 32];
                sig.r.to_big_endian(&mut r_bytes);
                sig.s.to_big_endian(&mut s_bytes);
                res = ethers::abi::encode(&[Token::Tuple(vec![
                    Token::Uint(sig.v.into()),
                    Token::FixedBytes(r_bytes.to_vec()),
                    Token::FixedBytes(s_bytes.to_vec()),
                ])]);
            }
            HEVMCalls::Prank0(inner) => {
                self.add_debug(CheatOp::PRANK);
                let caller = inner.0;
                if let Err(err) = self.prank(true, msg_sender, caller, None) {
                    return err
                }
            }
            HEVMCalls::StartPrank0(inner) => {
                self.add_debug(CheatOp::STARTPRANK);
                let caller = inner.0;
                if let Err(err) = self.prank(false, msg_sender, caller, None) {
                    return err
                }
            }
            HEVMCalls::Prank1(inner) => {
                self.add_debug(CheatOp::PRANK);
                let caller = inner.0;
                let origin = inner.1;
                if let Err(err) = self.prank(true, msg_sender, caller, Some(origin)) {
                    return err
                }
            }
            HEVMCalls::StartPrank1(inner) => {
                self.add_debug(CheatOp::STARTPRANK);
                let caller = inner.0;
                let origin = inner.1;
                if let Err(err) = self.prank(false, msg_sender, caller, Some(origin)) {
                    return err
                }
            }
            HEVMCalls::StopPrank(_) => {
                self.add_debug(CheatOp::STOPPRANK);
                self.state_mut().prank = None;
            }
            HEVMCalls::ExpectRevert0(inner) => {
                if let Err(e) = self.expect_revert(inner.0.to_vec()) {
                    return e
                }
            }
            HEVMCalls::ExpectRevert1(inner) => {
                if let Err(e) = self.expect_revert(inner.0.to_vec()) {
                    return e
                }
            }
            HEVMCalls::Deal(inner) => {
                self.add_debug(CheatOp::DEAL);
                let who = inner.0;
                let value = inner.1;
                self.state_mut().reset_balance(who);
                self.state_mut().deposit(who, value);
            }
            HEVMCalls::Etch(inner) => {
                self.add_debug(CheatOp::ETCH);
                let who = inner.0;
                let code = inner.1;
                self.state_mut().set_code(who, code.to_vec());
            }
            HEVMCalls::Record(_) => {
                self.add_debug(CheatOp::RECORD);
                self.state_mut().accesses = Some(Default::default());
            }
            HEVMCalls::Accesses(inner) => {
                self.add_debug(CheatOp::ACCESSES);
                let address = inner.0;
                // we dont reset all records in case user wants to query multiple address
                if let Some(record_accesses) = &self.state().accesses {
                    res = ethers::abi::encode(&[
                        record_accesses
                            .reads
                            .borrow_mut()
                            .remove(&address)
                            .unwrap_or_default()
                            .into_tokens()[0]
                            .clone(),
                        record_accesses
                            .writes
                            .borrow_mut()
                            .remove(&address)
                            .unwrap_or_default()
                            .into_tokens()[0]
                            .clone(),
                    ]);
                    if record_accesses.reads.borrow().len() == 0 &&
                        record_accesses.writes.borrow().len() == 0
                    {
                        self.state_mut().accesses = None;
                    }
                } else {
                    res = ethers::abi::encode(&[Token::Array(vec![]), Token::Array(vec![])]);
                }
            }
            HEVMCalls::ExpectEmit(inner) => {
                self.add_debug(CheatOp::EXPECTEMIT);
                let expected_emit = ExpectedEmit {
                    depth: if let Some(depth) = self.state().metadata().depth() {
                        depth + 1
                    } else {
                        0
                    },
                    log: None,
                    checks: [inner.0, inner.1, inner.2, inner.3],
                    found: false,
                };
                self.state_mut().expected_emits.push(expected_emit);
            }
            HEVMCalls::MockCall(inner) => {
                self.add_debug(CheatOp::MOCKCALL);
                self.state_mut()
                    .mocked_calls
                    .entry(inner.0)
                    .or_default()
                    .insert(inner.1.to_vec(), inner.2.to_vec());
            }
            HEVMCalls::ClearMockedCalls(_) => {
                self.add_debug(CheatOp::CLEARMOCKEDCALLS);
                self.state_mut().mocked_calls = Default::default();
            }
            HEVMCalls::ExpectCall(inner) => {
                self.add_debug(CheatOp::EXPECTCALL);
                self.state_mut().expected_calls.entry(inner.0).or_default().push(inner.1.to_vec());
            }
            HEVMCalls::Label(inner) => {
                self.add_debug(CheatOp::LABEL);
                let address = inner.0;
                let label = inner.1;

                self.state_mut().labels.insert(address, label);
            }
        };

        self.fill_trace(&trace, true, Some(res.clone()), pre_index);
        // cheatcodes should cost 0 gas
        if let Some(new_trace) = &trace {
            let trace = &mut self.state_mut().trace_mut().arena[new_trace.idx].trace;
            trace.cost = 0;
        }
        // TODO: Add more cheat codes.
        Capture::Exit((ExitReason::Succeed(ExitSucceed::Stopped), res))
    }

    /// Executes the call/create while also tracking the state of the machine (including opcodes)
    fn do_debug_execute(
        &mut self,
        runtime: &mut Runtime,
        address: Address,
        code: Rc<Vec<u8>>,
        creation: bool,
    ) -> ExitReason {
        let depth = if let Some(depth) = self.state().metadata().depth() { depth + 1 } else { 0 };

        match self.debug_run(runtime, address, depth, code, creation) {
            Capture::Exit(s) => s,
            Capture::Trap(_) => unreachable!("Trap is Infallible"),
        }
    }

    /// Does *not* actually perform a step, just records the debug information for the step
    fn debug_step(
        &mut self,
        runtime: &mut Runtime,
        code: Rc<Vec<u8>>,
        steps: &mut Vec<DebugStep>,
        pc_ic: Rc<BTreeMap<usize, usize>>,
    ) -> bool {
        // grab the pc, opcode and stack
        let pc = runtime.machine().position().as_ref().map(|p| *p).unwrap_or_default();
        let mut push_bytes = None;

        if let Some((op, stack)) = runtime.machine().inspect() {
            // wrap the op to make it compatible with opcode extensions for cheatops
            let wrapped_op = OpCode::from(op);

            // check how big the push size is, and grab the pushed bytes if possible
            if let Some(push_size) = wrapped_op.push_size() {
                let push_start = pc + 1;
                let push_end = pc + 1 + push_size as usize;
                if push_end < code.len() {
                    push_bytes = Some(code[push_start..push_end].to_vec());
                } else {
                    panic!("PUSH{} exceeds limit of codesize", push_size)
                }
            }

            // grab the stack data and reverse it (last element is "top" of stack)
            let mut stack = stack.data().clone();
            stack.reverse();
            // push the step into the vector
            steps.push(DebugStep {
                pc,
                stack,
                memory: runtime.machine().memory().clone(),
                op: wrapped_op,
                push_bytes,
                ic: *pc_ic.get(&pc).as_ref().copied().unwrap_or(&0usize),
                total_gas_used: self.handler.used_gas(),
            });
            match op {
                Opcode::CREATE |
                Opcode::CREATE2 |
                Opcode::CALL |
                Opcode::CALLCODE |
                Opcode::DELEGATECALL |
                Opcode::STATICCALL => {
                    // this would create an interrupt, have `debug_run` construct a new vec
                    // to commit the current vector of steps into the debugarena
                    // this maintains the call hierarchy correctly
                    true
                }
                _ => false,
            }
        } else {
            // failure case.
            let mut stack = runtime.machine().stack().data().clone();
            stack.reverse();
            steps.push(DebugStep {
                pc,
                stack,
                memory: runtime.machine().memory().clone(),
                op: OpCode::from(Opcode::INVALID),
                push_bytes,
                ic: *pc_ic.get(&pc).as_ref().copied().unwrap_or(&0usize),
                total_gas_used: self.handler.used_gas(),
            });
            true
        }
    }

    fn debug_run(
        &mut self,
        runtime: &mut Runtime,
        address: Address,
        depth: usize,
        code: Rc<Vec<u8>>,
        creation: bool,
    ) -> Capture<ExitReason, ()> {
        let mut done = false;
        let mut res = Capture::Exit(ExitReason::Succeed(ExitSucceed::Returned));
        let mut steps = Vec::new();
        // grab the debug instruction pointers for either construct or runtime bytecode
        let dip = if creation {
            &mut self.state_mut().debug_instruction_pointers.0
        } else {
            &mut self.state_mut().debug_instruction_pointers.1
        };
        // get the program counter => instruction counter mapping from memory or construct it
        let ics = if let Some(pc_ic) = dip.get(&address) {
            // grabs an Rc<BTreemap> of an already created pc -> ic mapping
            pc_ic.clone()
        } else {
            // builds a program counter to instruction counter map
            // basically this strips away bytecodes to make it work
            // with the sourcemap output from the solc compiler
            let mut pc_ic: BTreeMap<usize, usize> = BTreeMap::new();

            let mut i = 0;
            let mut push_ctr = 0usize;
            while i < code.len() {
                let wrapped_op = OpCode::from(Opcode(code[i]));
                pc_ic.insert(i, i - push_ctr);

                if let Some(push_size) = wrapped_op.push_size() {
                    i += push_size as usize;
                    i += 1;
                    push_ctr += push_size as usize;
                } else {
                    i += 1;
                }
            }
            let pc_ic = Rc::new(pc_ic);

            dip.insert(address, pc_ic.clone());
            pc_ic
        };
        while !done {
            // debug step doesnt actually execute the step, it just peeks into the machine
            // will return true or false, which signifies whether to push the steps
            // as a node and reset the steps vector or not
            if self.debug_step(runtime, code.clone(), &mut steps, ics.clone()) && !steps.is_empty()
            {
                self.state_mut().debug_mut().push_node(
                    0,
                    DebugNode {
                        address,
                        depth,
                        steps: steps.clone(),
                        creation,
                        ..Default::default()
                    },
                );
                steps = Vec::new();
            }

            let mut dbg_node = None;

            // actually executes the opcode step
            let r = runtime.step(&mut self.as_handler());
            match r {
                Ok(()) => {}
                Err(e) => {
                    done = true;
                    // we wont hit an interrupt when we finish stepping
                    // so we have add the accumulated steps as if debug_step returned true
                    if !steps.is_empty() {
                        dbg_node = Some(DebugNode {
                            address,
                            depth,
                            steps: steps.clone(),
                            creation,
                            ..Default::default()
                        });
                    }
                    match e {
                        Capture::Exit(s) => res = Capture::Exit(s),
                        Capture::Trap(_) => unreachable!("Trap is Infallible"),
                    }
                }
            }

            // need to push the node here because of mutable borrow
            if let Some(node) = dbg_node {
                self.state_mut().debug_mut().push_node(0, node);
            }
        }

        res
    }

    fn start_trace(
        &mut self,
        address: H160,
        input: Vec<u8>,
        transfer: U256,
        creation: bool,
    ) -> Option<CallTrace> {
        if self.enable_trace {
            let mut trace: CallTrace = CallTrace {
                // depth only starts tracking at first child substate and is 0. so add 1 when depth
                // is some.
                depth: if let Some(depth) = self.state().metadata().depth() {
                    depth + 1
                } else {
                    0
                },
                addr: address,
                created: creation,
                data: input,
                value: transfer,
                label: self.state().labels.get(&address).cloned(),
                ..Default::default()
            };

            self.state_mut().trace_mut().push_trace(0, &mut trace);
            self.state_mut().trace_index = trace.idx;
            Some(trace)
        } else {
            None
        }
    }
}

impl<'a, 'b, Back, Precom: 'b> ExecutionHandler<'a, 'b, Back, Precom, CheatcodeStackState<'a, Back>>
    for CheatcodeStackExecutor<'a, 'b, Back, Precom>
where
    Back: Backend,
    Precom: PrecompileSet,
{
    fn stack_executor(&self) -> &StackExecutor<'a, 'b, CheatcodeStackState<'a, Back>, Precom> {
        &self.handler
    }

    fn stack_executor_mut(
        &mut self,
    ) -> &mut StackExecutor<'a, 'b, CheatcodeStackState<'a, Back>, Precom> {
        &mut self.handler
    }

    fn on_clear_logs(&mut self) {
        self.console_logs.clear();
    }

    fn additional_logs(&self) -> Vec<String> {
        self.console_logs.clone()
    }

    fn is_tracing_enabled(&self) -> bool {
        self.enable_trace
    }

    fn debug_execute(
        &mut self,
        runtime: &mut Runtime,
        address: Address,
        code: Rc<Vec<u8>>,
        creation: bool,
    ) -> ExitReason {
        self.do_debug_execute(runtime, address, code, creation)
    }

    fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    ) {
        self.state_mut().trace_index = pre_trace_index;
        if let Some(new_trace) = new_trace {
            let used_gas = self.stack_executor().used_gas();
            let trace = &mut self.state_mut().trace_mut().arena[new_trace.idx].trace;
            trace.output = output.unwrap_or_default();
            trace.cost = used_gas;
            trace.success = success;
        }
    }

    fn do_create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        // modify execution context depending on the cheatcode

        let prev_origin = self.state().backend.cheats.origin;
        let expected_revert = self.state_mut().expected_revert.take();
        let mut new_tx_caller = caller;
        let mut new_scheme = scheme;
        let curr_depth =
            if let Some(depth) = self.state().metadata().depth() { depth + 1 } else { 0 };

        // handle `startPrank` - see apply_cheatcodes for more info
        if let Some(Prank { prank_caller, new_caller, new_origin, depth }) = self.state().prank {
            if curr_depth == depth && new_tx_caller == prank_caller {
                new_tx_caller = new_caller;

                self.state_mut().backend.cheats.origin = new_origin
            }
        }

        // handle normal `prank`
        if let Some(Prank { new_caller, new_origin, .. }) = self.state_mut().next_prank.take() {
            new_tx_caller = new_caller;

            self.state_mut().backend.cheats.origin = new_origin
        }

        if caller != new_tx_caller {
            new_scheme = match scheme {
                CreateScheme::Legacy { .. } => CreateScheme::Legacy { caller: new_tx_caller },
                CreateScheme::Create2 { code_hash, salt, .. } => {
                    CreateScheme::Create2 { caller: new_tx_caller, code_hash, salt }
                }
                _ => scheme,
            };
        }

        let res = self.as_handler().create_inner(
            new_tx_caller,
            new_scheme,
            value,
            init_code,
            target_gas,
            true,
        );

        // if we set the origin, now we should reset to prior origin
        self.state_mut().backend.cheats.origin = prev_origin;

        if !self.state_mut().expected_emits.is_empty() &&
            !self
                .state()
                .expected_emits
                .iter()
                .filter(|expected| expected.depth == curr_depth)
                .all(|expected| expected.found)
        {
            return revert_return_evm(false, None, || "Log != expected log").into_create_inner()
        } else {
            // empty out expected_emits after successfully capturing all of them
            self.state_mut().expected_emits = Vec::new();
        }

        self.expected_revert(ExpectRevertReturn::Create(res), expected_revert).into_create_inner()
    }

    fn do_call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        // We intercept calls to the `CHEATCODE_ADDRESS` to apply the cheatcode directly
        // to the state.
        // NB: This is very similar to how Optimism's custom intercept logic to "predeploys" work
        // (e.g. with the StateManager)
        if code_address == *CHEATCODE_ADDRESS {
            self.apply_cheatcode(input, context.caller)
        } else if code_address == *CONSOLE_ADDRESS {
            self.console_log(input)
        } else {
            // record prior origin
            let prev_origin = self.state().backend.cheats.origin;

            // modify execution context depending on the cheatcode
            let expected_revert = self.state_mut().expected_revert.take();
            let mut new_context = context;
            let mut new_transfer = transfer;
            let curr_depth =
                if let Some(depth) = self.state().metadata().depth() { depth + 1 } else { 0 };

            // handle `startPrank` - see apply_cheatcodes for more info
            if let Some(Prank { prank_caller, new_caller, new_origin, depth }) = self.state().prank
            {
                // if depth and msg.sender match, perform the prank
                if curr_depth == depth && new_context.caller == prank_caller {
                    new_context.caller = new_caller;

                    if let Some(t) = &new_transfer {
                        new_transfer =
                            Some(Transfer { source: new_caller, target: t.target, value: t.value });
                    }

                    // set the origin if the user used the overloaded func
                    self.state_mut().backend.cheats.origin = new_origin;
                }
            }

            // handle normal `prank`
            if let Some(Prank { new_caller, new_origin, .. }) = self.state_mut().next_prank.take() {
                new_context.caller = new_caller;

                if let Some(t) = &new_transfer {
                    new_transfer =
                        Some(Transfer { source: new_caller, target: t.target, value: t.value });
                }

                self.state_mut().backend.cheats.origin = new_origin;
            }

            // handle expected calls
            if let Some(expecteds) = self.state_mut().expected_calls.get_mut(&code_address) {
                if let Some(found_match) = expecteds.iter().position(|expected| {
                    expected.len() <= input.len() && expected == &input[..expected.len()]
                }) {
                    expecteds.remove(found_match);
                }
            }

            // handle mocked calls
            if let Some(mocks) = self.state().mocked_calls.get(&code_address) {
                if let Some(mock_retdata) = mocks.get(&input) {
                    return Capture::Exit((
                        ExitReason::Succeed(ExitSucceed::Returned),
                        mock_retdata.clone(),
                    ))
                } else if let Some((_, mock_retdata)) =
                    mocks.iter().find(|(mock, _)| *mock == &input[..mock.len()])
                {
                    return Capture::Exit((
                        ExitReason::Succeed(ExitSucceed::Returned),
                        mock_retdata.clone(),
                    ))
                }
            }

            // perform the call
            let res = self.as_handler().call_inner(
                code_address,
                new_transfer,
                input,
                target_gas,
                is_static,
                true,
                true,
                new_context,
            );

            // if we set the origin, now we should reset to previous
            self.state_mut().backend.cheats.origin = prev_origin;

            // handle expected emits
            if !self.state_mut().expected_emits.is_empty() &&
                !self
                    .state()
                    .expected_emits
                    .iter()
                    .filter(|expected| expected.depth == curr_depth)
                    .all(|expected| expected.found)
            {
                return evm_error("Log != expected log")
            } else {
                // empty out expected_emits after successfully capturing all of them
                self.state_mut().expected_emits = Vec::new();
            }

            self.expected_revert(ExpectRevertReturn::Call(res), expected_revert).into_call_inner()
        }
    }
}

impl<'a, 'b, B: Backend, P: PrecompileSet + 'b>
    Executor<CheatcodeStackState<'a, B>, CheatcodeExecutionHandler<'a, 'b, B, P>>
{
    /// Instantiates a cheatcode-enabled [`Executor`]
    pub fn new_with_cheatcodes(
        backend: B,
        gas_limit: u64,
        config: &'a Config,
        precompiles: &'b P,
        enable_ffi: bool,
        enable_trace: bool,
        debug: bool,
    ) -> Self {
        // make this a cheatcode-enabled backend
        let backend = CheatcodeBackend { backend, cheats: Default::default() };

        // create the memory stack state (owned, so that we can modify the backend via
        // self.state_mut on the transact_call fn)
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        let state = MemoryStackStateOwned::new(metadata, backend, enable_trace, debug);

        // create the executor and wrap it with the cheatcode handler
        let executor = StackExecutor::new_with_precompiles(state, config, precompiles);
        let executor = CheatcodeHandler {
            handler: executor,
            enable_ffi,
            enable_trace,
            console_logs: Vec::new(),
        };

        let executor = CheatcodeExecutionHandler::new(executor);

        let mut evm = Executor::from_executor(executor, gas_limit);

        // Need to create a non-empty contract at the cheat code address so that the EVM backend
        // thinks that something exists there.
        evm.initialize_contracts([
            (*CHEATCODE_ADDRESS, vec![0u8; 1].into()),
            (*CONSOLE_ADDRESS, vec![0u8; 1].into()),
        ]);

        evm
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        call_tracing::ExecutionInfo,
        fuzz::FuzzedExecutor,
        sputnik::helpers::{vm, vm_no_limit, vm_tracing},
        test_helpers::COMPILED,
        Evm,
    };

    use super::*;

    #[test]
    fn ds_test_logs() {
        let mut evm = vm();
        let compiled = COMPILED.find("DebugLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) = evm
            .call::<(), _, _>(Address::zero(), addr, "test_log()", (), 0.into(), compiled.abi)
            .unwrap();
        let expected = [
            "Hi",
            "0x1234",
            "0x1111111111111111111111111111111111111111",
            "0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d",
            "123",
            "1234",
            "0x4567",
            "lol",
            "addr: 0x2222222222222222222222222222222222222222",
            "key: 0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d",
            "key: 0.000000000000000123",
            "key: -0.000000000000000123",
            "key: 1.000000000000000000",
            "key: -1.000000000000000000",
            "key: -0.000000000123",
            "key: -1000000.000000000000",
            "key: 0.000000000000001234",
            "key: 1.000000000000000000",
            "key: 0.000000001234",
            "key: 1000000.000000000000",
            "key: 123",
            "key: 1234",
            "key: 0x4567",
            "key: lol",
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        assert_eq!(logs, expected);
    }

    #[test]
    fn console_logs() {
        let mut evm = vm();

        let compiled = COMPILED.find("ConsoleLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) = evm
            .call::<(), _, _>(Address::zero(), addr, "test_log()", (), 0.into(), compiled.abi)
            .unwrap();
        let expected = [
            "0x1111111111111111111111111111111111111111",
            "Hi",
            "Hi, Hi",
            "1337",
            "1337, 1245",
            "Hi, 1337",
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        assert_eq!(logs, expected);

        let (_, _, _, logs) = evm
            .call::<(), _, _>(Address::zero(), addr, "test_log()", (), 0.into(), compiled.abi)
            .unwrap();
        assert_eq!(logs, expected);
    }

    #[test]
    fn console_logs_types() {
        let mut evm = vm();

        let compiled = COMPILED.find("ConsoleLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) = evm
            .call::<(), _, _>(Address::zero(), addr, "test_log_types()", (), 0.into(), compiled.abi)
            .unwrap();
        let expected =
            ["String", "1337", "-20", "1245", "true", "0x1111111111111111111111111111111111111111"]
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
        assert_eq!(logs, expected);
    }

    #[test]
    fn console_logs_types_bytes() {
        let mut evm = vm();

        let compiled = COMPILED.find("ConsoleLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) = evm
            .call::<(), _, _>(
                Address::zero(),
                addr,
                "test_log_types_bytes()",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();
        let expected = [
            r#"Bytes(b"logBytes")"#,
            r#"Bytes(b"\xfb\xa3\xa4\xb5")"#,
            "0xfb",
            "0xfba3",
            "0xfba3a4",
            "0xfba3a4b5",
            "0xfba3a4b500",
            "0xfba3a4b50000",
            "0xfba3a4b5000000",
            "0xfba3a4b500000000",
            "0xfba3a4b50000000000",
            "0xfba3a4b5000000000000",
            "0xfba3a4b500000000000000",
            "0xfba3a4b50000000000000000",
            "0xfba3a4b5000000000000000000",
            "0xfba3a4b500000000000000000000",
            "0xfba3a4b50000000000000000000000",
            "0xfba3a4b5000000000000000000000000",
            "0xfba3a4b500000000000000000000000000",
            "0xfba3a4b50000000000000000000000000000",
            "0xfba3a4b5000000000000000000000000000000",
            "0xfba3a4b500000000000000000000000000000000",
            "0xfba3a4b50000000000000000000000000000000000",
            "0xfba3a4b5000000000000000000000000000000000000",
            "0xfba3a4b500000000000000000000000000000000000000",
            "0xfba3a4b50000000000000000000000000000000000000000",
            "0xfba3a4b5000000000000000000000000000000000000000000",
            "0xfba3a4b500000000000000000000000000000000000000000000",
            "0xfba3a4b50000000000000000000000000000000000000000000000",
            "0xfba3a4b5000000000000000000000000000000000000000000000000",
            "0xfba3a4b500000000000000000000000000000000000000000000000000",
            "0xfba3a4b50000000000000000000000000000000000000000000000000000",
            "0xfba3a4b5000000000000000000000000000000000000000000000000000000",
            "0xfba3a4b500000000000000000000000000000000000000000000000000000000",
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        assert_eq!(logs, expected);
    }

    #[test]
    fn logs_external_contract() {
        let mut evm = vm();

        let compiled = COMPILED.find("DebugLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) = evm
            .call::<(), _, _>(
                Address::zero(),
                addr,
                "test_log_elsewhere()",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();
        let expected = ["0x1111111111111111111111111111111111111111", "Hi"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(logs, expected);
    }

    #[test]
    fn cheatcodes() {
        let mut evm = vm_no_limit();
        let compiled = COMPILED.find("CheatCodes").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        let state = evm.state().clone();
        let mut cfg = proptest::test_runner::Config::default();
        cfg.failure_persistence = None;
        let runner = proptest::test_runner::TestRunner::new(cfg);

        // ensure the storage slot is set at 10 anyway
        let (storage_contract, _, _, _) = evm
            .call::<Address, _, _>(
                Address::zero(),
                addr,
                "store()(address)",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();
        let (slot, _, _, _) = evm
            .call::<U256, _, _>(
                Address::zero(),
                storage_contract,
                "slot0()(uint256)",
                (),
                0.into(),
                compiled.abi,
            )
            .unwrap();
        assert_eq!(slot, 10.into());

        let evm = FuzzedExecutor::new(&mut evm, runner, Address::zero());

        let abi = compiled.abi.as_ref().unwrap();
        for func in abi.functions().filter(|func| func.name.starts_with("test")) {
            // Skip the FFI unit test if not in a unix system
            if func.name == "testFFI" && !cfg!(unix) {
                continue
            }

            let should_fail = func.name.starts_with("testFail");
            if func.inputs.is_empty() {
                let (_, reason, _, _) =
                    evm.as_mut().call_unchecked(Address::zero(), addr, func, (), 0.into()).unwrap();
                assert!(evm.as_mut().check_success(addr, &reason, should_fail));
            } else {
                assert!(evm.fuzz(func, addr, should_fail, Some(abi)).is_ok());
            }

            evm.as_mut().reset(state.clone());
        }
    }

    #[test]
    fn ffi_fails_if_disabled() {
        let mut evm = vm_no_limit();
        evm.executor.handler_mut().enable_ffi = false;

        let compiled = COMPILED.find("CheatCodes").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0u64.into()).unwrap();

        let err = evm
            .call::<(), _, _>(Address::zero(), addr, "testFFI()", (), 0.into(), compiled.abi)
            .unwrap_err();
        let reason = match err {
            crate::EvmError::Execution { reason, .. } => reason,
            _ => panic!("unexpected error"),
        };
        assert_eq!(reason, "ffi disabled: run again with --ffi if you want to allow tests to call external scripts");
    }

    #[test]
    fn tracing_call() {
        use std::collections::BTreeMap;
        let mut evm = vm_tracing(false);

        let compiled = COMPILED.find("Trace").expect("could not find contract");
        let (addr, _, _, _) = evm
            .deploy(
                Address::zero(),
                compiled.bin.unwrap().clone().into_bytes().expect("shouldn't be linked"),
                0.into(),
            )
            .unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, _) = evm
            .call::<(), _, _>(
                Address::zero(),
                addr,
                "recurseCall(uint256,uint256)",
                (U256::from(2u32), U256::from(0u32)),
                0u32.into(),
                compiled.abi,
            )
            .unwrap();

        let mut mapping = BTreeMap::new();
        mapping.insert(
            "Trace".to_string(),
            (
                compiled.abi.expect("No abi").clone(),
                compiled
                    .bin_runtime
                    .expect("No runtime")
                    .clone()
                    .into_bytes()
                    .expect("Linking?")
                    .to_vec(),
            ),
        );
        let compiled = COMPILED.find("RecursiveCall").expect("could not find contract");
        mapping.insert(
            "RecursiveCall".to_string(),
            (
                compiled.abi.expect("No abi").clone(),
                compiled
                    .bin_runtime
                    .expect("No runtime")
                    .clone()
                    .into_bytes()
                    .expect("Linking?")
                    .to_vec(),
            ),
        );
        let mut identified = Default::default();
        let (funcs, events, errors) = foundry_utils::flatten_known_contracts(&mapping);
        let labels = BTreeMap::new();
        let mut exec_info =
            ExecutionInfo::new(&mapping, &mut identified, &labels, &funcs, &events, &errors);
        let mut trace_string = "".to_string();
        evm.traces()[1].construct_trace_string(0, &mut exec_info, &evm, "", &mut trace_string);
        println!("{}", trace_string);
    }

    #[test]
    fn tracing_create() {
        use std::collections::BTreeMap;

        let mut evm = vm_tracing(false);

        let compiled = COMPILED.find("Trace").expect("could not find contract");
        let (addr, _, _, _) = evm
            .deploy(
                Address::zero(),
                compiled.bin.unwrap().clone().into_bytes().expect("shouldn't be linked"),
                0.into(),
            )
            .unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, _) = evm
            .call::<(), _, _>(
                Address::zero(),
                addr,
                "recurseCreate(uint256,uint256)",
                (U256::from(3u32), U256::from(0u32)),
                0u32.into(),
                compiled.abi,
            )
            .unwrap();

        let mut mapping = BTreeMap::new();
        mapping.insert(
            "Trace".to_string(),
            (
                compiled.abi.expect("No abi").clone(),
                compiled
                    .bin_runtime
                    .expect("No runtime")
                    .clone()
                    .into_bytes()
                    .expect("Linking?")
                    .to_vec(),
            ),
        );
        let compiled = COMPILED.find("RecursiveCall").expect("could not find contract");
        mapping.insert(
            "RecursiveCall".to_string(),
            (
                compiled.abi.expect("No abi").clone(),
                compiled
                    .bin_runtime
                    .expect("No runtime")
                    .clone()
                    .into_bytes()
                    .expect("Linking?")
                    .to_vec(),
            ),
        );
        let mut identified = Default::default();
        let (funcs, events, errors) = foundry_utils::flatten_known_contracts(&mapping);
        let labels = BTreeMap::new();
        let mut exec_info =
            ExecutionInfo::new(&mapping, &mut identified, &labels, &funcs, &events, &errors);
        let mut trace_string = "".to_string();
        evm.traces()[1].construct_trace_string(0, &mut exec_info, &evm, "", &mut trace_string);
        println!("{}", trace_string);
    }
}
