use std::collections::BTreeMap;

use super::Cheatcodes;
use crate::{
    abi::HEVMCalls,
    error::SolError,
    executor::{
        backend::DatabaseExt,
        inspector::cheatcodes::{util::with_journaled_account, DealRecord},
    },
};
use bytes::Bytes;
use ethers::{
    abi::{self, AbiEncode, RawLog, Token, Tokenizable, Tokenize},
    prelude::k256::{
        ecdsa::SigningKey,
        elliptic_curve::{bigint::Encoding, Curve},
        Secp256k1,
    },
    signers::{LocalWallet, Signer},
    types::{Address, U256},
};
use foundry_config::Config;
use revm::{Bytecode, Database, EVMData};
use tracing::trace;

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
}

/// Sets up broadcasting from a script using `origin` as the sender
fn broadcast(
    state: &mut Cheatcodes,
    new_origin: Address,
    original_caller: Address,
    original_origin: Address,
    depth: u64,
    single_call: bool,
) -> Result<Bytes, Bytes> {
    let broadcast = Broadcast { new_origin, original_origin, original_caller, depth, single_call };

    if state.prank.is_some() {
        return Err("You have an active prank. Broadcasting and pranks are not compatible. Disable one or the other".to_string().encode().into());
    }

    if state.broadcast.is_some() {
        return Err("You have an active broadcast already.".to_string().encode().into())
    }

    state.broadcast = Some(broadcast);
    Ok(Bytes::new())
}

/// Sets up broadcasting from a script with the sender derived from `private_key`
/// Adds this private key to `state`'s `script_wallets` vector to later be used for signing
fn broadcast_key(
    state: &mut Cheatcodes,
    private_key: U256,
    original_caller: Address,
    original_origin: Address,
    chain_id: U256,
    depth: u64,
    single_call: bool,
) -> Result<Bytes, Bytes> {
    if private_key.is_zero() {
        return Err("Private key cannot be 0.".to_string().encode().into())
    }

    if private_key >= U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()) {
        return Err("Private key must be less than 115792089237316195423570985008687907852837564279074904382605163141518161494337 (the secp256k1 curve order).".to_string().encode().into());
    }

    let mut bytes: [u8; 32] = [0; 32];
    private_key.to_big_endian(&mut bytes);

    let key = SigningKey::from_bytes((&bytes).into()).map_err(|err| err.to_string().encode())?;
    let wallet = LocalWallet::from(key).with_chain_id(chain_id.as_u64());

    let new_origin = wallet.address();

    state.script_wallets.push(wallet);

    broadcast(state, new_origin, original_caller, original_origin, depth, single_call)
}

fn prank(
    state: &mut Cheatcodes,
    prank_caller: Address,
    prank_origin: Address,
    new_caller: Address,
    new_origin: Option<Address>,
    depth: u64,
    single_call: bool,
) -> Result<Bytes, Bytes> {
    let prank = Prank { prank_caller, prank_origin, new_caller, new_origin, depth, single_call };

    if state.prank.is_some() {
        return Err("You have an active prank already.".encode().into())
    }

    if state.broadcast.is_some() {
        return Err("You cannot `prank` for a broadcasted transaction. Pass the desired tx.origin into the broadcast cheatcode call".encode().into());
    }

    state.prank = Some(prank);
    Ok(Bytes::new())
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
        ethers::abi::encode(&[
            storage_accesses.reads.remove(&address).unwrap_or_default().into_tokens()[0].clone(),
            storage_accesses.writes.remove(&address).unwrap_or_default().into_tokens()[0].clone(),
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

pub fn apply<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    caller: Address,
    call: &HEVMCalls,
) -> Result<Option<Bytes>, Bytes> {
    let res = match call {
        HEVMCalls::Warp(inner) => {
            data.env.block.timestamp = inner.0;
            Bytes::new()
        }
        HEVMCalls::Difficulty(inner) => {
            data.env.block.difficulty = inner.0;
            Bytes::new()
        }
        HEVMCalls::Roll(inner) => {
            data.env.block.number = inner.0;
            Bytes::new()
        }
        HEVMCalls::Fee(inner) => {
            data.env.block.basefee = inner.0;
            Bytes::new()
        }
        HEVMCalls::Coinbase(inner) => {
            data.env.block.coinbase = inner.0;
            Bytes::new()
        }
        HEVMCalls::Store(inner) => {
            data.journaled_state
                .load_account(inner.0, data.db)
                .map_err(|err| err.encode_string())?;
            // ensure the account is touched
            data.journaled_state.touch(&inner.0);

            data.journaled_state
                .sstore(inner.0, inner.1.into(), inner.2.into(), data.db)
                .map_err(|err| err.encode_string())?;
            Bytes::new()
        }
        HEVMCalls::Load(inner) => {
            // TODO: Does this increase gas usage?
            data.journaled_state
                .load_account(inner.0, data.db)
                .map_err(|err| err.encode_string())?;
            let (val, _) = data
                .journaled_state
                .sload(inner.0, inner.1.into(), data.db)
                .map_err(|err| err.encode_string())?;
            val.encode().into()
        }
        HEVMCalls::Etch(inner) => {
            let code = inner.1.clone();
            trace!(address=?inner.0, code=?hex::encode(&code.0), "etch cheatcode");
            // TODO: Does this increase gas usage?
            data.journaled_state
                .load_account(inner.0, data.db)
                .map_err(|err| err.encode_string())?;
            data.journaled_state.set_code(inner.0, Bytecode::new_raw(code.0).to_checked());
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
                    old_balance: account.info.balance,
                    new_balance: value,
                };
                state.eth_deals.push(record);

                account.info.balance = value;
            })
            .map_err(|err| err.encode_string())?;
            Bytes::new()
        }
        HEVMCalls::Prank0(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            None,
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::Prank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            Some(inner.1),
            data.journaled_state.depth(),
            true,
        )?,
        HEVMCalls::StartPrank0(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            None,
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StartPrank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            Some(inner.1),
            data.journaled_state.depth(),
            false,
        )?,
        HEVMCalls::StopPrank(_) => {
            state.prank = None;
            Bytes::new()
        }
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
            with_journaled_account(&mut data.journaled_state, data.db, inner.0, |account| -> Result<Bytes, Bytes>{
                // nonce must increment only
                if account.info.nonce < inner.1 {
                    account.info.nonce = inner.1;
                    Ok(Bytes::new())
                } else {
                    Err(format!("Nonce lower than account's current nonce. Please provide a higher nonce than {}", account.info.nonce).encode().into())
                }
            }).map_err(|err| err.encode_string())??
        }
        HEVMCalls::GetNonce(inner) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )
            .map_err(|err| err.encode_string())?;

            // TODO:  this is probably not a good long-term solution since it might mess up the gas
            // calculations
            data.journaled_state
                .load_account(inner.0, data.db)
                .map_err(|err| err.encode_string())?;

            // we can safely unwrap because `load_account` insert inner.0 to DB.
            let account = data.journaled_state.state().get(&inner.0).unwrap();
            abi::encode(&[Token::Uint(account.info.nonce.into())]).into()
        }
        HEVMCalls::ChainId(inner) => {
            if inner.0 > U256::from(u64::MAX) {
                return Err("Chain ID must be less than 2^64".to_string().encode().into())
            }
            data.env.cfg.chain_id = inner.0;
            Bytes::new()
        }
        HEVMCalls::GasPrice(inner) => {
            data.env.tx.gas_price = inner.0;
            Bytes::new()
        }
        HEVMCalls::Broadcast0(_) => {
            correct_sender_nonce(
                data.env.tx.caller,
                &mut data.journaled_state,
                &mut data.db,
                state,
            )
            .map_err(|err| err.encode_string())?;
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
            )
            .map_err(|err| err.encode_string())?;
            broadcast(
                state,
                inner.0,
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
            )
            .map_err(|err| err.encode_string())?;
            broadcast_key(
                state,
                inner.0,
                caller,
                data.env.tx.caller,
                data.env.cfg.chain_id,
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
            )
            .map_err(|err| err.encode_string())?;
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
            )
            .map_err(|err| err.encode_string())?;
            broadcast(
                state,
                inner.0,
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
            )
            .map_err(|err| err.encode_string())?;
            broadcast_key(
                state,
                inner.0,
                caller,
                data.env.tx.caller,
                data.env.cfg.chain_id,
                data.journaled_state.depth(),
                false,
            )?
        }
        HEVMCalls::StopBroadcast(_) => {
            if state.broadcast.is_none() {
                return Err("No broadcast in progress to stop".to_string().encode().into())
            }
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

    Ok(Some(res))
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
