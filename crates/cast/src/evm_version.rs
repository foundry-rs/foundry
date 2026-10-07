//! Detects the EVM version a node executes at a given block.
//!
//! Replaying a transaction needs the spec its chain applied at that block. For chains without a
//! hardfork schedule known to Foundry, header fields only hint at it, and replaying an old block
//! under a newer spec can make a transaction succeed locally that failed on-chain because an
//! opcode was not active yet. [`probe_evm_version`] instead asks the node which features it
//! executes.

use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use foundry_compilers::artifacts::EvmVersion;
use revm::bytecode::opcode::{
    AND, CALL, CLZ, CODECOPY, CREATE, DUP6, EQ, GAS, INVALID, ISZERO, JUMPDEST, JUMPI, MCOPY,
    MSTORE, MUL, OR, POP, PUSH0, PUSH1, PUSH2, PUSH3, RETURN, RETURNDATASIZE, SLOTNUM, STATICCALL,
    STOP, SWAP1,
};

/// Probed features in activation order, each paired with the EVM version that introduced it and
/// runtime code that halts exceptionally unless the feature is active.
#[rustfmt::skip]
const PROBES: [(EvmVersion, &[u8]); 5] = [
    (EvmVersion::Shanghai, &[PUSH0, POP, STOP]),
    (EvmVersion::Cancun, &[PUSH1, 0, PUSH1, 0, PUSH1, 0, MCOPY, STOP]),
    // Calls the BLS12-381 G1ADD precompile (0x0b) with two points at infinity and requires its
    // 128-byte result. Before Prague the call reaches an empty account and returns no data.
    (EvmVersion::Prague, &[
        // STATICCALL(GAS, 0x0b, 0, 256, 0, 0)
        PUSH1, 0, PUSH1, 0, PUSH2, 0x01, 0x00, PUSH1, 0, PUSH1, 0x0b, GAS, STATICCALL,
        // Halt unless the call succeeded and returned 128 bytes.
        RETURNDATASIZE, PUSH1, 0x80, EQ, AND, PUSH1, 22, JUMPI, INVALID, JUMPDEST, STOP,
    ]),
    (EvmVersion::Osaka, &[PUSH1, 0, CLZ, POP, STOP]),
    (EvmVersion::Amsterdam, &[SLOTNUM, POP, STOP]),
];

/// Gas forwarded to each probe. A failing probe consumes all of it.
const PROBE_GAS: u32 = 20_000;

/// Gas limit of the probe call, enough for the worst case in which every probe fails. Under
/// EIP-8037 the six accounts the probe creates cost 1,101,600 state gas alone.
const PROBE_CALL_GAS: u64 = 3_000_000;

/// Returns the newest EVM version whose features, and those of every earlier probed version, the
/// node executes at `block`.
///
/// Returns `None` if the node rejects the probe call or returns an unexpected result.
pub async fn probe_evm_version<N: Network, P: Provider<N>>(
    provider: &P,
    block: BlockId,
) -> Option<EvmVersion> {
    let request = N::TransactionRequest::default()
        .with_deploy_code(probe_code())
        .with_gas_limit(PROBE_CALL_GAS);
    let output = match provider.call(request).block(block).await {
        Ok(output) => output,
        Err(err) => {
            trace!(%err, "EVM version probe failed");
            return None;
        }
    };
    let Ok(word) = <[u8; 32]>::try_from(output.as_ref()) else {
        trace!(%output, "unexpected EVM version probe output");
        return None;
    };
    let mask = U256::from_be_bytes(word);
    if !(mask >> PROBES.len()).is_zero() {
        trace!(%mask, "unexpected EVM version probe mask");
        return None;
    }
    let version = evm_version_from_mask(mask);
    trace!(?version, %mask, "probed EVM version");
    Some(version)
}

/// Maps the probe result to the newest version whose probe and all earlier probes succeeded.
///
/// A node that fails every probe predates Shanghai. Paris is returned then, the newest version
/// without `PUSH0`.
fn evm_version_from_mask(mask: U256) -> EvmVersion {
    PROBES
        .iter()
        .enumerate()
        .take_while(|(bit, _)| mask.bit(*bit))
        .last()
        .map_or(EvmVersion::Paris, |(_, (version, _))| *version)
}

/// Builds the creation code of the probe call.
///
/// For every probe it deploys the probe's runtime code, calls it with a bounded gas stipend so a
/// failure cannot exhaust the call, and sets bit `i` of the returned word if probe `i` succeeded.
/// The code itself only uses opcodes that predate every probed feature.
fn probe_code() -> Bytes {
    const PER_PROBE: usize = 39;
    const HEADER_AND_FOOTER: usize = 10;

    let [_, gas @ ..] = PROBE_GAS.to_be_bytes();
    // The accumulator for the result stays at the bottom of the stack.
    let mut code = vec![PUSH1, 0];
    let mut data = Vec::new();
    let mut offset = HEADER_AND_FOOTER + PER_PROBE * PROBES.len();
    for (bit, (_, runtime)) in PROBES.iter().enumerate() {
        let initcode = deploy_code(runtime);
        let len = u8::try_from(initcode.len()).expect("probe initcode length fits in PUSH1");
        let [start_hi, start_lo] =
            u16::try_from(offset).expect("probe initcode offset fits in PUSH2").to_be_bytes();
        // CODECOPY(0, start, len)
        code.extend([PUSH1, len, PUSH2, start_hi, start_lo, PUSH1, 0, CODECOPY]);
        // CREATE(0, 0, len)
        code.extend([PUSH1, len, PUSH1, 0, PUSH1, 0, CREATE]);
        // CALL(PROBE_GAS, address, 0, 0, 0, 0, 0), duplicating the created address from below the
        // five zero arguments.
        code.extend([PUSH1, 0].repeat(5));
        code.extend([DUP6, PUSH3, gas[0], gas[1], gas[2], CALL]);
        // success && address != 0
        code.extend([SWAP1, ISZERO, ISZERO, AND]);
        // accumulator |= result << bit
        code.extend([PUSH1, 1 << bit, MUL, OR]);
        offset += initcode.len();
        data.extend(initcode);
    }
    // MSTORE(0, accumulator) RETURN(0, 32)
    code.extend([PUSH1, 0, MSTORE, PUSH1, 32, PUSH1, 0, RETURN]);
    debug_assert_eq!(code.len(), HEADER_AND_FOOTER + PER_PROBE * PROBES.len());
    code.extend(data);
    code.into()
}

/// Returns initcode that deploys `runtime`.
fn deploy_code(runtime: &[u8]) -> Vec<u8> {
    const PREFIX_LEN: u8 = 12;
    let len = u8::try_from(runtime.len()).expect("probe runtime length fits in PUSH1");
    // CODECOPY(0, PREFIX_LEN, len) RETURN(0, len)
    let mut code =
        vec![PUSH1, len, PUSH1, PREFIX_LEN, PUSH1, 0, CODECOPY, PUSH1, len, PUSH1, 0, RETURN];
    code.extend_from_slice(runtime);
    code
}

#[cfg(test)]
mod tests {

    use super::*;
    use alloy_primitives::TxKind;
    use revm::{
        Context, ExecuteEvm, MainBuilder, MainContext, context::TxEnv, database::InMemoryDB,
        primitives::hardfork::SpecId,
    };

    #[test]
    fn detects_executed_spec() {
        for (spec, expected) in [
            (SpecId::LONDON, EvmVersion::Paris),
            (SpecId::MERGE, EvmVersion::Paris),
            (SpecId::SHANGHAI, EvmVersion::Shanghai),
            (SpecId::CANCUN, EvmVersion::Cancun),
            (SpecId::PRAGUE, EvmVersion::Prague),
            (SpecId::OSAKA, EvmVersion::Osaka),
            (SpecId::AMSTERDAM, EvmVersion::Amsterdam),
        ] {
            let result = Context::mainnet()
                .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(spec))
                .with_db(InMemoryDB::default())
                .build_mainnet()
                .transact(
                    TxEnv::builder()
                        .kind(TxKind::Create)
                        .data(probe_code())
                        .gas_limit(PROBE_CALL_GAS)
                        .build()
                        .unwrap(),
                )
                .unwrap();
            let output = result.result.into_output().unwrap();
            assert_eq!(evm_version_from_mask(U256::from_be_slice(&output)), expected, "{spec:?}");
        }
    }

    #[test]
    fn ignores_probes_after_a_failed_one() {
        assert_eq!(evm_version_from_mask(U256::from(0b1011)), EvmVersion::Cancun);
    }
}
