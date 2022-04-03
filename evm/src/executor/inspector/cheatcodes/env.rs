use std::collections::BTreeMap;

use super::Cheatcodes;
use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::{AbiEncode, Token, Tokenize},
    types::{Address, H256, U256},
    utils::keccak256,
};
use revm::{Database, EVMData};

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
    /// Whether or not the prank stops by itself after the next call
    pub single_call: bool,
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
        return Err("You have an active prank already.".to_string().encode().into())
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

pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    caller: Address,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Warp(inner) => {
            data.env.block.timestamp = inner.0;
            Ok(Bytes::new())
        }
        HEVMCalls::Roll(inner) => {
            data.env.block.number = inner.0;
            Ok(Bytes::new())
        }
        HEVMCalls::Fee(inner) => {
            data.env.block.basefee = inner.0;
            Ok(Bytes::new())
        }
        HEVMCalls::Store(inner) => {
            // TODO: Does this increase gas usage?
            data.subroutine.load_account(inner.0, data.db);
            data.subroutine.sstore(inner.0, inner.1.into(), inner.2.into(), data.db);
            Ok(Bytes::new())
        }
        HEVMCalls::Load(inner) => {
            // TODO: Does this increase gas usage?
            data.subroutine.load_account(inner.0, data.db);
            let (val, _) = data.subroutine.sload(inner.0, inner.1.into(), data.db);
            Ok(val.encode().into())
        }
        HEVMCalls::Etch(inner) => {
            let code = inner.1.clone();
            let hash = H256::from_slice(&keccak256(&code));

            // TODO: Does this increase gas usage?
            data.subroutine.load_account(inner.0, data.db);
            data.subroutine.set_code(inner.0, code.0, hash);
            Ok(Bytes::new())
        }
        HEVMCalls::Deal(inner) => {
            let who = inner.0;
            let value = inner.1;

            // TODO: Does this increase gas usage?
            data.subroutine.load_account(who, data.db);
            let balance = data.subroutine.account(inner.0).info.balance;

            // TODO: We should probably upstream a `set_balance` function
            if balance < value {
                data.subroutine.balance_add(who, value - balance);
            } else {
                data.subroutine.balance_sub(who, balance - value);
            }
            Ok(Bytes::new())
        }
        HEVMCalls::Prank0(inner) => {
            prank(state, caller, data.env.tx.caller, inner.0, None, data.subroutine.depth(), true)
        }
        HEVMCalls::Prank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            Some(inner.1),
            data.subroutine.depth(),
            true,
        ),
        HEVMCalls::StartPrank0(inner) => {
            prank(state, caller, data.env.tx.caller, inner.0, None, data.subroutine.depth(), false)
        }
        HEVMCalls::StartPrank1(inner) => prank(
            state,
            caller,
            data.env.tx.caller,
            inner.0,
            Some(inner.1),
            data.subroutine.depth(),
            false,
        ),
        HEVMCalls::StopPrank(_) => {
            state.prank = None;
            Ok(Bytes::new())
        }
        HEVMCalls::Record(_) => {
            start_record(state);
            Ok(Bytes::new())
        }
        HEVMCalls::Accesses(inner) => Ok(accesses(state, inner.0)),
        _ => return None,
    })
}
