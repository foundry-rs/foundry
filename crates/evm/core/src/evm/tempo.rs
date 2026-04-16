use super::*;

// Will be removed when the next revm release includes bluealloy/revm#3518.
pub type TempoRevmEvm<'db, I> = tempo_revm::TempoEvm<&'db mut dyn DatabaseExt<TempoEvmFactory>, I>;

/// Tempo counterpart of [`EthFoundryEvm`]. Wraps `tempo_revm::TempoEvm` and routes execution
/// through [`TempoEvmHandler`].
///
/// Uses [`TempoEvmFactory`] for construction to reuse factory setup logic, then unwraps to the
/// raw revm EVM via `into_inner()` since the handler operates at the revm level.
pub struct TempoFoundryEvm<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
> {
    pub inner: TempoRevmEvm<'db, I>,
}

/// Initialize Tempo precompiles and contracts for a newly created [`TempoFoundryEvm`].
///
/// In non-fork mode, runs full genesis initialization (precompile sentinel bytecode,
/// TIP20 fee tokens, standard contracts) via [`StorageCtx::enter_evm`].
///
/// In fork mode, warms up precompile and TIP20 token addresses with sentinel bytecode
/// to prevent repeated RPC round-trips for addresses that are Rust-native precompiles
/// on Tempo nodes (no real EVM bytecode on-chain).
pub(crate) fn initialize_tempo_evm<
    'db,
    I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>,
>(
    evm: &mut TempoFoundryEvm<'db, I>,
    is_forked: bool,
) {
    let ctx = &mut evm.inner.inner.ctx;
    StorageCtx::enter_evm(&mut ctx.journaled_state, &ctx.block, &ctx.cfg, &ctx.tx, || {
        if is_forked {
            // In fork mode, warm up precompile accounts to avoid repeated RPC fetches.
            let mut sctx = StorageCtx;
            let sentinel = Bytecode::new_legacy(Bytes::from_static(&[0xef]));
            for addr in TEMPO_PRECOMPILE_ADDRESSES.iter().chain(TEMPO_TIP20_TOKENS.iter()) {
                sctx.set_code(*addr, sentinel.clone())
                    .expect("failed to warm tempo precompile address");
            }
        } else {
            // In non-fork mode, run full genesis initialization.
            initialize_tempo_genesis_inner(TEST_CONTRACT_ADDRESS, CALLER)
                .expect("tempo genesis initialization failed");
        }
    });
}

impl FoundryEvmFactory for TempoEvmFactory {
    type FoundryContext<'db> = TempoContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        TempoFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let is_forked = db.is_forked_mode();
        let spec = *evm_env.spec_id();
        let tempo_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        let mut inner = tempo_evm.into_inner();
        inner.ctx.cfg.gas_params = tempo_gas_params(spec);
        inner.ctx.cfg.tx_chain_id_check = true;
        if inner.ctx.cfg.tx_gas_limit_cap.is_none() {
            inner.ctx.cfg.tx_gas_limit_cap = spec.tx_gas_limit_cap();
        }

        let mut evm = TempoFoundryEvm { inner };
        let networks = Evm::inspector(&evm).get_networks();
        networks.inject_precompiles(evm.precompiles_mut());

        initialize_tempo_evm(&mut evm, is_forked);
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = TempoHardfork, Block = TempoBlockEnv, Tx = TempoTxEnv> + 'db>
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_nested_evm())
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Evm
    for TempoFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<TempoEvmFactory>;
    type Error = EVMError<DatabaseError, TempoInvalidTransaction>;
    type HaltReason = TempoHaltReason;
    type Spec = TempoHardfork;
    type Tx = TempoTxEnv;
    type BlockEnv = TempoBlockEnv;

    fn block(&self) -> &TempoBlockEnv {
        &self.inner.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        let evm = &self.inner.inner;
        (&evm.ctx.journaled_state.database, &evm.inspector, &evm.precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        let evm = &mut self.inner.inner;
        (&mut evm.ctx.journaled_state.database, &mut evm.inspector, &mut evm.precompiles)
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("TempoFoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.set_tx(tx);

        let mut handler = TempoEvmHandler::new();
        let result = handler.inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.inner.ctx.journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult<Self::HaltReason>>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec, Self::BlockEnv>)
    where
        Self: Sized,
    {
        let revm_evm = self.inner.inner;
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = revm_evm.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> Deref
    for TempoFoundryEvm<'db, I>
{
    type Target = TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> DerefMut
    for TempoFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>>
    IntoNestedEvm<TempoHardfork, TempoBlockEnv, TempoTxEnv> for TempoFoundryEvm<'db, I>
{
    type Inner = TempoRevmEvm<'db, I>;

    fn into_nested_evm(self) -> Self::Inner {
        self.inner
    }
}

/// Maps a Tempo [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
///
/// This exists because [`NestedEvm`] currently uses Eth-typed errors. When `NestedEvm` gains
/// an associated `Error` type, this mapping can be removed.
pub(crate) fn map_tempo_error(
    e: EVMError<DatabaseError, TempoInvalidTransaction>,
) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
        EVMError::CustomAny(custom_any_error) => EVMError::CustomAny(custom_any_error),
        EVMError::Transaction(t) => match t {
            TempoInvalidTransaction::EthInvalidTransaction(eth) => EVMError::Transaction(eth),
            t => EVMError::Custom(format!("tempo transaction error: {t}")),
        },
    }
}

impl<'db, I: FoundryInspectorExt<TempoContext<&'db mut dyn DatabaseExt<TempoEvmFactory>>>> NestedEvm
    for TempoRevmEvm<'db, I>
{
    type Spec = TempoHardfork;
    type Block = TempoBlockEnv;
    type Tx = TempoTxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = TempoEvmHandler::new();

        let memory =
            SharedMemory::new_with_buffer(self.ctx().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_tempo_error)?;

        handler.last_frame_result(self, &mut frame_result).map_err(map_tempo_error)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let mut handler = TempoEvmHandler::new();
        let result = handler.inspect_run(self).map_err(map_tempo_error)?;

        let result = result.map_haltreason(|h| match h {
            TempoHaltReason::Ethereum(eth) => eth,
            _ => HaltReason::PrecompileError,
        });

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
