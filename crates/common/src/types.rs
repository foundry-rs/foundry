//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_json_abi::{Event, EventParam, Function, InternalType, Param, StateMutability};
use alloy_primitives::{Address, Bloom, Bytes, B256, B64, I256, U256, U64};
use ethers_core::{
    abi as ethabi,
    types::{
        Bloom as EthersBloom, Bytes as EthersBytes, H160, H256, H64, I256 as EthersI256,
        U256 as EthersU256, U64 as EthersU64,
    },
};

/// Conversion trait to easily convert from Ethers types to Alloy types.
pub trait ToAlloy {
    /// The corresponding Alloy type.
    type To;

    /// Converts the Ethers type to the corresponding Alloy type.
    fn to_alloy(self) -> Self::To;
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
