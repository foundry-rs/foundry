use super::{ensure, fmt_err, Cheatcodes, Result};
use crate::{
    abi::HEVMCalls,
    executor::{
        backend::DatabaseExt,
        inspector::cheatcodes::{
            mapping::{get_mapping_key_and_parent, get_mapping_length, get_mapping_slot_at},
            util::{is_potential_precompile, with_journaled_account},
            DealRecord,
        },
    },
};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, Bytes, Log, B256, U256};
use ethers::signers::{LocalWallet, Signer};
use foundry_config::Config;
use foundry_utils::types::{ToAlloy, ToEthers};
use revm::{
    primitives::{Bytecode, SpecId, KECCAK_EMPTY},
    Database, EVMData,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Broadcast {
    /// Address of the transaction origin
    pub new_origin: Address,
    /// Original caller
    pub original_caller: Address,
    /// Original `tx.origin`
    pub original_origin: Address,
    /// Depth of the broadcast
    pub depth: u64,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Prank {
    /// Address of the contract that initiated the prank
    pub prank_caller: Address,
    /// Address of `tx.origin` when the prank was initiated
    pub prank_origin: Address,
    /// The address to assign to `msg.sender`
    pub new_caller: Address,
    /// The address to assign to `tx.origin`
    pub new_origin: Option<Address>,
    /// The depth at which the prank was called
    pub depth: u64,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
    /// Whether the prank has been used yet (false if unused)
    pub used: bool,
}

impl Prank {
    pub fn new(
        prank_caller: Address,
        prank_origin: Address,
        new_caller: Address,
        new_origin: Option<Address>,
        depth: u64,
        single_call: bool,
    ) -> Prank {
        Prank {
            prank_caller,
            prank_origin,
            new_caller,
            new_origin,
            depth,
            single_call,
            used: false,
        }
    }

    /// Apply the prank by setting `used` to true iff it is false
    /// Only returns self in the case it is updated (first application)
    pub fn first_time_applied(&self) -> Option<Self> {
        if self.used {
            None
        } else {
            Some(Prank { used: true, ..self.clone() })
        }
    }
}

/// Represents the possible caller modes for the readCallers() cheat code return value
enum CallerMode {
    /// No caller modification is currently active
    None,
    /// A one time broadcast triggered by a `vm.broadcast()` call is currently active
    Broadcast,
    /// A recurrent broadcast triggered by a `vm.startBroadcast()` call is currently active
    RecurrentBroadcast,
    /// A one time prank triggered by a `vm.prank()` call is currently active
    Prank,
    /// A recurrent prank triggered by a `vm.startPrank()` call is currently active
    RecurrentPrank,
}

impl From<CallerMode> for U256 {
    fn from(value: CallerMode) -> Self {
        U256::from(value as u8)
    }
}

/// Sets up broadcasting from a script using `origin` as the sender
fn broadcast(
    state: &mut Cheatcodes,
    new_origin: Address,
    original_caller: Address,
    original_origin: Address,
    depth: u64,
    single_call: bool,
) -> Result {
    ensure!(
        state.prank.is_none(),
        "You have an active prank. Broadcasting and pranks are not compatible. \
         Disable one or the other"
    );
    ensure!(state.broadcast.is_none(), "You have an active broadcast already.");

    let broadcast = Broadcast { new_origin, original_origin, original_caller, depth, single_call };
    state.broadcast = Some(broadcast);
    Ok(Bytes::new())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`
/// Adds this private key to `state`'s `script_wallets` vector to later be used for signing
/// iff broadcast is successful
fn broadcast_key(
    state: &mut Cheatcodes,
    private_key: U256,
    original_caller: Address,
    original_origin: Address,
    chain_id: U256,
    depth: u64,
    single_call: bool,
) -> Result {
    let key = super::util::parse_private_key(private_key.to_ethers())?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.to::<u64>());
    let new_origin = wallet.address();

    let result = broadcast(
        state,
        new_origin.to_alloy(),
        original_caller,
        original_origin,
        depth,
        single_call,
    );
    if result.is_ok() {
        state.script_wallets.push(wallet);
    }
    result
}

fn prank(
    state: &mut Cheatcodes,
    prank_caller: Address,
    prank_origin: Address,
    new_caller: Address,
    new_origin: Option<Address>,
    depth: u64,
    single_call: bool,
) -> Result {
    let prank = Prank::new(prank_caller, prank_origin, new_caller, new_origin, depth, single_call);

    if let Some(Prank { used, single_call: current_single_call, .. }) = state.prank {
        ensure!(used, "You cannot overwrite `prank` until it is applied at least once");
        // This case can only fail if the user calls `vm.startPrank` and then `vm.prank` later on.
        // This should not be possible without first calling `stopPrank`
        ensure!(single_call == current_single_call, "You cannot override an ongoing prank with a single vm.prank. Use vm.startPrank to override the current prank.");
    }

    ensure!(
        state.broadcast.is_none(),
        "You cannot `prank` for a broadcasted transaction.\
         Pass the desired tx.origin into the broadcast cheatcode call"
    );

    state.prank = Some(prank);
    Ok(Bytes::new())
}

/// Reads the current caller information and returns the current [CallerMode], `msg.sender` and
/// `tx.origin`.
///
/// Depending on the current caller mode, one of the following results will be returned:
/// - If there is an active prank:
///     - caller_mode will be equal to:
///         - [CallerMode::Prank] if the prank has been set with `vm.prank(..)`.
///         - [CallerMode::RecurrentPrank] if the prank has been set with `vm.startPrank(..)`.
///     - `msg.sender` will be equal to the address set for the prank.
///     - `tx.origin` will be equal to the default sender address unless an alternative one has been
///       set when configuring the prank.
///
/// - If there is an active broadcast:
///     - caller_mode will be equal to:
///         - [CallerMode::Broadcast] if the broadcast has been set with `vm.broadcast(..)`.
///         - [CallerMode::RecurrentBroadcast] if the broadcast has been set with
///           `vm.startBroadcast(..)`.
///     - `msg.sender` and `tx.origin` will be equal to the address provided when setting the
///       broadcast.
///
/// - If no caller modification is active:
///     - caller_mode will be equal to [CallerMode::None],
///     - `msg.sender` and `tx.origin` will be equal to the default sender address.
fn read_callers(state: &Cheatcodes, default_sender: Address) -> Bytes {
    let Cheatcodes { prank, broadcast, .. } = &state;

    let data = if let Some(prank) = prank {
        let caller_mode =
            if prank.single_call { CallerMode::Prank } else { CallerMode::RecurrentPrank };

        vec![
            DynSolValue::Uint(caller_mode.into(), 32),
            DynSolValue::Address(prank.new_caller),
            DynSolValue::Address(prank.new_origin.unwrap_or(default_sender)),
        ]
    } else if let Some(broadcast) = broadcast {
        let caller_mode = if broadcast.single_call {
            CallerMode::Broadcast
        } else {
            CallerMode::RecurrentBroadcast
        };

        vec![
            DynSolValue::Uint(caller_mode.into(), 32),
            DynSolValue::Address(broadcast.new_origin),
            DynSolValue::Address(broadcast.new_origin),
        ]
    } else {
        vec![
            DynSolValue::Uint(CallerMode::None.into(), 32),
            DynSolValue::Address(default_sender),
            DynSolValue::Address(default_sender),
        ]
    };

    DynSolValue::Tuple(data).encode().into()
}

#[derive(Clone, Debug, Default)]
pub struct RecordAccess {
    pub reads: BTreeMap<Address, Vec<U256>>,
    pub writes: BTreeMap<Address, Vec<U256>>,
}

fn start_record(state: &mut Cheatcodes) {
    state.accesses = Some(Default::default());
}

fn accesses(state: &mut Cheatcodes, address: Address) -> Bytes {
    if let Some(storage_accesses) = &mut state.accesses {
        println!("encoding that {storage_accesses:?}");
        let write_accesses: Vec<DynSolValue> = storage_accesses
            .writes
            .entry(address)
            .or_default()
            .iter_mut()
            .map(|u| DynSolValue::FixedBytes(u.to_owned().into(), 32))
            .collect();
        let read_accesses = storage_accesses
            .reads
            .entry(address)
            .or_default()
            .iter_mut()
            .map(|u| DynSolValue::FixedBytes(u.to_owned().into(), 32))
            .collect();
        DynSolValue::Tuple(vec![
            DynSolValue::Array(read_accesses),
            DynSolValue::Array(write_accesses),
        ])
        .encode()
        .into()
    } else {
        println!("encoding this");
        DynSolValue::Tuple(vec![DynSolValue::Array(vec![]), DynSolValue::Array(vec![])])
            .encode()
            .into()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecordedLogs {
    pub entries: Vec<RecordedLog>,
}

#[derive(Clone, Debug)]
pub struct RecordedLog {
    pub emitter: Address,
    pub inner: Log,
}

fn start_record_logs(state: &mut Cheatcodes) {
    state.recorded_logs = Some(Default::default());
}

fn get_recorded_logs(state: &mut Cheatcodes) -> Bytes {
    if let Some(recorded_logs) = state.recorded_logs.replace(Default::default()) {
        DynSolValue::Array(
            recorded_logs
                .entries
                .iter()
                .map(|entry| {
                    DynSolValue::Tuple(vec![
                        DynSolValue::Array(
                            entry
                                .inner
                                .topics
                                .clone()
                                .into_iter()
                                .map(|t| DynSolValue::FixedBytes(t, 32))
                                .collect(),
                        ),
                        DynSolValue::Bytes(entry.inner.data.clone().to_vec()),
                        DynSolValue::Address(entry.emitter),
                    ])
                })
                .collect::<Vec<DynSolValue>>(),
        )
        .encode()
        .into()
    } else {
        DynSolValue::Array(vec![]).encode().into()
    }
}

/// Entry point of the breakpoint cheatcode. Adds the called breakpoint to the state.
fn add_breakpoint(state: &mut Cheatcodes, caller: Address, inner: &str, add: bool) -> Result {
    let mut chars = inner.chars();
    let point = chars.next();

    let point =
        point.ok_or_else(|| fmt_err!("Please provide at least one char for the breakpoint"))?;

    ensure!(chars.next().is_none(), "Provide only one character for the breakpoint");
    ensure!(point.is_alphabetic(), "Only alphabetic characters are accepted as breakpoints");

    // add a breakpoint from the interpreter
    if add {
        state.breakpoints.insert(point, (caller.to_ethers(), state.pc));
    } else {
        state.breakpoints.remove(&point);
    }

    Ok(Bytes::new())
}

// mark the slots of an account and the account address as cold
fn cool_account<DB: DatabaseExt>(data: &mut EVMData<'_, DB>, address: Address) -> Result {
    if let Some(account) = data.journaled_state.state.get_mut(&address) {
        if account.is_touched() {
            account.unmark_touch();
        }
        account.storage.clear();
    }

    Ok(Bytes::new())
}

#[instrument(level = "error", name = "env", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    caller: Address,
    call: &HEVMCalls,
) -> Result<Option<Bytes>> {
    let result = match call {
        HEVMCalls::Warp(inner) => {
            data.env.block.timestamp = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Difficulty(inner) => {
            ensure!(
                data.env.cfg.spec_id < SpecId::MERGE,
                "`difficulty` is not supported after the Paris hard fork, \
                 use `prevrandao` instead. \
                 For more information, please see https://eips.ethereum.org/EIPS/eip-4399"
            );
            data.env.block.difficulty = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Prevrandao(inner) => {
            ensure!(
                data.env.cfg.spec_id >= SpecId::MERGE,
                "`prevrandao` is not supported before the Paris hard fork, \
                 use `difficulty` instead. \
                 For more information, please see https://eips.ethereum.org/EIPS/eip-4399"
            );
            data.env.block.prevrandao = Some(B256::from(inner.0));
            Bytes::new()
        }
        HEVMCalls::Roll(inner) => {
            data.env.block.number = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Fee(inner) => {
            data.env.block.basefee = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Coinbase(inner) => {
            data.env.block.coinbase = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Store(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Store cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            data.journaled_state.load_account(inner.0.to_alloy(), data.db)?;
            // ensure the account is touched
            data.journaled_state.touch(&inner.0.to_alloy());

            data.journaled_state.sstore(
                inner.0.to_alloy(),
                U256::from_be_bytes(inner.1),
                U256::from_be_bytes(inner.2),
                data.db,
            )?;
            Bytes::new()
        }
        HEVMCalls::Load(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Load cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            // TODO: Does this increase gas usage?
            data.journaled_state.load_account(inner.0.to_alloy(), data.db)?;
            let (val, _) = data.journaled_state.sload(
                inner.0.to_alloy(),
                U256::from_be_bytes(inner.1),
                data.db,
            )?;
            DynSolValue::from(val).encode().into()
        }
        HEVMCalls::Cool(inner) => cool_account(data, inner.0.to_alloy())?,
        HEVMCalls::Breakpoint0(inner) => add_breakpoint(state, caller, &inner.0, true)?,
        HEVMCalls::Breakpoint1(inner) => add_breakpoint(state, caller, &inner.0, inner.1)?,
        HEVMCalls::Etch(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Etch cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            let code = inner.1.clone();
            trace!(address=?inner.0, code=?hex::encode(&code), "etch cheatcode");
            // TODO: Does this increase gas usage?
            data.journaled_state.load_account(inner.0.to_alloy(), data.db)?;
            data.journaled_state.set_code(
                inner.0.to_alloy(),
                Bytecode::new_raw(alloy_primitives::Bytes(code.0)).to_checked(),
            );
            Bytes::new()
        }
        HEVMCalls::Deal(inner) => {
            let who = inner.0;
            let value = inner.1;
            trace!(?who, ?value, "deal cheatcode");
            with_journaled_account(&mut data.journaled_state, data.db, who, |account| {
                // record the deal
                let record = DealRecord {
                    address: who.to_alloy(),
                    old_balance: account.info.balance,
                    new_balance: value.to_alloy(),
                };
                state.eth_deals.push(record);

                account.info.balance = value.to_alloy();
            })?;
            Bytes::new()
        }
        HEVMCalls::Prank0(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0.to_alloy(),
            None,
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::Prank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0.to_alloy(),
            Some(inner.1.to_alloy()),
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::StartPrank0(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0.to_alloy(),
            None,
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StartPrank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0.to_alloy(),
            Some(inner.1.to_alloy()),
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StopPrank(_) => {
            ensure!(state.prank.is_some(), "No prank in progress to stop");
            state.prank = None;
            Bytes::new()
        }
        HEVMCalls::ReadCallers(_) => read_callers(state, data.env.tx.caller),
        HEVMCalls::Record(_) => {
            start_record(state);
            Bytes::new()
        }
        HEVMCalls::Accesses(inner) => accesses(state, inner.0.to_alloy()),
        HEVMCalls::RecordLogs(_) => {
            start_record_logs(state);
            Bytes::new()
        }
        HEVMCalls::GetRecordedLogs(_) => get_recorded_logs(state),
        HEVMCalls::SetNonce(inner) => {
            with_journaled_account(
                &mut data.journaled_state,
                data.db,
                inner.0,
                |account| -> Result {
                    // nonce must increment only
                    let current = account.info.nonce;
                    let new = inner.1;
                    ensure!(
                        new >= current,
                        "New nonce ({new}) must be strictly equal to or higher than the \
                         account's current nonce ({current})."
                    );
                    account.info.nonce = new;
                    Ok(Bytes::new())
                },
            )??
        }
        HEVMCalls::SetNonceUnsafe(inner) => with_journaled_account(
            &mut data.journaled_state,
            data.db,
            inner.0,
            |account| -> Result {
                let new = inner.1;
                account.info.nonce = new;
                Ok(Bytes::new())
            },
        )??,
        HEVMCalls::ResetNonce(inner) => with_journaled_account(
            &mut data.journaled_state,
            data.db,
            inner.0,
            |account| -> Result {
                // Per EIP-161, EOA nonces start at 0, but contract nonces
                // start at 1. Comparing by code_hash instead of code
                // to avoid hitting the case where account's code is None.
                let empty = account.info.code_hash == KECCAK_EMPTY;
                let nonce = if empty { 0 } else { 1 };
                account.info.nonce = nonce;
                Ok(Bytes::new())
            },
        )??,
        // [function getNonce(address)] returns the current nonce of a given ETH address
        HEVMCalls::GetNonce1(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;

            // TODO:  this is probably not a good long-term solution since it might mess up the gas
            // calculations
            data.journaled_state.load_account(inner.0.to_alloy(), data.db)?;

            // we can safely unwrap because `load_account` insert inner.0 to DB.
            let account = data.journaled_state.state().get(&inner.0.to_alloy()).unwrap();
            DynSolValue::from(account.info.nonce).encode().into()
        }
        // [function getNonce(Wallet)] returns the current nonce of the Wallet's ETH address
        HEVMCalls::GetNonce0(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;

            // TODO:  this is probably not a good long-term solution since it might mess up the gas
            // calculations
            data.journaled_state.load_account(inner.0.addr.to_alloy(), data.db)?;

            // we can safely unwrap because `load_account` insert inner.0 to DB.
            let account = data.journaled_state.state().get(&inner.0.addr.to_alloy()).unwrap();
            DynSolValue::from(account.info.nonce.to_alloy()).encode().into()
        }
        HEVMCalls::ChainId(inner) => {
            ensure!(
                inner.0.to_alloy() <= U256::from(u64::MAX),
                "Chain ID must be less than 2^64 - 1"
            );
            data.env.cfg.chain_id = inner.0.as_u64();
            Bytes::new()
        }
        HEVMCalls::TxGasPrice(inner) => {
            data.env.tx.gas_price = inner.0.to_alloy();
            Bytes::new()
        }
        HEVMCalls::Broadcast0(_) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                data.env.tx.caller,
                caller,
                data.env.tx.caller,
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::Broadcast1(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                inner.0.to_alloy(),
                caller,
                data.env.tx.caller,
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::Broadcast2(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast_key(
                state,
                inner.0.to_alloy(),
                caller,
                data.env.tx.caller,
                U256::from(data.env.cfg.chain_id),
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::StartBroadcast0(_) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                data.env.tx.caller,
                caller,
                data.env.tx.caller,
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StartBroadcast1(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                inner.0.to_alloy(),
                caller,
                data.env.tx.caller,
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StartBroadcast2(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast_key(
                state,
                inner.0.to_alloy(),
                caller,
                data.env.tx.caller,
                U256::from(data.env.cfg.chain_id),
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StopBroadcast(_) => {
            ensure!(state.broadcast.is_some(), "No broadcast in progress to stop");
            state.broadcast = None;
            Bytes::new()
        }
        HEVMCalls::PauseGasMetering(_) => {
            if state.gas_metering.is_none() {
                state.gas_metering = Some(None);
            }
            Bytes::new()
        }
        HEVMCalls::ResumeGasMetering(_) => {
            state.gas_metering = None;
            Bytes::new()
        }
        HEVMCalls::StartMappingRecording(_) => {
            if state.mapping_slots.is_none() {
                state.mapping_slots = Some(Default::default());
            }
            Bytes::new()
        }
        HEVMCalls::StopMappingRecording(_) => {
            state.mapping_slots = None;
            Bytes::new()
        }
        HEVMCalls::GetMappingLength(inner) => {
            get_mapping_length(state, inner.0.to_alloy(), U256::from_be_bytes(inner.1))
        }
        HEVMCalls::GetMappingSlotAt(inner) => get_mapping_slot_at(
            state,
            inner.0.to_alloy(),
            U256::from_be_bytes(inner.1),
            inner.2.to_alloy(),
        ),
        HEVMCalls::GetMappingKeyAndParentOf(inner) => {
            get_mapping_key_and_parent(state, inner.0.to_alloy(), U256::from_be_bytes(inner.1))
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

/// When using `forge script`, the script method is called using the address from `--sender`.
/// That leads to its nonce being incremented by `call_raw`. In a `broadcast` scenario this is
/// undesirable. Therefore, we make sure to fix the sender's nonce **once**.
fn correct_sender_nonce<DB: Database>(
    sender: Address,
    journaled_state: &mut revm::JournaledState,
    db: &mut DB,
    state: &mut Cheatcodes,
) -> Result<(), DB::Error> {
    if !state.corrected_nonce && sender.to_ethers() != Config::DEFAULT_SENDER {
        with_journaled_account(journaled_state, db, sender.to_ethers(), |account| {
            account.info.nonce = account.info.nonce.saturating_sub(1);
            state.corrected_nonce = true;
        })?;
    }
    Ok(())
}
