//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_json_abi::{Event, EventParam, Function, InternalType, Param, StateMutability};
use alloy_primitives::{Address, Bloom, Bytes, B256, B64, I256, U128, U256, U64};
use alloy_rpc_types::{AccessList, AccessListItem, CallInput, CallRequest, Signature, Transaction};
use ethers_core::{
    abi as ethabi,
    types::{
        transaction::eip2930::{
            AccessList as EthersAccessList, AccessListItem as EthersAccessListItem,
        },
        Bloom as EthersBloom, Bytes as EthersBytes, TransactionRequest, H160, H256, H64,
        I256 as EthersI256, U256 as EthersU256, U64 as EthersU64,
    },
};

/// Conversion trait to easily convert from Ethers types to Alloy types.
pub trait ToAlloy {
    /// The corresponding Alloy type.
    type To;

    /// Converts the Ethers type to the corresponding Alloy type.
    fn to_alloy(self) -> Self::To;
}

/// Conversion trait to easily convert from Alloy tracing types to Reth tracing types.
pub trait ToReth {
    /// The corresponding Reth type.
    type To;

    /// Converts the Alloy type to the corresponding Reth type.
    fn to_reth(self) -> Self::To;
}

impl ToAlloy for EthersBytes {
    type To = Bytes;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Bytes(self.0)
    }
}

impl ToAlloy for H64 {
    type To = B64;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        B64::new(self.0)
    }
}

impl ToAlloy for H160 {
    type To = Address;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Address::new(self.0)
    }
}

impl ToAlloy for H256 {
    type To = B256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        B256::new(self.0)
    }
}

impl ToAlloy for EthersBloom {
    type To = Bloom;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Bloom::new(self.0)
    }
}

impl ToAlloy for EthersU256 {
    type To = U256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U256::from_limbs(self.0)
    }
}

impl ToAlloy for EthersI256 {
    type To = I256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        I256::from_raw(self.into_raw().to_alloy())
    }
}

impl ToAlloy for EthersU64 {
    type To = U64;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U64::from_limbs(self.0)
    }
}

impl ToAlloy for u64 {
    type To = U256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U256::from(self)
    }
}

impl ToAlloy for ethers_core::types::Transaction {
    type To = Transaction;

    fn to_alloy(self) -> Self::To {
        Transaction {
            hash: self.hash.to_alloy(),
            nonce: U64::from(self.nonce.as_u64()),
            block_hash: self.block_hash.map(ToAlloy::to_alloy),
            block_number: self.block_number.map(|b| U256::from(b.as_u64())),
            transaction_index: self.transaction_index.map(|b| U256::from(b.as_u64())),
            from: self.from.to_alloy(),
            to: self.to.map(ToAlloy::to_alloy),
            value: self.value.to_alloy(),
            gas_price: self.gas_price.map(|a| U128::from(a.as_u128())),
            gas: self.gas.to_alloy(),
            max_fee_per_gas: self.max_fee_per_gas.map(|f| U128::from(f.as_u128())),
            max_priority_fee_per_gas: self
                .max_priority_fee_per_gas
                .map(|f| U128::from(f.as_u128())),
            max_fee_per_blob_gas: None,
            input: self.input.0.into(),
            signature: Some(Signature {
                r: self.r.to_alloy(),
                s: self.s.to_alloy(),
                v: U256::from(self.v.as_u64()),
                y_parity: None,
            }),
            chain_id: self.chain_id.map(|c| U64::from(c.as_u64())),
            blob_versioned_hashes: Vec::new(),
            access_list: self.access_list.map(|a| a.0.into_iter().map(ToAlloy::to_alloy).collect()),
            transaction_type: self.transaction_type.map(|t| t.to_alloy()),
            other: Default::default(),
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::LocalizedTransactionTrace {
    type To = reth_rpc_types::trace::parity::LocalizedTransactionTrace;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::LocalizedTransactionTrace {
            trace: self.trace.to_reth(),
            block_hash: self.block_hash,
            block_number: self.block_number,
            transaction_hash: self.transaction_hash,
            transaction_position: self.transaction_position,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::TransactionTrace {
    type To = reth_rpc_types::trace::parity::TransactionTrace;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::TransactionTrace {
            action: self.action.to_reth(),
            error: self.error,
            result: self.result.map(ToReth::to_reth),
            subtraces: self.subtraces,
            trace_address: self.trace_address,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::TraceOutput {
    type To = reth_rpc_types::trace::parity::TraceOutput;

    fn to_reth(self) -> Self::To {
        match self {
            alloy_rpc_types::trace::parity::TraceOutput::Call(call_output) => {
                reth_rpc_types::trace::parity::TraceOutput::Call(call_output.to_reth())
            }
            alloy_rpc_types::trace::parity::TraceOutput::Create(create_output) => {
                reth_rpc_types::trace::parity::TraceOutput::Create(create_output.to_reth())
            }
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::CallOutput {
    type To = reth_rpc_types::trace::parity::CallOutput;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::CallOutput { gas_used: self.gas_used, output: self.output }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::CreateOutput {
    type To = reth_rpc_types::trace::parity::CreateOutput;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::CreateOutput {
            gas_used: self.gas_used,
            code: self.code,
            address: self.address,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::Action {
    type To = reth_rpc_types::trace::parity::Action;

    fn to_reth(self) -> Self::To {
        match self {
            alloy_rpc_types::trace::parity::Action::Call(call_action) => {
                reth_rpc_types::trace::parity::Action::Call(call_action.to_reth())
            }
            alloy_rpc_types::trace::parity::Action::Create(create_action) => {
                reth_rpc_types::trace::parity::Action::Create(create_action.to_reth())
            }
            alloy_rpc_types::trace::parity::Action::Selfdestruct(self_destruct_action) => {
                reth_rpc_types::trace::parity::Action::Selfdestruct(self_destruct_action.to_reth())
            }
            alloy_rpc_types::trace::parity::Action::Reward(reward_action) => {
                reth_rpc_types::trace::parity::Action::Reward(reward_action.to_reth())
            }
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::CallType {
    type To = reth_rpc_types::trace::parity::CallType;

    fn to_reth(self) -> Self::To {
        match self {
            alloy_rpc_types::trace::parity::CallType::Call => {
                reth_rpc_types::trace::parity::CallType::Call
            }
            alloy_rpc_types::trace::parity::CallType::DelegateCall => {
                reth_rpc_types::trace::parity::CallType::DelegateCall
            }
            alloy_rpc_types::trace::parity::CallType::StaticCall => {
                reth_rpc_types::trace::parity::CallType::StaticCall
            }
            alloy_rpc_types::trace::parity::CallType::CallCode => {
                reth_rpc_types::trace::parity::CallType::CallCode
            }
            alloy_rpc_types::trace::CallType::None => reth_rpc_types::trace::parity::CallType::None,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::CallAction {
    type To = reth_rpc_types::trace::parity::CallAction;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::CallAction {
            call_type: self.call_type.to_reth(),
            from: self.from,
            gas: self.gas,
            input: self.input,
            to: self.to,
            value: self.value,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::CreateAction {
    type To = reth_rpc_types::trace::parity::CreateAction;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::CreateAction {
            from: self.from,
            gas: self.gas,
            init: self.init,
            value: self.value,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::SelfdestructAction {
    type To = reth_rpc_types::trace::parity::SelfdestructAction;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::SelfdestructAction {
            refund_address: self.refund_address,
            address: self.address,
            balance: self.balance,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::RewardAction {
    type To = reth_rpc_types::trace::parity::RewardAction;

    fn to_reth(self) -> Self::To {
        reth_rpc_types::trace::parity::RewardAction {
            author: self.author,
            reward_type: self.reward_type.to_reth(),
            value: self.value,
        }
    }
}

impl ToReth for alloy_rpc_types::trace::parity::RewardType {
    type To = reth_rpc_types::trace::parity::RewardType;

    fn to_reth(self) -> Self::To {
        match self {
            alloy_rpc_types::trace::parity::RewardType::Block => {
                reth_rpc_types::trace::parity::RewardType::Block
            }
            alloy_rpc_types::trace::parity::RewardType::Uncle => {
                reth_rpc_types::trace::parity::RewardType::Uncle
            }
        }
    }
}

/// Converts from a [TransactionRequest] to a [CallRequest].
pub fn to_call_request_from_tx_request(tx: TransactionRequest) -> CallRequest {
    CallRequest {
        from: tx.from.map(|f| f.to_alloy()),
        to: match tx.to {
            Some(to) => match to {
                ethers_core::types::NameOrAddress::Address(addr) => Some(addr.to_alloy()),
                ethers_core::types::NameOrAddress::Name(_) => None,
            },
            None => None,
        },
        gas_price: tx.gas_price.map(|g| g.to_alloy()),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        gas: tx.gas.map(|g| g.to_alloy()),
        value: tx.value.map(|v| v.to_alloy()),
        input: CallInput::maybe_input(tx.data.map(|b| b.0.into())),
        nonce: tx.nonce.map(|n| U64::from(n.as_u64())),
        chain_id: tx.chain_id.map(|c| c.to_alloy()),
        access_list: None,
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: None,
        transaction_type: None,
    }
}

impl ToAlloy for EthersAccessList {
    type To = AccessList;
    fn to_alloy(self) -> Self::To {
        AccessList(self.0.into_iter().map(ToAlloy::to_alloy).collect())
    }
}

impl ToAlloy for EthersAccessListItem {
    type To = AccessListItem;

    fn to_alloy(self) -> Self::To {
        AccessListItem {
            address: self.address.to_alloy(),
            storage_keys: self.storage_keys.into_iter().map(ToAlloy::to_alloy).collect(),
        }
    }
}

impl ToAlloy for ethabi::Event {
    type To = Event;

    fn to_alloy(self) -> Self::To {
        Event {
            name: self.name,
            inputs: self.inputs.into_iter().map(ToAlloy::to_alloy).collect(),
            anonymous: self.anonymous,
        }
    }
}

impl ToAlloy for ethabi::Function {
    type To = Function;

    fn to_alloy(self) -> Self::To {
        Function {
            name: self.name,
            inputs: self.inputs.into_iter().map(ToAlloy::to_alloy).collect(),
            outputs: self.outputs.into_iter().map(ToAlloy::to_alloy).collect(),
            state_mutability: self.state_mutability.to_alloy(),
        }
    }
}

impl ToAlloy for ethabi::Param {
    type To = Param;

    fn to_alloy(self) -> Self::To {
        let (ty, components) = self.kind.to_alloy();
        Param {
            name: self.name,
            ty,
            internal_type: self.internal_type.as_deref().and_then(InternalType::parse),
            components,
        }
    }
}

impl ToAlloy for ethabi::EventParam {
    type To = EventParam;

    fn to_alloy(self) -> Self::To {
        let (ty, components) = self.kind.to_alloy();
        EventParam { name: self.name, ty, internal_type: None, components, indexed: self.indexed }
    }
}

impl ToAlloy for ethabi::ParamType {
    type To = (String, Vec<Param>);

    fn to_alloy(self) -> Self::To {
        let (s, t) = split_pt(self);
        (s, t.into_iter().map(pt_to_param).collect())
    }
}

fn split_pt(x: ethabi::ParamType) -> (String, Vec<ethabi::ParamType>) {
    let s = ethabi::ethabi::param_type::Writer::write_for_abi(&x, false);
    let t = get_tuple(x);
    (s, t)
}

fn get_tuple(x: ethabi::ParamType) -> Vec<ethabi::ParamType> {
    match x {
        ethabi::ParamType::FixedArray(x, _) | ethabi::ParamType::Array(x) => get_tuple(*x),
        ethabi::ParamType::Tuple(t) => t,
        _ => Default::default(),
    }
}

fn pt_to_param(x: ethabi::ParamType) -> Param {
    let (ty, components) = split_pt(x);
    Param {
        name: String::new(),
        ty,
        internal_type: None,
        components: components.into_iter().map(pt_to_param).collect(),
    }
}

impl ToAlloy for ethabi::StateMutability {
    type To = StateMutability;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        match self {
            ethabi::StateMutability::Pure => StateMutability::Pure,
            ethabi::StateMutability::View => StateMutability::View,
            ethabi::StateMutability::NonPayable => StateMutability::NonPayable,
            ethabi::StateMutability::Payable => StateMutability::Payable,
        }
    }
}

/// Conversion trait to easily convert from Alloy types to Ethers types.
pub trait ToEthers {
    /// The corresponding Ethers type.
    type To;

    /// Converts the Alloy type to the corresponding Ethers type.
    fn to_ethers(self) -> Self::To;
}

impl ToEthers for Address {
    type To = H160;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        H160(self.0 .0)
    }
}

impl ToEthers for B256 {
    type To = H256;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        H256(self.0)
    }
}

impl ToEthers for U256 {
    type To = EthersU256;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersU256(self.into_limbs())
    }
}

impl ToEthers for U64 {
    type To = EthersU64;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersU64(self.into_limbs())
    }
}

impl ToEthers for Bytes {
    type To = EthersBytes;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersBytes(self.0)
    }
}
