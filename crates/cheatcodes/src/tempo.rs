//! Cheatcodes that delegate to the imported Tempo precompile crates.

use std::str::FromStr;

use alloy_sol_types::SolValue;
use foundry_evm_core::evm::FoundryEvmNetwork;
use revm::context::ContextTr;
use spec::Vm::{assumeImplicitApprovalCall, isImplicitlyApprovedCall};
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_precompiles::address_registry;

use crate::{Cheatcode, CheatsCtxt, Error, Result};
use foundry_config::ExecutionSpec;
use foundry_evm_core::constants::MAGIC_ASSUME;

/// Resolves the active Tempo hardfork from the cheatcode-visible spec, or `None` on non-Tempo
/// networks. Goes through the spec's stable string name because the context is generic over
/// the network type.
fn active_tempo_hardfork<FEN: FoundryEvmNetwork>(
    ccx: &CheatsCtxt<'_, '_, FEN>,
) -> Option<TempoHardfork> {
    let spec = *ccx.ecx.cfg().spec();
    TempoHardfork::from_str(&spec.evm_version_name()).ok()
}

impl Cheatcode for isImplicitlyApprovedCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { spender } = *self;
        let approved = match active_tempo_hardfork(ccx) {
            Some(hardfork) => address_registry::is_implicitly_approved(spender, hardfork),
            None => false,
        };
        Ok(approved.abi_encode())
    }
}

impl Cheatcode for assumeImplicitApprovalCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { spender } = *self;
        let approved = match active_tempo_hardfork(ccx) {
            Some(hardfork) => address_registry::is_implicitly_approved(spender, hardfork),
            None => false,
        };
        if approved { Ok(Default::default()) } else { Err(Error::from(MAGIC_ASSUME)) }
    }
}
