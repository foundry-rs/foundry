//! OP-stack-specific impls for [`FoundryTxEnvelope`] and [`FoundryTransactionRequest`].

use super::{FoundryTransactionRequest, FoundryTxEnvelope};
use alloy_consensus::Sealed;
use op_alloy_consensus::{
    OpTransaction as OpTransactionTrait, OpTxEnvelope, TxDeposit, TxPostExec,
};

impl OpTransactionTrait for FoundryTxEnvelope {
    fn is_deposit(&self) -> bool {
        Self::is_deposit(self)
    }

    fn as_deposit(&self) -> Option<&Sealed<TxDeposit>> {
        match self {
            Self::Deposit(tx) => Some(tx),
            _ => None,
        }
    }

    fn as_post_exec(&self) -> Option<&Sealed<TxPostExec>> {
        if let Self::PostExec(tx) = self { Some(tx) } else { None }
    }
}

impl From<OpTxEnvelope> for FoundryTxEnvelope {
    fn from(tx: OpTxEnvelope) -> Self {
        match tx {
            OpTxEnvelope::Legacy(tx) => Self::Legacy(tx),
            OpTxEnvelope::Eip2930(tx) => Self::Eip2930(tx),
            OpTxEnvelope::Eip1559(tx) => Self::Eip1559(tx),
            OpTxEnvelope::Eip7702(tx) => Self::Eip7702(tx),
            OpTxEnvelope::Deposit(tx) => Self::Deposit(tx),
            OpTxEnvelope::PostExec(tx) => Self::PostExec(tx),
        }
    }
}

impl From<op_alloy_rpc_types::Transaction<FoundryTxEnvelope>> for FoundryTransactionRequest {
    fn from(tx: op_alloy_rpc_types::Transaction<FoundryTxEnvelope>) -> Self {
        tx.inner.into_inner().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::eip2718::Encodable2718;
    use alloy_primitives::{TxHash, b256};
    use alloy_rlp::Decodable;

    #[test]
    fn test_decode_encode_deposit_tx() {
        // https://sepolia-optimism.etherscan.io/tx/0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let tx_hash: TxHash =
            b256!("0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7");

        // https://sepolia-optimism.etherscan.io/getRawTx?tx=0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let raw_tx = alloy_primitives::hex::decode(
            "7ef861a0dfd7ae78bf3c414cfaa77f13c0205c82eb9365e217b2daa3448c3156b69b27ac94778f2146f48179643473b82931c4cd7b8f153efd94778f2146f48179643473b82931c4cd7b8f153efd872386f26fc10000872386f26fc10000830186a08080",
        )
        .unwrap();
        let dep_tx = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        let mut encoded = Vec::new();
        dep_tx.encode_2718(&mut encoded);

        assert_eq!(raw_tx, encoded);

        assert_eq!(tx_hash, dep_tx.hash());
    }
}
