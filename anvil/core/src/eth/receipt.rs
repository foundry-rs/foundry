use crate::eth::utils::enveloped;
use ethers_core::{
    types::{Address, Bloom, Bytes, H256, U256},
    utils::{
        rlp,
        rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream},
    },
};
use foundry_evm::revm;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Log {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: Bytes,
}

impl From<revm::Log> for Log {
    fn from(log: revm::Log) -> Self {
        let revm::Log { address, topics, data } = log;
        Log { address, topics, data: data.into() }
    }
}

impl From<Log> for revm::Log {
    fn from(log: Log) -> Self {
        let Log { address, topics, data } = log;
        revm::Log { address, topics, data: data.0 }
    }
}

impl Encodable for Log {
    fn rlp_append(&self, stream: &mut rlp::RlpStream) {
        stream.begin_list(3);
        stream.append(&self.address);
        stream.append_list(&self.topics);
        stream.append(&self.data.as_ref());
    }
}

impl Decodable for Log {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let result = Log {
            address: rlp.val_at(0)?,
            topics: rlp.list_at(1)?,
            data: rlp.val_at::<Vec<u8>>(2)?.into(),
        };
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EIP658Receipt {
    pub status_code: u8,
    pub gas_used: U256,
    pub logs_bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Encodable for EIP658Receipt {
    fn rlp_append(&self, stream: &mut RlpStream) {
        stream.begin_list(4);
        stream.append(&self.status_code);
        stream.append(&self.gas_used);
        stream.append(&self.logs_bloom);
        stream.append_list(&self.logs);
    }
}

impl Decodable for EIP658Receipt {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let result = EIP658Receipt {
            status_code: rlp.val_at(0)?,
            gas_used: rlp.val_at(1)?,
            logs_bloom: rlp.val_at(2)?,
            logs: rlp.list_at(3)?,
        };
        Ok(result)
    }
}

// same underlying data structure
pub type EIP2930Receipt = EIP658Receipt;
pub type EIP1559Receipt = EIP658Receipt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedReceipt {
    /// Legacy receipt
    Legacy(EIP658Receipt),
    /// EIP-2930 receipt
    EIP2930(EIP2930Receipt),
    /// EIP-1559 receipt
    EIP1559(EIP1559Receipt),
}

// == impl TypedReceipt ==

impl TypedReceipt {
    /// Returns the gas used by the transactions
    pub fn gas_used(&self) -> U256 {
        match self {
            TypedReceipt::Legacy(r) | TypedReceipt::EIP2930(r) | TypedReceipt::EIP1559(r) => {
                r.gas_used
            }
        }
    }

    /// Returns the gas used by the transactions
    pub fn logs_bloom(&self) -> &Bloom {
        match self {
            TypedReceipt::Legacy(r) | TypedReceipt::EIP2930(r) | TypedReceipt::EIP1559(r) => {
                &r.logs_bloom
            }
        }
    }
}

impl Encodable for TypedReceipt {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            TypedReceipt::Legacy(r) => r.rlp_append(s),
            TypedReceipt::EIP2930(r) => enveloped(1, r, s),
            TypedReceipt::EIP1559(r) => enveloped(2, r, s),
        }
    }
}

impl Decodable for TypedReceipt {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let slice = rlp.data()?;

        let first = *slice.first().ok_or(DecoderError::Custom("empty receipt"))?;

        if rlp.is_list() {
            return Ok(TypedReceipt::Legacy(Decodable::decode(rlp)?))
        }

        let s = slice.get(1..).ok_or(DecoderError::Custom("no receipt content"))?;

        if first == 0x01 {
            return rlp::decode(s).map(TypedReceipt::EIP2930)
        }

        if first == 0x02 {
            return rlp::decode(s).map(TypedReceipt::EIP1559)
        }

        Err(DecoderError::Custom("unknown receipt type"))
    }
}

impl From<TypedReceipt> for EIP658Receipt {
    fn from(v3: TypedReceipt) -> Self {
        match v3 {
            TypedReceipt::Legacy(receipt) => receipt,
            TypedReceipt::EIP2930(receipt) => receipt,
            TypedReceipt::EIP1559(receipt) => receipt,
        }
    }
}
