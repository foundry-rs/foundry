use crate::{backend::DatabaseExt, Env, InspectorExt};
use alloy_evm::eth::EthEvmContext;
use alloy_primitives::Address;
use revm::{
    context::{ContextTr, Evm, EvmData, JournalTr},
    handler::{instructions::EthInstructions, EthPrecompiles, PrecompileProvider},
    interpreter::{interpreter::EthInterpreter, InputsImpl, InterpreterResult},
    Journal,
};

pub type FoundryEvmContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt>;

pub type FoundryEvm<'db, INSP> = Evm<
    FoundryEvmContext<'db>,
    INSP,
    EthInstructions<EthInterpreter, FoundryEvmContext<'db>>,
    FoundryPrecompiles,
>;

pub struct FoundryPrecompiles {
    inner: EthPrecompiles,
}

impl FoundryPrecompiles {
    pub fn new() -> Self {
        Self { inner: EthPrecompiles::default() }
    }
}

impl Default for FoundryPrecompiles {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for FoundryPrecompiles {
    type Output = InterpreterResult;

    /// Set the spec for the precompiles.
    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context::Cfg>::Spec) -> bool {
        PrecompileProvider::<CTX>::set_spec(&mut self.inner, spec)
    }

    /// Run the precompile.
    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        self.inner.run(context, address, inputs, is_static, gas_limit)
    }

    /// Get the warm addresses.
    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.inner.warm_addresses()
    }

    /// Check if the address is a precompile.
    fn contains(&self, address: &Address) -> bool {
        self.inner.contains(address)
    }
}

pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: &Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    new_evm_with_context(
        FoundryEvmContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                journal.set_spec_id(env.evm_env.cfg_env.spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            error: Ok(()),
        },
        inspector,
    )
}

pub fn new_evm_with_context<'db, 'i, I: InspectorExt + ?Sized>(
    ctx: FoundryEvmContext<'db>,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    Evm {
        data: EvmData { ctx, inspector },
        instruction: EthInstructions::default(),
        precompiles: FoundryPrecompiles::new(),
    }
}
