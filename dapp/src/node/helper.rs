use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{
        transaction::eip2718::TypedTransaction, Block, NameOrAddress, Signer, Transaction, TxHash,
        Wallet, U256,
    },
    utils::keccak256,
};
use evm_adapters::Evm;

use super::methods::EthResponse;

pub struct State<E> {
    pub(crate) evm: Arc<RwLock<E>>,
    pub(crate) sender: Wallet<SigningKey>,
    pub(crate) blocks: Vec<Block<TxHash>>,
    pub(crate) txs: HashMap<TxHash, Transaction>,
}

pub type SharedState<E> = Arc<RwLock<State<E>>>;

pub fn send_transaction<E, S>(state: SharedState<E>, tx: TypedTransaction) -> EthResponse
where
    E: Evm<S>,
{
    let from = if let Some(from) = tx.from() {
        if state.read().unwrap().sender.address().ne(from) {
            unimplemented!("handle: tx.from != node.sender");
        } else {
            *from
        }
    } else {
        state.read().unwrap().sender.address()
    };
    let value = *tx.value().unwrap_or(&U256::zero());
    let calldata = match tx.data() {
        Some(data) => data.to_vec(),
        None => vec![],
    };
    let to = tx.to().unwrap();

    let to = match to {
        NameOrAddress::Address(addr) => *addr,
        NameOrAddress::Name(_) => unimplemented!("handle: tx.to is an ENS name"),
    };
    // FIXME(rohit): state (and node) must need the chainID
    let tx_hash =
        keccak256(tx.rlp_signed(1, &state.read().unwrap().sender.sign_transaction_sync(&tx)));

    match state.write().unwrap().evm.write().unwrap().call_raw(
        from,
        to,
        calldata.into(),
        value,
        false,
    ) {
        Ok((retdata, status, _gas_used)) => {
            if E::is_success(&status) {
                EthResponse::EthSendTransaction(Ok(tx_hash.into()))
            } else {
                EthResponse::EthSendTransaction(Err(Box::new(
                    dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default(),
                )))
            }
        }
        Err(e) => EthResponse::EthSendTransaction(Err(Box::new(e.to_string()))),
    }
}

pub fn deploy_contract<E, S>(state: SharedState<E>, tx: TypedTransaction) -> EthResponse
where
    E: Evm<S>,
{
    let from = if let Some(from) = tx.from() {
        if state.read().unwrap().sender.address().ne(from) {
            unimplemented!("handle: tx.from != node.sender");
        } else {
            *from
        }
    } else {
        state.read().unwrap().sender.address()
    };
    let value = *tx.value().unwrap_or(&U256::zero());
    let bytecode = match tx.data() {
        Some(data) => data.to_vec(),
        None => vec![],
    };
    let tx_hash =
        keccak256(tx.rlp_signed(1, &state.read().unwrap().sender.sign_transaction_sync(&tx)));
    match state.write().unwrap().evm.write().unwrap().deploy(from, bytecode.into(), value) {
        Ok((retdata, status, _gas_used)) => {
            if E::is_success(&status) {
                EthResponse::EthSendTransaction(Ok(tx_hash.into()))
            } else {
                EthResponse::EthSendTransaction(Err(Box::new(
                    dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default(),
                )))
            }
        }
        Err(e) => EthResponse::EthSendTransaction(Err(Box::new(e.to_string()))),
    }
}
