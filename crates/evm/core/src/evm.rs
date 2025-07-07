use std::ops::{Deref, DerefMut};

use crate::{Env, InspectorExt, backend::DatabaseExt};
use alloy_evm::{
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompileInput, PrecompilesMap},
};
use revm::{
    Context, Journal,
    context::{BlockEnv, CfgEnv, Evm as RevmEvm, JournalTr, LocalContext, TxEnv},
    handler::{EthFrame, EthPrecompiles, instructions::EthInstructions},
    inspector::InspectorEvmTr,
    interpreter::interpreter::EthInterpreter,
    precompile::{
        PrecompileSpecId, Precompiles,
        secp256r1::{P256VERIFY, P256VERIFY_BASE_GAS_FEE},
    },
    primitives::hardfork::SpecId,
};

pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    let mut ctx = EthEvmContext {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(env.evm_env.cfg_env.spec);
            journal
        },
        block: env.evm_env.block_env,
        cfg: env.evm_env.cfg_env,
        tx: env.tx,
        chain: (),
        local: LocalContext::default(),
        error: Ok(()),
    };
    ctx.cfg.tx_chain_id_check = true;
    let spec = ctx.cfg.spec;

    let mut evm = FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            get_precompiles(spec),
        ),
    };

    inject_precompiles(&mut evm);

    evm
}

pub fn new_evm_with_existing_context<'a>(
    ctx: EthEvmContext<&'a mut dyn DatabaseExt>,
    inspector: &'a mut dyn InspectorExt,
) -> FoundryEvm<'a, &'a mut dyn InspectorExt> {
    let spec = ctx.cfg.spec;

    let mut evm = FoundryEvm {
        inner: RevmEvm::new_with_inspector(
            ctx,
            inspector,
            EthInstructions::default(),
            get_precompiles(spec),
        ),
    };

    inject_precompiles(&mut evm);

    evm
}

/// Conditionally inject additional precompiles into the EVM context.
fn inject_precompiles(evm: &mut FoundryEvm<'_, impl InspectorExt>) {
    if evm.inner.inspector().is_odyssey() {
        evm.inner.precompiles.apply_precompile(P256VERIFY.address(), |_| {
            // Create a wrapper function that adapts the new API
            let precompile_fn = |input: PrecompileInput<'_>| -> Result<_, _> {
                P256VERIFY.precompile()(input.data, P256VERIFY_BASE_GAS_FEE)
            };
            Some(DynPrecompile::from(precompile_fn))
        });
    }
}

/// Get the precompiles for the given spec.
fn get_precompiles(spec: SpecId) -> PrecompilesMap {
    PrecompilesMap::from_static(
        EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles,
    )
}

pub struct FoundryEvm<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    pub inner: RevmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
        EthFrame<EthInterpreter>,
    >,
}

impl<'db, I: InspectorExt> Deref for FoundryEvm<'db, I> {
    type Target = Context<BlockEnv, TxEnv, CfgEnv, &'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<I: InspectorExt> DerefMut for FoundryEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}
