pub use crate::ic::*;
use alloy_primitives::{address, Address, Bytes, B256};
use revm::{
    context::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{Gas, InstructionResult, InterpreterResult},
    precompile::{
        secp256r1::p256_verify as revm_p256_verify, PrecompileError, PrecompileResult,
        PrecompileWithAddress,
    },
};

/// The ECRecover precompile address.
pub const EC_RECOVER: Address = address!("0000000000000000000000000000000000000001");

/// The SHA-256 precompile address.
pub const SHA_256: Address = address!("0000000000000000000000000000000000000002");

/// The RIPEMD-160 precompile address.
pub const RIPEMD_160: Address = address!("0000000000000000000000000000000000000003");

/// The Identity precompile address.
pub const IDENTITY: Address = address!("0000000000000000000000000000000000000004");

/// The ModExp precompile address.
pub const MOD_EXP: Address = address!("0000000000000000000000000000000000000005");

/// The ECAdd precompile address.
pub const EC_ADD: Address = address!("0000000000000000000000000000000000000006");

/// The ECMul precompile address.
pub const EC_MUL: Address = address!("0000000000000000000000000000000000000007");

/// The ECPairing precompile address.
pub const EC_PAIRING: Address = address!("0000000000000000000000000000000000000008");

/// The Blake2F precompile address.
pub const BLAKE_2F: Address = address!("0000000000000000000000000000000000000009");

/// The PointEvaluation precompile address.
pub const POINT_EVALUATION: Address = address!("000000000000000000000000000000000000000a");

/// Precompile addresses.
pub const PRECOMPILES: &[Address] = &[
    EC_RECOVER,
    SHA_256,
    RIPEMD_160,
    IDENTITY,
    MOD_EXP,
    EC_ADD,
    EC_MUL,
    EC_PAIRING,
    BLAKE_2F,
    POINT_EVALUATION,
    ODYSSEY_P256_ADDRESS,
];

/// [RIP-7212](https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md) secp256r1 precompile address on Odyssey.
///
/// <https://github.com/ithacaxyz/odyssey/blob/482f4547631ae5c64ebea6a4b4ef93184a4abfee/crates/node/src/evm.rs#L35-L35>
pub const ODYSSEY_P256_ADDRESS: Address = address!("0000000000000000000000000000000000000014");

/// Wrapper around revm P256 precompile, matching EIP-7212 spec.
///
/// Per Optimism implementation, P256 precompile returns empty bytes on failure, but per EIP-7212 it
/// should be 32 bytes of zeros instead.
pub fn p256_verify(input: &Bytes, gas_limit: u64) -> PrecompileResult {
    revm_p256_verify(input, gas_limit).map(|mut result| {
        if result.bytes.is_empty() {
            result.bytes = B256::default().into();
        }

        result
    })
}

/// [RIP-7212](https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md) secp256r1 precompile.
pub const ODYSSEY_P256: PrecompileWithAddress =
    PrecompileWithAddress(ODYSSEY_P256_ADDRESS, p256_verify);

/// [`PrecompileProvider`] wrapper that enables [`P256VERIFY`] if `odyssey` is enabled.
pub struct MaybeOdysseyPrecompiles {
    inner: EthPrecompiles,
    odyssey: bool,
}

impl MaybeOdysseyPrecompiles {
    /// Creates a new instance of the [`MaybeOdysseyPrecompiles`].
    pub fn new(odyssey: bool) -> Self {
        Self { inner: EthPrecompiles::default(), odyssey }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for MaybeOdysseyPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context::Cfg>::Spec) {
        PrecompileProvider::<CTX>::set_spec(&mut self.inner, spec);
    }

    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        bytes: &Bytes,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        if self.odyssey && address == ODYSSEY_P256.address() {
            let mut result = InterpreterResult {
                result: InstructionResult::Return,
                gas: Gas::new(gas_limit),
                output: Bytes::new(),
            };

            match ODYSSEY_P256.precompile()(bytes, gas_limit) {
                Ok(output) => {
                    let underflow = result.gas.record_cost(output.gas_used);
                    if underflow {
                        result.result = InstructionResult::PrecompileOOG;
                    } else {
                        result.result = InstructionResult::Return;
                        result.output = output.bytes;
                    }
                }
                Err(e) => {
                    if let PrecompileError::Fatal(_) = e {
                        return Err(e.to_string());
                    }
                    result.result = if e.is_oog() {
                        InstructionResult::PrecompileOOG
                    } else {
                        InstructionResult::PrecompileError
                    };
                }
            }
        }

        self.inner.run(context, address, bytes, gas_limit)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let warm_addresses = self.inner.warm_addresses() as Box<dyn Iterator<Item = Address>>;

        let iter = if self.odyssey {
            Box::new(warm_addresses.chain(core::iter::once(*ODYSSEY_P256.address())))
        } else {
            warm_addresses
        };

        Box::new(iter)
    }

    fn contains(&self, address: &Address) -> bool {
        if self.odyssey && address == ODYSSEY_P256.address() {
            true
        } else {
            self.inner.contains(address)
        }
    }
}
