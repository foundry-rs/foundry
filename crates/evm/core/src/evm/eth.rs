use super::*;

type EthEvmHandler<'db, I> =
    MainnetHandler<EthRevmEvm<'db, I>, EVMError<DatabaseError>, EthFrame<EthInterpreter>>;

pub type EthRevmEvm<'db, I> = RevmEvm<
    EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
    PrecompilesMap,
    EthFrame<EthInterpreter>,
>;

pub struct EthFoundryEvm<
    'db,
    I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>,
> {
    pub inner: EthRevmEvm<'db, I>,
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Evm
    for EthFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<EthEvmFactory>;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Tx = TxEnv;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (&self.inner.ctx.journaled_state.database, &self.inner.inspector, &self.inner.precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.ctx.journaled_state.database,
            &mut self.inner.inspector,
            &mut self.inner.precompiles,
        )
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.set_tx(tx);

        let result = EthEvmHandler::<I>::default().inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.ctx.journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> Deref
    for EthFoundryEvm<'db, I>
{
    type Target = EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.ctx
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> DerefMut
    for EthFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.ctx
    }
}

impl FoundryEvmFactory for EthEvmFactory {
    type FoundryContext<'db> = EthEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = EthFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let eth_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        let mut inner = eth_evm.into_inner();
        inner.ctx.cfg.tx_chain_id_check = true;

        let mut evm = EthFoundryEvm { inner };
        evm.inspector().get_networks().inject_precompiles(evm.precompiles_mut());
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = SpecId, Block = BlockEnv, Tx = TxEnv> + 'db> {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).inner)
    }
}

impl<'db, I: FoundryInspectorExt<EthEvmContext<&'db mut dyn DatabaseExt<EthEvmFactory>>>> NestedEvm
    for EthRevmEvm<'db, I>
{
    type Spec = SpecId;
    type Block = BlockEnv;
    type Tx = TxEnv;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = EthEvmHandler::<I>::default();

        // Create first frame
        let memory =
            SharedMemory::new_with_buffer(self.ctx().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        // Run execution loop
        let mut frame_result = handler.inspect_run_exec_loop(self, first_frame_input)?;

        // Handle last frame result
        handler.last_frame_result(self, &mut frame_result)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.set_tx(tx);

        let result = EthEvmHandler::<I>::default().inspect_run(self)?;

        Ok(ResultAndState::new(result, self.ctx.journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}
