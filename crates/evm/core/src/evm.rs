use crate::{backend::DatabaseExt, EnvMut, InspectorExt};
use alloy_evm::{eth::EthEvmContext, EthEvm};
use alloy_primitives::Address;
use revm::{
    context::{ContextTr, Evm as RevmEvm, JournalTr},
    handler::{instructions::EthInstructions, EthPrecompiles, PrecompileProvider},
    interpreter::{InputsImpl, InterpreterResult},
    Journal,
};

pub type FoundryEvmContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt>;

pub type FoundryEvm<'db, I, P = FoundryPrecompiles> = EthEvm<FoundryEvmContext<'db>, I, P>;

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
    env: &EnvMut<'_>,
    inspector: &'i mut I,
) -> EthEvm<&'db mut dyn DatabaseExt, &'i mut I, FoundryPrecompiles> {
    let evm_context = EthEvmContext {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(env.cfg.spec);
            journal
        },
        block: env.block.clone(),
        cfg: env.cfg.clone(),
        tx: env.tx.clone(),
        chain: (),
        error: Ok(()),
    };

    let evm = RevmEvm::new_with_inspector(
        evm_context,
        inspector,
        EthInstructions::default(),
        FoundryPrecompiles::default(),
    );

    EthEvm::new(evm, true)
}
