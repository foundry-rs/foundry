use super::*;

pub type OpRevmEvm<'db, I> = op_revm::OpEvm<
    OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
    PrecompilesMap,
>;

/// Optimism counterpart of [`EthFoundryEvm`]. Wraps `op_revm::OpEvm` and routes execution
/// through [`OpFoundryHandler`] which composes [`OpHandler`] with CREATE2 factory redirect logic.
pub struct OpFoundryEvm<
    'db,
    I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
> {
    pub inner: OpRevmEvm<'db, I>,
}

impl FoundryEvmFactory for OpEvmFactory {
    type FoundryContext<'db> = OpContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = OpFoundryEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let spec_id = *evm_env.spec_id();
        let mut inner = Context::op()
            .with_db(db)
            .with_block(evm_env.block_env)
            .with_cfg(evm_env.cfg_env)
            .build_op_with_inspector(inspector)
            .with_precompiles(PrecompilesMap::from_static(
                OpPrecompiles::new_with_spec(spec_id).precompiles(),
            ));
        inner.ctx().cfg.tx_chain_id_check = true;

        let mut evm = OpFoundryEvm { inner };
        let networks = Evm::inspector(&evm).get_networks();
        networks.inject_precompiles(evm.precompiles_mut());
        evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<dyn NestedEvm<Spec = OpSpecId, Block = BlockEnv, Tx = OpTransaction<TxEnv>> + 'db>
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_nested_evm())
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Evm
    for OpFoundryEvm<'db, I>
{
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt<OpEvmFactory>;
    type Error = EVMError<DatabaseError, OpTransactionError>;
    type HaltReason = OpHaltReason;
    type Spec = OpSpecId;
    type Tx = OpTransaction<TxEnv>;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        &self.inner.ctx_ref().block
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx_ref().cfg.chain_id
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        let (ctx, _, precompiles, _, inspector) = self.inner.all_inspector();
        (&ctx.journaled_state.database, inspector, precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        let (ctx, _, precompiles, _, inspector) = self.inner.all_mut_inspector();
        (&mut ctx.journaled_state.database, inspector, precompiles)
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("OpFoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.ctx().set_tx(tx);

        let mut handler = OpFoundryHandler::<I>::default();
        let result = handler.inspect_run(&mut self.inner)?;

        Ok(ResultAndState::new(result, self.inner.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ExecResultAndState<ExecutionResult<Self::HaltReason>>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.0.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Deref
    for OpFoundryEvm<'db, I>
{
    type Target = OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.0.ctx
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> DerefMut
    for OpFoundryEvm<'db, I>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.0.ctx
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>>
    IntoNestedEvm<OpSpecId, BlockEnv, OpTransaction<TxEnv>> for OpFoundryEvm<'db, I>
{
    type Inner = OpRevmEvm<'db, I>;

    fn into_nested_evm(self) -> Self::Inner {
        self.inner
    }
}

/// Maps an OP [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
fn map_op_error(e: EVMError<DatabaseError, OpTransactionError>) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
        EVMError::Transaction(t) => EVMError::Custom(format!("op transaction error: {t}")),
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> NestedEvm
    for OpRevmEvm<'db, I>
{
    type Spec = OpSpecId;
    type Block = BlockEnv;
    type Tx = OpTransaction<TxEnv>;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx().journaled_state.inner
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = OpFoundryHandler::<I>::default();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local.shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_op_error)?;

        handler.last_frame_result(self, &mut frame_result).map_err(map_op_error)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        self.ctx().set_tx(tx);

        let mut handler = OpFoundryHandler::<I>::default();
        let result = handler.inspect_run(self).map_err(map_op_error)?;

        let result = result.map_haltreason(|h| match h {
            OpHaltReason::Base(eth) => eth,
            _ => HaltReason::PrecompileError,
        });

        Ok(ResultAndState::new(result, self.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        EvmEnv::new(self.ctx_ref().cfg.clone(), self.ctx_ref().block.clone())
    }
}

/// Optimism handler that composes [`OpHandler`] with CREATE2 factory redirect logic.
pub struct OpFoundryHandler<
    'db,
    I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
> {
    inner: OpHandler<
        OpRevmEvm<'db, I>,
        EVMError<DatabaseError, OpTransactionError>,
        EthFrame<EthInterpreter>,
    >,
    create2_overrides: Vec<(usize, CallInputs)>,
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Default
    for OpFoundryHandler<'db, I>
{
    fn default() -> Self {
        Self { inner: OpHandler::new(), create2_overrides: Vec::new() }
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> Handler
    for OpFoundryHandler<'db, I>
{
    type Evm = OpRevmEvm<'db, I>;
    type Error = EVMError<DatabaseError, OpTransactionError>;
    type HaltReason = OpHaltReason;

    #[inline]
    fn run(
        &mut self,
        evm: &mut Self::Evm,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.run(evm)
    }

    #[inline]
    fn execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &revm::interpreter::InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        self.inner.execution(evm, init_and_floor_gas)
    }

    #[inline]
    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        self.inner.validate_env(evm)
    }

    #[inline]
    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<(), Self::Error> {
        self.inner.validate_against_state_and_deduct_caller(evm)
    }

    #[inline]
    fn reimburse_caller(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reimburse_caller(evm, exec_result)
    }

    #[inline]
    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reward_beneficiary(evm, exec_result)
    }

    #[inline]
    fn validate_initial_tx_gas(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<revm::interpreter::InitialAndFloorGas, Self::Error> {
        self.inner.validate_initial_tx_gas(evm)
    }

    #[inline]
    fn execution_result(
        &mut self,
        evm: &mut Self::Evm,
        result: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
        result_gas: revm::context::result::ResultGas,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.execution_result(evm, result, result_gas)
    }

    #[inline]
    fn catch_error(
        &self,
        evm: &mut Self::Evm,
        error: Self::Error,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.catch_error(evm, error)
    }
}

impl<'db, I: FoundryInspectorExt<OpContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>>
    InspectorHandler for OpFoundryHandler<'db, I>
{
    type IT = EthInterpreter;

    fn inspect_run_exec_loop(
        &mut self,
        evm: &mut Self::Evm,
        first_frame_input: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameInit,
    ) -> Result<FrameResult, Self::Error> {
        let res = evm.inspect_frame_init(first_frame_input)?;

        if let ItemOrResult::Result(frame_result) = res {
            return Ok(frame_result);
        }

        loop {
            let call_or_result = evm.inspect_frame_run()?;

            let result = match call_or_result {
                ItemOrResult::Item(mut init) => {
                    // Handle CREATE/CREATE2 frame initialization
                    if let FrameInput::Create(inputs) = &init.frame_input
                        && let CreateScheme::Create2 { salt } = inputs.scheme()
                    {
                        let (ctx, inspector) = evm.ctx_inspector();
                        if inspector.should_use_create2_factory(ctx.journal().depth(), inputs) {
                            let gas_limit = inputs.gas_limit();
                            let create2_deployer = evm.inspector().create2_deployer();
                            let call_inputs =
                                get_create2_factory_call_inputs(salt, inputs, create2_deployer);

                            self.create2_overrides
                                .push((evm.ctx_ref().journal().depth(), call_inputs.clone()));

                            let code_hash = evm
                                .ctx()
                                .journal_mut()
                                .load_account(create2_deployer)?
                                .info
                                .code_hash;
                            if code_hash == KECCAK_EMPTY {
                                return Ok(FrameResult::Call(CallOutcome {
                                    result: InterpreterResult {
                                        result: InstructionResult::Revert,
                                        output: Bytes::from(
                                            format!("missing CREATE2 deployer: {create2_deployer}")
                                                .into_bytes(),
                                        ),
                                        gas: Gas::new(gas_limit),
                                    },
                                    memory_offset: 0..0,
                                    was_precompile_called: false,
                                    precompile_call_logs: vec![],
                                }));
                            } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                                return Ok(FrameResult::Call(CallOutcome {
                                    result: InterpreterResult {
                                        result: InstructionResult::Revert,
                                        output: "invalid CREATE2 deployer bytecode".into(),
                                        gas: Gas::new(gas_limit),
                                    },
                                    memory_offset: 0..0,
                                    was_precompile_called: false,
                                    precompile_call_logs: vec![],
                                }));
                            }

                            init.frame_input = FrameInput::Call(Box::new(call_inputs));
                        }
                    }

                    match evm.inspect_frame_init(init)? {
                        ItemOrResult::Item(_) => continue,
                        ItemOrResult::Result(result) => result,
                    }
                }
                ItemOrResult::Result(result) => result,
            };

            // Handle CREATE2 override transformation if needed
            let result = if self
                .create2_overrides
                .last()
                .is_some_and(|(depth, _)| *depth == evm.ctx_ref().journal().depth())
            {
                let (_, call_inputs) = self.create2_overrides.pop().unwrap();
                let FrameResult::Call(mut call) = result else {
                    unreachable!("create2 override should be a call frame");
                };
                let address = match call.instruction_result() {
                    return_ok!() => Address::try_from(call.output().as_ref())
                        .map_err(|_| {
                            call.result = InterpreterResult {
                                result: InstructionResult::Revert,
                                output: "invalid CREATE2 factory output".into(),
                                gas: Gas::new(call_inputs.gas_limit),
                            };
                        })
                        .ok(),
                    _ => None,
                };
                FrameResult::Create(CreateOutcome { result: call.result, address })
            } else {
                result
            };

            if let Some(result) = evm.frame_return_result(result)? {
                return Ok(result);
            }
        }
    }
}
