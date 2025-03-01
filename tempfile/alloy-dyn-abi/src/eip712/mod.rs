//! Implementation of dynamic EIP-712.
//!
//! This allows for the encoding of EIP-712 messages without having to know the
//! types at compile time. This is useful for things like off-chain signing.
//! It implements the encoding portion of the EIP-712 spec, and does not
//! contain any of the signing logic.
//!
//! <https://eips.ethereum.org/EIPS/eip-712#specification-of-the-eth_signtypeddata-json-rpc>

pub mod parser;

mod typed_data;
pub use typed_data::{Eip712Types, TypedData};

mod resolver;
pub use resolver::{PropertyDef, Resolver, TypeDef};

pub(crate) mod coerce;

#[cfg(test)]
mod test {
    use super::*;
    use alloy_primitives::B256;
    use alloy_sol_types::SolStruct;

    #[test]
    fn repro_i128() {
        alloy_sol_types::sol! {
            #[derive(serde::Serialize)]
            struct Order {
                bytes32 sender;
                int128 priceX18;
                int128 amount;
                uint64 expiration;
                uint64 nonce;
            }
        }

        let msg = Order {
            sender: B256::repeat_byte(3),
            priceX18: -1000000000000000,
            amount: 2,
            expiration: 3,
            nonce: 4,
        };

        let domain = Default::default();

        let static_sh = msg.eip712_signing_hash(&domain);

        let fromst = TypedData::from_struct(&msg, Some(domain));
        let dyn_sh = fromst.eip712_signing_hash();
        assert_eq!(static_sh, dyn_sh.unwrap());
    }
}
