use super::{ensure, fmt_err, Cheatcodes, Result};
use crate::{
    abi::HEVMCalls,
    executor::{
        backend::DatabaseExt,
        inspector::cheatcodes::{
            util::{is_potential_precompile, with_journaled_account},
            DealRecord,
        },
    },
    utils::{b160_to_h160, h160_to_b160, ru256_to_u256, u256_to_ru256},
};
use ethers::{
    abi::{self, AbiEncode, RawLog, Token, Tokenizable, Tokenize},
    signers::{LocalWallet, Signer},
    types::{Address, Bytes, U256},
};
use foundry_config::Config;
use revm::{
    primitives::{Bytecode, SpecId, B256, KECCAK_EMPTY},
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
        (value as i8).into()
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
    let key = super::util::parse_private_key(private_key)?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());
    let new_origin = wallet.address();

    let result = broadcast(state, new_origin, original_caller, original_origin, depth, single_call);
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

    if let Some(Prank { used, .. }) = state.prank {
        ensure!(used, "You cannot overwrite `prank` until it is applied at least once");
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

        [
            Token::Uint(caller_mode.into()),
            Token::Address(prank.new_caller),
            Token::Address(prank.new_origin.unwrap_or(default_sender)),
        ]
    } else if let Some(broadcast) = broadcast {
        let caller_mode = if broadcast.single_call {
            CallerMode::Broadcast
        } else {
            CallerMode::RecurrentBroadcast
        };

        [
            Token::Uint(caller_mode.into()),
            Token::Address(broadcast.new_origin),
            Token::Address(broadcast.new_origin),
        ]
    } else {
        [
            Token::Uint(CallerMode::None.into()),
            Token::Address(default_sender),
            Token::Address(default_sender),
        ]
    };

    abi::encode(&data).into()
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
        let first_token =
            |x: Option<Vec<_>>| x.unwrap_or_default().into_tokens().into_iter().next().unwrap();
        ethers::abi::encode(&[
            first_token(storage_accesses.reads.remove(&address)),
            first_token(storage_accesses.writes.remove(&address)),
        ])
        .into()
    } else {
        ethers::abi::encode(&[Token::Array(vec![]), Token::Array(vec![])]).into()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecordedLogs {
    pub entries: Vec<Log>,
}

#[derive(Clone, Debug)]
pub struct Log {
    pub emitter: Address,
    pub inner: RawLog,
}

fn start_record_logs(state: &mut Cheatcodes) {
    state.recorded_logs = Some(Default::default());
}

fn get_recorded_logs(state: &mut Cheatcodes) -> Bytes {
    if let Some(recorded_logs) = state.recorded_logs.replace(Default::default()) {
        abi::encode(
            &recorded_logs
                .entries
                .iter()
                .map(|entry| {
                    Token::Tuple(vec![
                        entry.inner.topics.clone().into_token(),
                        Token::Bytes(entry.inner.data.clone()),
                        entry.emitter.into_token(),
                    ])
                })
                .collect::<Vec<Token>>()
                .into_tokens(),
        )
        .into()
    } else {
        abi::encode(&[Token::Array(vec![])]).into()
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
        state.breakpoints.insert(point, (caller, state.pc));
    } else {
        state.breakpoints.remove(&point);
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
            data.env.block.timestamp = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::Difficulty(inner) => {
            ensure!(
                data.env.cfg.spec_id < SpecId::MERGE,
                "Difficulty is not supported after the Paris hard fork. Please use vm.prevrandao instead. For more information, please see https://eips.ethereum.org/EIPS/eip-4399"
            );
            data.env.block.difficulty = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::Prevrandao(inner) => {
            ensure!(
                data.env.cfg.spec_id >= SpecId::MERGE,
                "Prevrandao is not supported before the Paris hard fork. Please use vm.difficulty instead. For more information, please see https://eips.ethereum.org/EIPS/eip-4399"
            );
            data.env.block.prevrandao = Some(B256::from(inner.0));
            Bytes::new()
        }
        HEVMCalls::Roll(inner) => {
            data.env.block.number = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::Fee(inner) => {
            data.env.block.basefee = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::Coinbase(inner) => {
            data.env.block.coinbase = h160_to_b160(inner.0);
            Bytes::new()
        }
        HEVMCalls::Store(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Store cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            data.journaled_state.load_account(h160_to_b160(inner.0), data.db)?;
            // ensure the account is touched
            data.journaled_state.touch(&h160_to_b160(inner.0));

            data.journaled_state.sstore(
                h160_to_b160(inner.0),
                u256_to_ru256(inner.1.into()),
                u256_to_ru256(inner.2.into()),
                data.db,
            )?;
            Bytes::new()
        }
        HEVMCalls::Load(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Load cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            // TODO: Does this increase gas usage?
            data.journaled_state.load_account(h160_to_b160(inner.0), data.db)?;
            let (val, _) = data.journaled_state.sload(
                h160_to_b160(inner.0),
                u256_to_ru256(inner.1.into()),
                data.db,
            )?;
            ru256_to_u256(val).encode().into()
        }
        HEVMCalls::Breakpoint0(inner) => add_breakpoint(state, caller, &inner.0, true)?,
        HEVMCalls::Breakpoint1(inner) => add_breakpoint(state, caller, &inner.0, inner.1)?,
        HEVMCalls::Etch(inner) => {
            ensure!(!is_potential_precompile(inner.0), "Etch cannot be used on precompile addresses (N < 10). Please use an address bigger than 10 instead");
            let code = inner.1.clone();
            trace!(address=?inner.0, code=?hex::encode(&code), "etch cheatcode");
            // TODO: Does this increase gas usage?
            data.journaled_state.load_account(h160_to_b160(inner.0), data.db)?;
            data.journaled_state
                .set_code(h160_to_b160(inner.0), Bytecode::new_raw(code.0).to_checked());
            Bytes::new()
        }
        HEVMCalls::Deal(inner) => {
            let who = inner.0;
            let value = inner.1;
            trace!(?who, ?value, "deal cheatcode");
            with_journaled_account(&mut data.journaled_state, data.db, who, |account| {
                // record the deal
                let record = DealRecord {
                    address: who,
                    old_balance: account.info.balance.into(),
                    new_balance: value,
                };
                state.eth_deals.push(record);

                account.info.balance = value.into();
            })?;
            Bytes::new()
        }
        HEVMCalls::Prank0(inner) => prank(
            state,
            caller,
            b160_to_h160(data.env.tx.caller),
            inner.0,
            None,
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::Prank1(inner) => prank(
            state,
            caller,
            b160_to_h160(data.env.tx.caller),
            inner.0,
            Some(inner.1),
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::StartPrank0(inner) => prank(
            state,
            caller,
            b160_to_h160(data.env.tx.caller),
            inner.0,
            None,
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StartPrank1(inner) => prank(
            state,
            caller,
            b160_to_h160(data.env.tx.caller),
            inner.0,
            Some(inner.1),
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StopPrank(_) => {
            ensure!(state.prank.is_some(), "No prank in progress to stop");
            state.prank = None;
            Bytes::new()
        }
        HEVMCalls::ReadCallers(_) => read_callers(state, b160_to_h160(data.env.tx.caller)),
        HEVMCalls::Record(_) => {
            start_record(state);
            Bytes::new()
        }
        HEVMCalls::Accesses(inner) => accesses(state, inner.0),
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
        HEVMCalls::GetNonce(inner) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;

            // TODO:  this is probably not a good long-term solution since it might mess up the gas
            // calculations
            data.journaled_state.load_account(h160_to_b160(inner.0), data.db)?;

            // we can safely unwrap because `load_account` insert inner.0 to DB.
            let account = data.journaled_state.state().get(&h160_to_b160(inner.0)).unwrap();
            abi::encode(&[Token::Uint(account.info.nonce.into())]).into()
        }
        HEVMCalls::ChainId(inner) => {
            ensure!(inner.0 <= U256::from(u64::MAX), "Chain ID must be less than 2^64 - 1");
            data.env.cfg.chain_id = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::TxGasPrice(inner) => {
            data.env.tx.gas_price = inner.0.into();
            Bytes::new()
        }
        HEVMCalls::Broadcast0(_) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                b160_to_h160(data.env.tx.caller),
                caller,
                b160_to_h160(data.env.tx.caller),
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::Broadcast1(inner) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                inner.0,
                caller,
                b160_to_h160(data.env.tx.caller),
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::Broadcast2(inner) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast_key(
                state,
                inner.0,
                caller,
                b160_to_h160(data.env.tx.caller),
                data.env.cfg.chain_id.into(),
                data.journaled_state.depth(),
                true,
            )?
        }
        HEVMCalls::StartBroadcast0(_) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                b160_to_h160(data.env.tx.caller),
                caller,
                b160_to_h160(data.env.tx.caller),
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StartBroadcast1(inner) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast(
                state,
                inner.0,
                caller,
                b160_to_h160(data.env.tx.caller),
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StartBroadcast2(inner) => {
            correct_sender_nonce(
                b160_to_h160(data.env.tx.caller),
                &mut data.journaled_state,
                &mut data.db,
                state,
            )?;
            broadcast_key(
                state,
                inner.0,
                caller,
                b160_to_h160(data.env.tx.caller),
                data.env.cfg.chain_id.into(),
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
    if !state.corrected_nonce && sender != Config::DEFAULT_SENDER {
        with_journaled_account(journaled_state, db, sender, |account| {
            account.info.nonce = account.info.nonce.saturating_sub(1);
            state.corrected_nonce = true;
        })?;
    }
    Ok(())
}
