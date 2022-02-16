//! Hooks to EVM execution
use super::{
    backend::CheatcodeBackend, memory_stackstate_owned::MemoryStackStateOwned, ConsoleCalls,
    HEVMCalls, HevmConsoleEvents,
};
use crate::{
    call_tracing::{CallTrace, CallTraceArena, LogCallOrder},
    sputnik::{cheatcodes::memory_stackstate_owned::ExpectedEmit, Executor, SputnikExecutor},
    Evm, ASSUME_MAGIC_RETURN_CODE,
};
use std::collections::BTreeMap;

use std::{fs::File, io::Read, path::Path};

use sputnik::{
    backend::Backend,
    executor::stack::{
        Log, PrecompileFailure, PrecompileOutput, PrecompileSet, StackExecutor, StackExitKind,
        StackState, StackSubstateMetadata,
    },
    gasometer, Capture, Config, Context, CreateScheme, ExitError, ExitReason, ExitRevert,
    ExitSucceed, Handler, Memory, Opcode, Runtime, Transfer,
};
use std::{process::Command, rc::Rc};

use ethers::{
    abi::{RawLog, Token},
    contract::EthLogDecode,
    core::{abi::AbiDecode, k256::ecdsa::SigningKey, utils},
    signers::{LocalWallet, Signer},
    solc::{artifacts::CompactContractBytecode, ProjectPathsConfig},
    types::{Address, H160, H256, U256},
};

use std::{convert::Infallible, str::FromStr};

use crate::sputnik::cheatcodes::{
    debugger::{CheatOp, DebugArena, DebugNode, DebugStep, OpCode},
    memory_stackstate_owned::Prank,
    patch_hardhat_console_log_selector,
};
use once_cell::sync::Lazy;

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

/// Wrapper around both return types for expectRevert in call or create
enum ExpectRevertReturn {
    Call(Capture<(ExitReason, Vec<u8>), Infallible>),
    Create(Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible>),
}

impl ExpectRevertReturn {
    pub fn into_call_inner(self) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        match self {
            ExpectRevertReturn::Call(inner) => inner,
            _ => panic!("tried to get call response inner from a create"),
        }
    }
    pub fn into_create_inner(self) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        match self {
            ExpectRevertReturn::Create(inner) => inner,
            _ => panic!("tried to get create response inner from a call"),
        }
    }

    pub fn is_call(&self) -> bool {
        matches!(self, ExpectRevertReturn::Call(..))
    }
}

/// For certain cheatcodes, we may internally change the status of the call, i.e. in
/// `expectRevert`. Solidity will see a successful call and attempt to abi.decode for the called
/// function. Therefore, we need to populate the return with dummy bytes such that the decode
/// doesn't fail
pub static DUMMY_OUTPUT: [u8; 320] = [0u8; 320];

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
// TODO: Should this be called `HookedHandler`? Maybe we could implement other hooks
// here, e.g. hardhat console.log-style, or dapptools logs, some ad-hoc method for tracing
// etc.
pub struct CheatcodeHandler<H> {
    handler: H,
    enable_ffi: bool,
    console_logs: Vec<String>,
}

pub(crate) fn convert_log(log: Log) -> Option<String> {
    use HevmConsoleEvents::*;
    let log = RawLog { topics: log.topics, data: log.data };
    let event = HevmConsoleEvents::decode_log(&log).ok()?;
    let ret = match event {
        LogsFilter(inner) => format!("{}", inner.0),
        LogBytesFilter(inner) => format!("{}", inner.0),
        LogNamedAddressFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedBytes32Filter(inner) => {
            format!("{}: 0x{}", inner.key, hex::encode(inner.val))
        }
        LogNamedDecimalIntFilter(inner) => {
            let (sign, val) = inner.val.into_sign_and_abs();
            format!(
                "{}: {}{}",
                inner.key,
                sign,
                ethers::utils::format_units(val, inner.decimals.as_u32()).unwrap()
            )
        }
        LogNamedDecimalUintFilter(inner) => {
            format!(
                "{}: {}",
                inner.key,
                ethers::utils::format_units(inner.val, inner.decimals.as_u32()).unwrap()
            )
        }
        LogNamedIntFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedUintFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedBytesFilter(inner) => {
            format!("{}: 0x{}", inner.key, hex::encode(inner.val))
        }
        LogNamedStringFilter(inner) => format!("{}: {}", inner.key, inner.val),

        e => e.to_string(),
    };
    Some(ret)
}

// Forwards everything internally except for the transact_call which is overwritten.
// TODO: Maybe we can pull this functionality up to the `Evm` trait to avoid having so many traits?
impl<'a, 'b, B: Backend, P: PrecompileSet> SputnikExecutor<CheatcodeStackState<'a, B>>
    for CheatcodeStackExecutor<'a, 'b, B, P>
{
    fn config(&self) -> &Config {
        self.handler.config()
    }

    fn state(&self) -> &CheatcodeStackState<'a, B> {
        self.handler.state()
    }

    fn state_mut(&mut self) -> &mut CheatcodeStackState<'a, B> {
        self.handler.state_mut()
    }

    fn expected_revert(&self) -> Option<&[u8]> {
        self.handler.state().expected_revert.as_deref()
    }

    fn set_tracing_enabled(&mut self, enabled: bool) -> bool {
        let curr = self.state_mut().trace_enabled;
        self.state_mut().trace_enabled = enabled;
        curr
    }

    fn tracing_enabled(&self) -> bool {
        self.state().trace_enabled
    }

    fn debug_calls(&self) -> Vec<DebugArena> {
        self.state().debug_steps.clone()
    }

    fn gas_left(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().gas())
    }

    fn gas_used(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().total_used_gas())
    }

    fn gas_refund(&self) -> U256 {
        U256::from(self.state().metadata().gasometer().refunded_gas())
    }

    fn all_logs(&self) -> Vec<String> {
        self.handler.state().all_logs.clone()
    }

    fn transact_call(
        &mut self,
        caller: H160,
        address: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>) {
        // reset all_logs because its a new call
        self.state_mut().all_logs = vec![];

        let transaction_cost = gasometer::call_transaction_cost(&data, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return (e.into(), Vec::new()),
        }

        // Initialize initial addresses for EIP-2929
        if self.config().increase_state_access_gas {
            let addresses = core::iter::once(caller).chain(core::iter::once(address));
            self.state_mut().metadata_mut().access_addresses(addresses);

            self.handler.initialize_with_access_list(access_list);
        }

        self.state_mut().inc_nonce(caller);

        let context = Context { caller, address, apparent_value: value };

        match self.call_inner(
            address,
            Some(Transfer { source: caller, target: address, value }),
            data,
            Some(gas_limit),
            false,
            false,
            false,
            context,
        ) {
            Capture::Exit((s, v)) => {
                if self.state().trace_enabled {
                    self.state_mut().increment_call_index();
                }

                // check if all expected calls were made
                if let Some((address, expecteds)) =
                    self.state().expected_calls.iter().find(|(_, expecteds)| !expecteds.is_empty())
                {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(format!(
                            "Expected a call to 0x{} with data {}, but got none",
                            address,
                            ethers::types::Bytes::from(expecteds[0].clone())
                        ))]),
                    )
                }

                if !self.state().expected_emits.is_empty() {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(
                            "Expected an emit, but no logs were emitted afterward".to_string(),
                        )]),
                    )
                }
                (s, v)
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
        }
    }

    fn transact_create(
        &mut self,
        caller: H160,
        value: U256,
        init_code: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason {
        // reset all_logs because its a new call
        self.state_mut().all_logs = vec![];

        let transaction_cost = gasometer::create_transaction_cost(&init_code, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return e.into(),
        };
        self.handler.initialize_with_access_list(access_list);

        match self.create_inner(
            caller,
            CreateScheme::Legacy { caller },
            value,
            init_code,
            Some(gas_limit),
            false,
        ) {
            Capture::Exit((s, _, _)) => {
                if self.state().trace_enabled {
                    self.state_mut().increment_call_index();
                }
                s
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
        }
    }

    fn create_address(&self, scheme: CreateScheme) -> Address {
        self.handler.create_address(scheme)
    }

    fn clear_logs(&mut self) {
        self.console_logs.clear();
        self.state_mut().substate.logs_mut().clear()
    }

    fn raw_logs(&self) -> Vec<RawLog> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().map(|log| RawLog { topics: log.topics, data: log.data }).collect()
    }

    fn traces(&self) -> Vec<CallTraceArena> {
        self.state().traces.clone()
    }

    fn reset_traces(&mut self) {
        self.state_mut().reset_traces();
    }

    fn logs(&self) -> Vec<String> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().filter_map(convert_log).chain(self.console_logs.clone()).collect()
    }
}

/// A [`MemoryStackStateOwned`] state instantiated over a [`CheatcodeBackend`]
pub type CheatcodeStackState<'a, B> = MemoryStackStateOwned<'a, CheatcodeBackend<B>>;

/// A [`CheatcodeHandler`] which uses a [`CheatcodeStackState`] to store its state and a
/// [`StackExecutor`] for executing transactions.
pub type CheatcodeStackExecutor<'a, 'b, B, P> =
    CheatcodeHandler<StackExecutor<'a, 'b, CheatcodeStackState<'a, B>, P>>;

impl<'a, 'b, B: Backend, P: PrecompileSet>
    Executor<CheatcodeStackState<'a, B>, CheatcodeStackExecutor<'a, 'b, B, P>>
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
        let executor = CheatcodeHandler { handler: executor, enable_ffi, console_logs: Vec::new() };

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

// helper for creating an exit type
fn evm_error(retdata: &str) -> Capture<(ExitReason, Vec<u8>), Infallible> {
    Capture::Exit((
        ExitReason::Revert(ExitRevert::Reverted),
        ethers::abi::encode(&[Token::String(retdata.to_owned())]),
    ))
}

// helper for creating the Expected Revert return type, based on if there was a call or a create,
// and if there was any decoded retdata that matched the expected revert value.
fn revert_return_evm<T: ToString>(
    call: bool,
    result: Option<(&[u8], &[u8])>,
    err: impl FnOnce() -> T,
) -> ExpectRevertReturn {
    let success =
        result.map(|(retdata, expected_revert)| retdata == expected_revert).unwrap_or(false);

    match (success, call) {
        // Success case for CALLs needs to return a dummy output value which
        // can be decoded
        (true, true) => ExpectRevertReturn::Call(Capture::Exit((
            ExitReason::Succeed(ExitSucceed::Returned),
            DUMMY_OUTPUT.to_vec(),
        ))),
        // Success case for CREATE doesn't need to return any value but must return a
        // dummy address
        (true, false) => ExpectRevertReturn::Create(Capture::Exit((
            ExitReason::Succeed(ExitSucceed::Returned),
            Some(Address::from_str("0000000000000000000000000000000000000001").unwrap()),
            Vec::new(),
        ))),
        // Failure cases just return the abi encoded error
        (false, true) => ExpectRevertReturn::Call(Capture::Exit((
            ExitReason::Revert(ExitRevert::Reverted),
            ethers::abi::encode(&[Token::String(err().to_string())]),
        ))),
        (false, false) => ExpectRevertReturn::Create(Capture::Exit((
            ExitReason::Revert(ExitRevert::Reverted),
            None,
            ethers::abi::encode(&[Token::String(err().to_string())]),
        ))),
    }
}

impl<'a, 'b, B: Backend, P: PrecompileSet> CheatcodeStackExecutor<'a, 'b, B, P> {
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
                // insert a random block hash for the specified block number if it was not
                // specified already
                self.state_mut()
                    .backend
                    .cheats
                    .block_hashes
                    .entry(inner.0)
                    .or_insert_with(H256::random);
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
            HEVMCalls::Assume(inner) => {
                self.add_debug(CheatOp::ASSUME);
                if !inner.0 {
                    res = ASSUME_MAGIC_RETURN_CODE.into();
                    return Capture::Exit((ExitReason::Revert(ExitRevert::Reverted), res))
                }
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

    // NB: This function is copy-pasted from upstream's `execute`, adjusted so that we call the
    // Runtime with our own handler
    pub fn execute(&mut self, runtime: &mut Runtime) -> ExitReason {
        match runtime.run(self) {
            Capture::Exit(s) => s,
            Capture::Trap(_) => unreachable!("Trap is Infallible"),
        }
    }

    /// Executes the call/create while also tracking the state of the machine (including opcodes)
    fn debug_execute(
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
            // actually executes the opcode step
            let r = runtime.step(self);
            match r {
                Ok(()) => {}
                Err(e) => {
                    done = true;
                    // we wont hit an interrupt when we finish stepping
                    // so we have add the accumulated steps as if debug_step returned true
                    if !steps.is_empty() {
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
                    }
                    match e {
                        Capture::Exit(s) => res = Capture::Exit(s),
                        Capture::Trap(_) => unreachable!("Trap is Infallible"),
                    }
                }
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
        if self.state().trace_enabled {
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

    fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    ) {
        self.state_mut().trace_index = pre_trace_index;
        if let Some(new_trace) = new_trace {
            let used_gas = self.handler.used_gas();
            let trace = &mut self.state_mut().trace_mut().arena[new_trace.idx].trace;
            trace.output = output.unwrap_or_default();
            trace.cost = used_gas;
            trace.success = success;
        }
    }

    // NB: This function is copy-pasted from upstream's call_inner
    #[allow(clippy::too_many_arguments)]
    fn call_inner(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        take_l64: bool,
        take_stipend: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        let pre_index = self.state().trace_index;
        let trace = self.start_trace(
            code_address,
            input.clone(),
            transfer.as_ref().map(|x| x.value).unwrap_or_default(),
            false,
        );

        macro_rules! try_or_fail {
            ( $e:expr ) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        return Capture::Exit((e.into(), Vec::new()))
                    }
                }
            };
        }

        fn l64(gas: u64) -> u64 {
            gas - gas / 64
        }

        let after_gas = if take_l64 && self.config().call_l64_after_gas {
            if self.config().estimate {
                let initial_after_gas = self.state().metadata().gasometer().gas();
                let diff = initial_after_gas - l64(initial_after_gas);
                try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(diff));
                self.state().metadata().gasometer().gas()
            } else {
                l64(self.state().metadata().gasometer().gas())
            }
        } else {
            self.state().metadata().gasometer().gas()
        };

        let target_gas = target_gas.unwrap_or(after_gas);
        let mut gas_limit = std::cmp::min(target_gas, after_gas);

        try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

        if let Some(transfer) = transfer.as_ref() {
            if take_stipend && transfer.value != U256::zero() {
                gas_limit = gas_limit.saturating_add(self.config().call_stipend);
            }
        }

        let code = self.code(code_address);
        self.handler.enter_substate(gas_limit, is_static);
        self.state_mut().touch(context.address);

        if let Some(depth) = self.state().metadata().depth() {
            if depth > self.config().call_stack_limit {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                return Capture::Exit((ExitError::CallTooDeep.into(), Vec::new()))
            }
        }

        if let Some(transfer) = transfer {
            match self.state_mut().transfer(transfer) {
                Ok(()) => (),
                Err(e) => {
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.handler.exit_substate(StackExitKind::Reverted);
                    return Capture::Exit((ExitReason::Error(e), Vec::new()))
                }
            }
        }

        if let Some(result) = self.handler.precompiles().execute(
            code_address,
            &input,
            Some(gas_limit),
            &context,
            is_static,
        ) {
            return match result {
                Ok(PrecompileOutput { exit_status, output, cost, logs }) => {
                    for Log { address, topics, data } in logs {
                        match self.log(address, topics, data) {
                            Ok(_) => continue,
                            Err(error) => {
                                self.fill_trace(&trace, false, Some(output.clone()), pre_index);
                                return Capture::Exit((ExitReason::Error(error), output))
                            }
                        }
                    }

                    let _ = self.state_mut().metadata_mut().gasometer_mut().record_cost(cost);
                    self.fill_trace(&trace, true, Some(output.clone()), pre_index);
                    let _ = self.handler.exit_substate(StackExitKind::Succeeded);
                    Capture::Exit((ExitReason::Succeed(exit_status), output))
                }
                Err(e) => {
                    let e = match e {
                        PrecompileFailure::Error { exit_status } => ExitReason::Error(exit_status),
                        PrecompileFailure::Revert { exit_status, .. } => {
                            ExitReason::Revert(exit_status)
                        }
                        PrecompileFailure::Fatal { exit_status } => ExitReason::Fatal(exit_status),
                    };
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.handler.exit_substate(StackExitKind::Failed);
                    Capture::Exit((e, Vec::new()))
                }
            }
        }

        // each cfg is about 200 bytes, is this a lot to clone? why does this error
        // not manifest upstream?
        let config = self.config().clone();
        let mut runtime;
        let reason = if self.state().debug_enabled {
            let code = Rc::new(code);
            runtime = Runtime::new(code.clone(), Rc::new(input), context, &config);
            self.debug_execute(&mut runtime, code_address, code, false)
        } else {
            runtime = Runtime::new(Rc::new(code), Rc::new(input), context, &config);
            self.execute(&mut runtime)
        };

        // // log::debug!(target: "evm", "Call execution using address {}: {:?}", code_address,
        // reason);

        match reason {
            ExitReason::Succeed(s) => {
                self.fill_trace(&trace, true, Some(runtime.machine().return_value()), pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Succeeded);
                Capture::Exit((ExitReason::Succeed(s), runtime.machine().return_value()))
            }
            ExitReason::Error(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Error(e), Vec::new()))
            }
            ExitReason::Revert(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                Capture::Exit((ExitReason::Revert(e), runtime.machine().return_value()))
            }
            ExitReason::Fatal(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                self.state_mut().metadata_mut().gasometer_mut().fail();
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Fatal(e), Vec::new()))
            }
        }
    }

    // NB: This function is copy-pasted from upstream's create_inner
    fn create_inner(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
        take_l64: bool,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        let pre_index = self.state().trace_index;

        let address = self.create_address(scheme);
        let trace = self.start_trace(address, init_code.clone(), value, true);

        macro_rules! try_or_fail {
            ( $e:expr ) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        return Capture::Exit((e.into(), None, Vec::new()))
                    }
                }
            };
        }

        fn check_first_byte(config: &Config, code: &[u8]) -> Result<(), ExitError> {
            if config.disallow_executable_format {
                if let Some(0xef) = code.get(0) {
                    return Err(ExitError::InvalidCode)
                }
            }
            Ok(())
        }

        fn l64(gas: u64) -> u64 {
            gas - gas / 64
        }

        self.state_mut().metadata_mut().access_address(caller);
        self.state_mut().metadata_mut().access_address(address);

        if let Some(depth) = self.state().metadata().depth() {
            if depth > self.config().call_stack_limit {
                self.fill_trace(&trace, false, None, pre_index);
                return Capture::Exit((ExitError::CallTooDeep.into(), None, Vec::new()))
            }
        }

        if self.balance(caller) < value {
            self.fill_trace(&trace, false, None, pre_index);
            return Capture::Exit((ExitError::OutOfFund.into(), None, Vec::new()))
        }

        let after_gas = if take_l64 && self.config().call_l64_after_gas {
            if self.config().estimate {
                let initial_after_gas = self.state().metadata().gasometer().gas();
                let diff = initial_after_gas - l64(initial_after_gas);
                try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(diff));
                self.state().metadata().gasometer().gas()
            } else {
                l64(self.state().metadata().gasometer().gas())
            }
        } else {
            self.state().metadata().gasometer().gas()
        };

        let target_gas = target_gas.unwrap_or(after_gas);

        let gas_limit = core::cmp::min(after_gas, target_gas);
        try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

        self.state_mut().inc_nonce(caller);

        self.handler.enter_substate(gas_limit, false);

        {
            if self.code_size(address) != U256::zero() {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
            }

            if self.handler.nonce(address) > U256::zero() {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
            }

            self.state_mut().reset_storage(address);
        }

        let context = Context { address, caller, apparent_value: value };
        let transfer = Transfer { source: caller, target: address, value };
        match self.state_mut().transfer(transfer) {
            Ok(()) => (),
            Err(e) => {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                return Capture::Exit((ExitReason::Error(e), None, Vec::new()))
            }
        }

        if self.config().create_increase_nonce {
            self.state_mut().inc_nonce(address);
        }

        let config = self.config().clone();
        let mut runtime;
        let reason = if self.state().debug_enabled {
            let code = Rc::new(init_code);
            runtime = Runtime::new(code.clone(), Rc::new(Vec::new()), context, &config);
            self.debug_execute(&mut runtime, address, code, true)
        } else {
            runtime = Runtime::new(Rc::new(init_code), Rc::new(Vec::new()), context, &config);
            self.execute(&mut runtime)
        };
        // log::debug!(target: "evm", "Create execution using address {}: {:?}", address, reason);

        match reason {
            ExitReason::Succeed(s) => {
                let out = runtime.machine().return_value();

                // As of EIP-3541 code starting with 0xef cannot be deployed
                if let Err(e) = check_first_byte(self.config(), &out) {
                    self.state_mut().metadata_mut().gasometer_mut().fail();
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.handler.exit_substate(StackExitKind::Failed);
                    return Capture::Exit((e.into(), None, Vec::new()))
                }

                if let Some(limit) = self.config().create_contract_limit {
                    if out.len() > limit {
                        self.state_mut().metadata_mut().gasometer_mut().fail();
                        self.fill_trace(&trace, false, None, pre_index);
                        let _ = self.handler.exit_substate(StackExitKind::Failed);
                        return Capture::Exit((
                            ExitError::CreateContractLimit.into(),
                            None,
                            Vec::new(),
                        ))
                    }
                }

                match self.state_mut().metadata_mut().gasometer_mut().record_deposit(out.len()) {
                    Ok(()) => {
                        self.fill_trace(&trace, true, Some(out.clone()), pre_index);
                        let e = self.handler.exit_substate(StackExitKind::Succeeded);
                        self.state_mut().set_code(address, out);
                        // this may overwrite the trace and thats okay
                        try_or_fail!(e);
                        Capture::Exit((ExitReason::Succeed(s), Some(address), Vec::new()))
                    }
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        let _ = self.handler.exit_substate(StackExitKind::Failed);
                        Capture::Exit((ExitReason::Error(e), None, Vec::new()))
                    }
                }
            }
            ExitReason::Error(e) => {
                self.state_mut().metadata_mut().gasometer_mut().fail();
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Error(e), None, Vec::new()))
            }
            ExitReason::Revert(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                Capture::Exit((ExitReason::Revert(e), None, runtime.machine().return_value()))
            }
            ExitReason::Fatal(e) => {
                self.state_mut().metadata_mut().gasometer_mut().fail();
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Fatal(e), None, Vec::new()))
            }
        }
    }
}

// Delegates everything internally, except the `call_inner` call, which is hooked
// so that we can modify
impl<'a, 'b, B: Backend, P: PrecompileSet> Handler for CheatcodeStackExecutor<'a, 'b, B, P> {
    type CreateInterrupt = Infallible;
    type CreateFeedback = Infallible;
    type CallInterrupt = Infallible;
    type CallFeedback = Infallible;

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
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
            let res = self.call_inner(
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

    // Everything else is left the same
    fn balance(&self, address: H160) -> U256 {
        self.handler.balance(address)
    }

    fn code_size(&self, address: H160) -> U256 {
        self.handler.code_size(address)
    }

    fn code_hash(&self, address: H160) -> H256 {
        self.handler.code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.handler.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.handler.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.handler.original_storage(address, index)
    }

    fn gas_left(&self) -> U256 {
        // Need to disambiguate type, because the same method exists in the `SputnikExecutor`
        // trait and the `Handler` trait.
        Handler::gas_left(&self.handler)
    }

    fn gas_price(&self) -> U256 {
        self.handler.gas_price()
    }

    fn origin(&self) -> H160 {
        self.handler.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.handler.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.handler.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.handler.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.handler.block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.handler.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.handler.block_gas_limit()
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.handler.block_base_fee_per_gas()
    }

    fn chain_id(&self) -> U256 {
        self.handler.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.handler.exists(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.handler.deleted(address)
    }

    fn is_cold(&self, address: H160, index: Option<H256>) -> bool {
        self.handler.is_cold(address, index)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.handler.set_storage(address, index, value)
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        if self.state().trace_enabled {
            let index = self.state().trace_index;
            let node = &mut self.state_mut().traces.last_mut().expect("no traces").arena[index];
            node.ordering.push(LogCallOrder::Log(node.logs.len()));
            node.logs.push(RawLog { topics: topics.clone(), data: data.clone() });
        }

        if let Some(decoded) =
            convert_log(Log { address, topics: topics.clone(), data: data.clone() })
        {
            self.state_mut().all_logs.push(decoded);
        }

        if !self.state().expected_emits.is_empty() {
            // get expected emits
            let expected_emits = &mut self.state_mut().expected_emits;

            // do we have empty expected emits to fill?
            if let Some(next_expect_to_fill) =
                expected_emits.iter_mut().find(|expect| expect.log.is_none())
            {
                next_expect_to_fill.log =
                    Some(RawLog { topics: topics.clone(), data: data.clone() });
            } else {
                // no unfilled, grab next unfound
                // try to fill the first unfound
                if let Some(next_expect) = expected_emits.iter_mut().find(|expect| !expect.found) {
                    // unpack the log
                    if let Some(RawLog { topics: expected_topics, data: expected_data }) =
                        &next_expect.log
                    {
                        if expected_topics[0] == topics[0] {
                            // same event topic 0, topic length should be the same
                            let topics_match = topics
                                .iter()
                                .skip(1)
                                .enumerate()
                                .filter(|(i, _topic)| {
                                    // do we want to check?
                                    next_expect.checks[*i]
                                })
                                .all(|(i, topic)| topic == &expected_topics[i + 1]);

                            // check data
                            next_expect.found = if next_expect.checks[3] {
                                expected_data == &data && topics_match
                            } else {
                                topics_match
                            };
                        }
                    }
                }
            }
        }

        self.handler.log(address, topics, data)
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.handler.mark_delete(address, target)
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
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

        let res = self.create_inner(new_tx_caller, new_scheme, value, init_code, target_gas, true);

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

    fn pre_validate(
        &mut self,
        context: &Context,
        opcode: sputnik::Opcode,
        stack: &sputnik::Stack,
    ) -> Result<(), ExitError> {
        self.handler.pre_validate(context, opcode, stack)
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
        evm.executor.enable_ffi = false;

        let compiled = COMPILED.find("CheatCodes").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

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
