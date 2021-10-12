use std::collections::HashMap;

use ethers::{
    prelude::{
        transaction::eip2718::TypedTransaction, Address, Block, NameOrAddress, Signer, Transaction,
        TransactionReceipt, TxHash, H256, U256, U64,
    },
    utils::keccak256,
};
use evm_adapters::Evm;

use super::{methods::EthResponse, SharedNode};

#[derive(Default)]
pub struct Blockchain {
    pub(crate) blocks_by_number: HashMap<U64, Block<TxHash>>,
    pub(crate) blocks_by_hash: HashMap<H256, Block<TxHash>>,
    pub(crate) txs: HashMap<TxHash, Transaction>,
    pub(crate) tx_receipts: HashMap<TxHash, TransactionReceipt>,
}

////////////////////////////////////// GETTERS ///////////////////////////////////////////////

pub fn get_balance<E, S>(node: SharedNode<E>, account: Address) -> U256
where
    E: Evm<S>,
{
    node.read().unwrap().evm.get_balance(account)
}

pub fn get_block_by_number<E>(node: SharedNode<E>, number: U64) -> Option<Block<TxHash>> {
    node.read().unwrap().blockchain.blocks_by_number.get(&number).cloned()
}

pub fn get_block_by_hash<E>(node: SharedNode<E>, hash: H256) -> Option<Block<TxHash>> {
    node.read().unwrap().blockchain.blocks_by_hash.get(&hash).cloned()
}

pub fn get_transaction<E>(node: SharedNode<E>, tx_hash: TxHash) -> Option<Transaction> {
    node.read().unwrap().blockchain.txs.get(&tx_hash).cloned()
}

pub fn get_tx_receipt<E>(node: SharedNode<E>, tx_hash: TxHash) -> Option<TransactionReceipt> {
    node.read().unwrap().blockchain.tx_receipts.get(&tx_hash).cloned()
}

////////////////////////////////////// SETTERS ///////////////////////////////////////////////

pub fn send_transaction<E, S>(node: SharedNode<E>, tx: TypedTransaction) -> EthResponse
where
    E: Evm<S> + Send + Sync + 'static,
{
    let sender = if let Some(from) = tx.from() {
        if let Some(sender) = node.read().unwrap().account(*from) {
            sender
        } else {
            unimplemented!("handle: tx.from != node.sender");
        }
    } else {
        node.read().unwrap().default_sender()
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
    // FIXME(rohit): node.(and node) must need the chainID
    let tx_hash = keccak256(tx.rlp_signed(1, &sender.sign_transaction_sync(&tx)));

    match node.write().unwrap().evm.call_raw(sender.address(), to, calldata.into(), value, false) {
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

pub fn deploy_contract<E, S>(node: SharedNode<E>, tx: TypedTransaction) -> EthResponse
where
    E: Evm<S> + Send + Sync + 'static,
{
    let sender = if let Some(from) = tx.from() {
        if let Some(sender) = node.read().unwrap().account(*from) {
            sender
        } else {
            unimplemented!("handle: tx.from != node.sender");
        }
    } else {
        node.read().unwrap().default_sender()
    };
    let value = *tx.value().unwrap_or(&U256::zero());
    let bytecode = match tx.data() {
        Some(data) => data.to_vec(),
        None => vec![],
    };
    let tx_hash = keccak256(tx.rlp_signed(1, &sender.sign_transaction_sync(&tx)));
    match node.write().unwrap().evm.deploy(sender.address(), bytecode.into(), value) {
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

pub fn add_block<E>(node: SharedNode<E>, block: Block<TxHash>) {
    node.write()
        .unwrap()
        .blockchain
        .blocks_by_number
        .insert(block.number.expect("pending block cannot be added"), block.clone());
    node.write()
        .unwrap()
        .blockchain
        .blocks_by_hash
        .insert(block.hash.expect("pending block cannot be added"), block);
}

pub fn add_transaction<E>(node: SharedNode<E>, tx: Transaction) {
    node.write().unwrap().blockchain.txs.insert(tx.hash(), tx);
}

pub fn add_tx_receipt<E>(node: SharedNode<E>, tx_receipt: TransactionReceipt) {
    node.write().unwrap().blockchain.tx_receipts.insert(tx_receipt.transaction_hash, tx_receipt);
}
