// NB: This function is copy-pasted from upstream's call_inner
macro_rules! call_inner {
    ($handle:ident, $code_address:ident, $transfer:ident, $input:ident, $target_gas:ident, $is_static:ident, $take_l64:ident,  $take_stipend:ident, $context:ident) => {
        {
            let code_address = $code_address;
            let transfer = $transfer;
            let input = $input;
            let target_gas = $target_gas;
            let is_static = $is_static;
            let take_l64 = $take_l64;
            let take_stipend = $take_stipend;
            let context = $context;

            let pre_index = $handle.state().trace_index;
            let trace = $handle.start_trace(
                code_address,
                input.clone(),
                transfer.as_ref().map(|x| x.value).unwrap_or_default(),
                false,
            );

            macro_rules! try_or_fail {
                ( $e:expr ) => {
                    match $e {
                        Ok(v) => v,
                        Err(e) => {
                            $handle.fill_trace(&trace, false, None, pre_index);
                            return Capture::Exit((e.into(), Vec::new()))
                        }
                    }
                };
            }

            fn l64(gas: u64) -> u64 {
                gas - gas / 64
            }

            let after_gas = if take_l64 && $handle.config().call_l64_after_gas {
                if $handle.config().estimate {
                    let initial_after_gas = $handle.state().metadata().gasometer().gas();
                    let diff = initial_after_gas - l64(initial_after_gas);
                    try_or_fail!($handle.state_mut().metadata_mut().gasometer_mut().record_cost(diff));
                    $handle.state().metadata().gasometer().gas()
                } else {
                    l64($handle.state().metadata().gasometer().gas())
                }
            } else {
                $handle.state().metadata().gasometer().gas()
            };

            let target_gas = target_gas.unwrap_or(after_gas);
            let mut gas_limit = std::cmp::min(target_gas, after_gas);

            try_or_fail!($handle.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

            if let Some(transfer) = transfer.as_ref() {
                if take_stipend && transfer.value != U256::zero() {
                    gas_limit = gas_limit.saturating_add($handle.config().call_stipend);
                }
            }

            let code = $handle.code(code_address);
            $handle.stack_executor_mut().enter_substate(gas_limit, is_static);
            $handle.state_mut().touch(context.address);

            if let Some(depth) = $handle.state().metadata().depth() {
                if depth > $handle.config().call_stack_limit {
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                    return Capture::Exit((ExitError::CallTooDeep.into(), Vec::new()))
                }
            }

            if let Some(transfer) = transfer {
                match $handle.state_mut().transfer(transfer) {
                    Ok(()) => (),
                    Err(e) => {
                        $handle.fill_trace(&trace, false, None, pre_index);
                        let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                        return Capture::Exit((ExitReason::Error(e), Vec::new()))
                    }
                }
            }

            if let Some(result) = $handle.stack_executor().precompiles().execute(
                code_address,
                &input,
                Some(gas_limit),
                &context,
                is_static,
            ) {
                return match result {
                    Ok(PrecompileOutput { exit_status, output, cost, logs }) => {
                        for Log { address, topics, data } in logs {
                            match $handle.log(address, topics, data) {
                                Ok(_) => continue,
                                Err(error) => {
                                    $handle.fill_trace(&trace, false, Some(output.clone()), pre_index);
                                    return Capture::Exit((ExitReason::Error(error), output))
                                }
                            }
                        }

                        let _ = $handle.state_mut().metadata_mut().gasometer_mut().record_cost(cost);
                        $handle.fill_trace(&trace, true, Some(output.clone()), pre_index);
                        let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                        Capture::Exit((ExitReason::Succeed(exit_status), output))
                    }
                    Err(e) => {
                        let e = match e {
                            PrecompileFailure::Error { exit_status } => ExitReason::Error(exit_status),
                            PrecompileFailure::Revert { exit_status, .. } => {
                                ExitReason::Revert(exit_status)
                            }
                            PrecompileFailure::Fatal { exit_status } => ExitReason::Fatal(exit_status),
                        };
                        $handle.fill_trace(&trace, false, None, pre_index);
                        let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                        Capture::Exit((e, Vec::new()))
                    }
                }
            }

            // each cfg is about 200 bytes, is this a lot to clone? why does this error
            // not manifest upstream?
            let config = $handle.config().clone();
            let mut runtime;
            let reason = if $handle.state().debug_enabled {
                let code = Rc::new(code);
                runtime = Runtime::new(code.clone(), Rc::new(input), context, &config);
                $handle.handler_mut().debug_execute(&mut runtime, code_address, code, false)
            } else {
                runtime = Runtime::new(Rc::new(code), Rc::new(input), context, &config);
                $handle.execute(&mut runtime)
            };

            // // log::debug!(target: "evm", "Call execution using address {}: {:?}", code_address,
            // reason);

            match reason {
                ExitReason::Succeed(s) => {
                    $handle.fill_trace(&trace, true, Some(runtime.machine().return_value()), pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                    Capture::Exit((ExitReason::Succeed(s), runtime.machine().return_value()))
                }
                ExitReason::Error(e) => {
                    $handle.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    Capture::Exit((ExitReason::Error(e), Vec::new()))
                }
                ExitReason::Revert(e) => {
                    $handle.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                    Capture::Exit((ExitReason::Revert(e), runtime.machine().return_value()))
                }
                ExitReason::Fatal(e) => {
                    $handle.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                    $handle.state_mut().metadata_mut().gasometer_mut().fail();
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    Capture::Exit((ExitReason::Fatal(e), Vec::new()))
                }
            }
        }
    };
}

// NB: This function is copy-pasted from upstream's create_inner
macro_rules! create_inner {
    ($handle:ident, $caller:ident, $scheme:ident, $value:ident, $init_code:ident, $target_gas:ident, $take_l64:ident) => {
        {
            let caller = $caller;
            let scheme = $scheme;
            let value = $value;
            let init_code = $init_code;
            let target_gas = $target_gas;
            let take_l64 = $take_l64;


            let pre_index = $handle.state().trace_index;
            let address = $handle.create_address(scheme);
            let trace = $handle.start_trace(address, init_code.clone(), value, true);

            macro_rules! try_or_fail {
                ( $e:expr ) => {
                    match $e {
                        Ok(v) => v,
                        Err(e) => {
                            $handle.fill_trace(&trace, false, None, pre_index);
                            return Capture::Exit((e.into(), None, Vec::new()))
                        }
                    }
                };
            }

            fn check_first_byte(config: &Config, code: &[u8]) -> Result<(), ExitError> {
                if config.disallow_executable_format {
                    if let Some(0xef) = code.get(0) {
                        return Err(ExitError::InvalidCode)
                    }
                }
                Ok(())
            }

            fn l64(gas: u64) -> u64 {
                gas - gas / 64
            }

            $handle.state_mut().metadata_mut().access_address(caller);
            $handle.state_mut().metadata_mut().access_address(address);

            if let Some(depth) = $handle.state().metadata().depth() {
                if depth > $handle.config().call_stack_limit {
                    $handle.fill_trace(&trace, false, None, pre_index);
                    return Capture::Exit((ExitError::CallTooDeep.into(), None, Vec::new()))
                }
            }

            if $handle.balance(caller) < value {
                $handle.fill_trace(&trace, false, None, pre_index);
                return Capture::Exit((ExitError::OutOfFund.into(), None, Vec::new()))
            }

            let after_gas = if take_l64 && $handle.config().call_l64_after_gas {
                if $handle.config().estimate {
                    let initial_after_gas = $handle.state().metadata().gasometer().gas();
                    let diff = initial_after_gas - l64(initial_after_gas);
                    try_or_fail!($handle.state_mut().metadata_mut().gasometer_mut().record_cost(diff));
                    $handle.state().metadata().gasometer().gas()
                } else {
                    l64($handle.state().metadata().gasometer().gas())
                }
            } else {
                $handle.state().metadata().gasometer().gas()
            };

            let target_gas = target_gas.unwrap_or(after_gas);

            let gas_limit = core::cmp::min(after_gas, target_gas);
            try_or_fail!($handle.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

            $handle.state_mut().inc_nonce(caller);

            $handle.stack_executor_mut().enter_substate(gas_limit, false);

            {
                if $handle.code_size(address) != U256::zero() {
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
                }

                if $handle.stack_executor_mut().nonce(address) > U256::zero() {
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
                }

                $handle.state_mut().reset_storage(address);
            }

            let context = Context { address, caller, apparent_value: value };
            let transfer = Transfer { source: caller, target: address, value };
            match $handle.state_mut().transfer(transfer) {
                Ok(()) => (),
                Err(e) => {
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                    return Capture::Exit((ExitReason::Error(e), None, Vec::new()))
                }
            }

            if $handle.config().create_increase_nonce {
                $handle.state_mut().inc_nonce(address);
            }

            let config = $handle.config().clone();
            let mut runtime;
            let reason = if $handle.state().debug_enabled {
                let code = Rc::new(init_code);
                runtime = Runtime::new(code.clone(), Rc::new(Vec::new()), context, &config);
                $handle.handler_mut().debug_execute(&mut runtime, address, code, true)
            } else {
                runtime = Runtime::new(Rc::new(init_code), Rc::new(Vec::new()), context, &config);
                $handle.execute(&mut runtime)
            };
            // log::debug!(target: "evm", "Create execution using address {}: {:?}", address, reason);

            match reason {
                ExitReason::Succeed(s) => {
                    let out = runtime.machine().return_value();

                    // As of EIP-3541 code starting with 0xef cannot be deployed
                    if let Err(e) = check_first_byte($handle.config(), &out) {
                        $handle.state_mut().metadata_mut().gasometer_mut().fail();
                        $handle.fill_trace(&trace, false, None, pre_index);
                        let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                        return Capture::Exit((e.into(), None, Vec::new()))
                    }

                    if let Some(limit) = $handle.config().create_contract_limit {
                        if out.len() > limit {
                            $handle.state_mut().metadata_mut().gasometer_mut().fail();
                            $handle.fill_trace(&trace, false, None, pre_index);
                            let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                            return Capture::Exit((
                                ExitError::CreateContractLimit.into(),
                                None,
                                Vec::new(),
                            ))
                        }
                    }

                    match $handle.state_mut().metadata_mut().gasometer_mut().record_deposit(out.len()) {
                        Ok(()) => {
                            $handle.fill_trace(&trace, true, Some(out.clone()), pre_index);
                            let e = $handle.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                            $handle.state_mut().set_code(address, out);
                            // this may overwrite the trace and thats okay
                            try_or_fail!(e);
                            Capture::Exit((ExitReason::Succeed(s), Some(address), Vec::new()))
                        }
                        Err(e) => {
                            $handle.fill_trace(&trace, false, None, pre_index);
                            let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                            Capture::Exit((ExitReason::Error(e), None, Vec::new()))
                        }
                    }
                }
                ExitReason::Error(e) => {
                    $handle.state_mut().metadata_mut().gasometer_mut().fail();
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    Capture::Exit((ExitReason::Error(e), None, Vec::new()))
                }
                ExitReason::Revert(e) => {
                    $handle.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                    Capture::Exit((ExitReason::Revert(e), None, runtime.machine().return_value()))
                }
                ExitReason::Fatal(e) => {
                    $handle.state_mut().metadata_mut().gasometer_mut().fail();
                    $handle.fill_trace(&trace, false, None, pre_index);
                    let _ = $handle.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    Capture::Exit((ExitReason::Fatal(e), None, Vec::new()))
                }
            }
        }
    }
}

macro_rules! log {
    ($handle:ident, $address:ident, $topics:ident, $data:ident) => {{
        let address = $address;
        let topics = $topics;
        let data = $data;

        if $handle.state().trace_enabled {
            let index = $handle.state().trace_index;
            let node = &mut $handle.state_mut().traces.last_mut().expect("no traces").arena[index];
            node.ordering.push(LogCallOrder::Log(node.logs.len()));
            node.logs.push(RawLog { topics: topics.clone(), data: data.clone() });
        }

        if let Some(decoded) =
            convert_log(Log { address, topics: topics.clone(), data: data.clone() })
        {
            $handle.state_mut().all_logs.push(decoded);
        }

        if !$handle.state().expected_emits.is_empty() {
            // get expected emits
            let expected_emits = &mut $handle.state_mut().expected_emits;

            // do we have empty expected emits to fill?
            if let Some(next_expect_to_fill) =
                expected_emits.iter_mut().find(|expect| expect.log.is_none())
            {
                next_expect_to_fill.log =
                    Some(RawLog { topics: topics.clone(), data: data.clone() });
            } else {
                // no unfilled, grab next unfound
                // try to fill the first unfound
                if let Some(next_expect) = expected_emits.iter_mut().find(|expect| !expect.found) {
                    // unpack the log
                    if let Some(RawLog { topics: expected_topics, data: expected_data }) =
                        &next_expect.log
                    {
                        if expected_topics[0] == topics[0] {
                            // same event topic 0, topic length should be the same
                            let topics_match = topics
                                .iter()
                                .skip(1)
                                .enumerate()
                                .filter(|(i, _topic)| {
                                    // do we want to check?
                                    next_expect.checks[*i]
                                })
                                .all(|(i, topic)| topic == &expected_topics[i + 1]);

                            // check data
                            next_expect.found = if next_expect.checks[3] {
                                expected_data == &data && topics_match
                            } else {
                                topics_match
                            };
                        }
                    }
                }
            }
        }

        $handle.stack_executor_mut().log(address, topics, data)
    }};
}

macro_rules! start_trace {
    ($handle:ident, $address:ident, $input:ident, $transfer:ident, $creation:ident) => {{
        let address = $address;
        let input = $input;
        let transfer = $transfer;
        let creation = $creation;

        if $handle.handler().is_tracing_enabled() {
            let mut trace: CallTrace = CallTrace {
                // depth only starts tracking at first child substate and is 0. so add 1 when depth
                // is some.
                depth: if let Some(depth) = $handle.state().metadata().depth() {
                    depth + 1
                } else {
                    0
                },
                addr: address,
                created: creation,
                data: input,
                value: transfer,
                label: $handle.state().labels.get(&address).cloned(),
                ..Default::default()
            };

            $handle.state_mut().trace_mut().push_trace(0, &mut trace);
            $handle.state_mut().trace_index = trace.idx;
            Some(trace)
        } else {
            None
        }
    }};
}

pub(crate) use call_inner;
pub(crate) use create_inner;
pub(crate) use log;
pub(crate) use start_trace;
