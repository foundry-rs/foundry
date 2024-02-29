use super::{
    build::{CompiledState, LinkedBuildData, LinkedState},
    runner::ScriptRunner,
    JsonResult, NestedValue, ScriptArgs, ScriptConfig, ScriptResult,
};
use alloy_dyn_abi::FunctionExt;
use alloy_json_abi::{Function, InternalType, JsonAbi};
use alloy_primitives::{Address, Bytes, U64};
use alloy_rpc_types::request::TransactionRequest;
use async_recursion::async_recursion;
use eyre::Result;
use forge::{
    decode::{decode_console_logs, RevertDecoder},
    inspectors::cheatcodes::{BroadcastableTransaction, BroadcastableTransactions},
    traces::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier, SignaturesIdentifier},
        render_trace_arena, CallTraceDecoder, CallTraceDecoderBuilder, TraceKind,
    },
};
use foundry_cli::utils::{ensure_clean_constructor, needs_setup};
use foundry_common::{
    fmt::{format_token, format_token_raw},
    shell, ContractsByArtifact,
};
use foundry_compilers::artifacts::ContractBytecodeSome;
use foundry_config::Config;
use foundry_debugger::Debugger;
use std::collections::{HashMap, VecDeque};
use yansi::Paint;

pub struct ExecutionData {
    pub func: Function,
    pub calldata: Bytes,
    pub bytecode: Bytes,
    pub abi: JsonAbi,
}

pub struct PreExecutionState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
}

impl LinkedState {
    /// Given linked and compiled artifacts, prepares data we need for execution.
    pub async fn prepare_execution(self) -> Result<PreExecutionState> {
        let ContractBytecodeSome { abi, bytecode, .. } = self.build_data.get_target_contract()?;

        let bytecode = bytecode.into_bytes().ok_or_else(|| {
            eyre::eyre!("expected fully linked bytecode, found unlinked bytecode")
        })?;

        let (func, calldata) = self.args.get_method_and_calldata(&abi)?;

        ensure_clean_constructor(&abi)?;

        Ok(PreExecutionState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: ExecutionData { func, calldata, bytecode, abi },
        })
    }
}

pub struct ExecutedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
}

impl PreExecutionState {
    #[async_recursion]
    pub async fn execute(mut self) -> Result<ExecutedState> {
        let mut runner = self.script_config.get_runner(true).await?;
        let mut result = self.execute_with_runner(&mut runner).await?;

        // If we have a new sender from execution, we need to use it to deploy libraries and relink
        // contracts.
        if let Some(new_sender) = self.maybe_new_sender(result.transactions.as_ref())? {
            self.script_config.update_sender(new_sender).await?;

            // Rollback to linking state to relink contracts with the new sender.
            let state = CompiledState {
                args: self.args,
                script_config: self.script_config,
                build_data: self.build_data.build_data,
            };

            return state.link()?.prepare_execution().await?.execute().await;
        }

        // Add library deployment transactions to broadcastable transactions list.
        if let Some(txs) = &mut result.transactions {
            let mut library_txs = self
                .build_data
                .predeploy_libraries
                .iter()
                .enumerate()
                .map(|(i, bytes)| BroadcastableTransaction {
                    rpc: self.script_config.evm_opts.fork_url.clone(),
                    transaction: TransactionRequest {
                        from: Some(self.script_config.evm_opts.sender),
                        input: Some(bytes.clone()).into(),
                        nonce: Some(U64::from(self.script_config.sender_nonce + i as u64)),
                        ..Default::default()
                    },
                })
                .collect::<VecDeque<_>>();

            for tx in txs.iter() {
                library_txs.push_back(BroadcastableTransaction {
                    rpc: tx.rpc.clone(),
                    transaction: tx.transaction.clone(),
                });
            }
            *txs = library_txs;
        }

        Ok(ExecutedState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_result: result,
        })
    }

    pub async fn execute_with_runner(&self, runner: &mut ScriptRunner) -> Result<ScriptResult> {
        let (address, mut setup_result) = runner.setup(
            &self.build_data.predeploy_libraries,
            self.execution_data.bytecode.clone(),
            needs_setup(&self.execution_data.abi),
            self.script_config.sender_nonce,
            self.args.broadcast,
            self.script_config.evm_opts.fork_url.is_none(),
        )?;

        if setup_result.success {
            let script_result = runner.script(address, self.execution_data.calldata.clone())?;

            setup_result.success &= script_result.success;
            setup_result.gas_used = script_result.gas_used;
            setup_result.logs.extend(script_result.logs);
            setup_result.traces.extend(script_result.traces);
            setup_result.debug = script_result.debug;
            setup_result.labeled_addresses.extend(script_result.labeled_addresses);
            setup_result.returned = script_result.returned;
            setup_result.breakpoints = script_result.breakpoints;

            match (&mut setup_result.transactions, script_result.transactions) {
                (Some(txs), Some(new_txs)) => {
                    txs.extend(new_txs);
                }
                (None, Some(new_txs)) => {
                    setup_result.transactions = Some(new_txs);
                }
                _ => {}
            }
        }

        Ok(setup_result)
    }

    fn maybe_new_sender(
        &self,
        transactions: Option<&BroadcastableTransactions>,
    ) -> Result<Option<Address>> {
        let mut new_sender = None;

        if let Some(txs) = transactions {
            // If the user passed a `--sender` don't check anything.
            if !self.build_data.predeploy_libraries.is_empty() &&
                self.args.evm_opts.sender.is_none()
            {
                for tx in txs.iter() {
                    if tx.transaction.to.is_none() {
                        let sender = tx.transaction.from.expect("no sender");
                        if let Some(ns) = new_sender {
                            if sender != ns {
                                shell::println("You have more than one deployer who could predeploy libraries. Using `--sender` instead.")?;
                                return Ok(None);
                            }
                        } else if sender != self.script_config.evm_opts.sender {
                            new_sender = Some(sender);
                        }
                    }
                }
            }
        }
        Ok(new_sender)
    }
}

impl ExecutedState {
    pub async fn prepare_simulation(mut self) -> Result<PreSimulationState> {
        let returns = self.get_returns()?;

        let known_contracts = self.build_data.get_flattened_contracts(true);
        let decoder = self.build_trace_decoder(&known_contracts)?;

        if let Some(txs) = self.execution_result.transactions.as_ref() {
            self.script_config.collect_rpcs(txs);
        }

        if self.execution_result.transactions.as_ref().map_or(true, |txs| txs.is_empty()) &&
            self.args.broadcast
        {
            eyre::bail!("No onchain transactions generated in script");
        }

        self.script_config.check_multi_chain_constraints(&self.build_data.libraries)?;
        self.script_config.check_shanghai_support().await?;

        Ok(PreSimulationState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_result: self.execution_result,
            execution_artifacts: ExecutionArtifacts { known_contracts, decoder, returns },
        })
    }

    fn build_trace_decoder(
        &self,
        known_contracts: &ContractsByArtifact,
    ) -> Result<CallTraceDecoder> {
        let verbosity = self.script_config.evm_opts.verbosity;
        let mut etherscan_identifier = EtherscanIdentifier::new(
            &self.script_config.config,
            self.script_config.evm_opts.get_remote_chain_id(),
        )?;

        let mut local_identifier = LocalTraceIdentifier::new(known_contracts);
        let mut decoder = CallTraceDecoderBuilder::new()
            .with_labels(self.execution_result.labeled_addresses.clone())
            .with_verbosity(verbosity)
            .with_local_identifier_abis(&local_identifier)
            .with_signature_identifier(SignaturesIdentifier::new(
                Config::foundry_cache_dir(),
                self.script_config.config.offline,
            )?)
            .build();

        // Decoding traces using etherscan is costly as we run into rate limits,
        // causing scripts to run for a very long time unnecessarily.
        // Therefore, we only try and use etherscan if the user has provided an API key.
        let should_use_etherscan_traces = self.script_config.config.etherscan_api_key.is_some();

        for (_, trace) in &self.execution_result.traces {
            decoder.identify(trace, &mut local_identifier);
            if should_use_etherscan_traces {
                decoder.identify(trace, &mut etherscan_identifier);
            }
        }

        Ok(decoder)
    }

    fn get_returns(&self) -> Result<HashMap<String, NestedValue>> {
        let mut returns = HashMap::new();
        let returned = &self.execution_result.returned;
        let func = &self.execution_data.func;

        match func.abi_decode_output(returned, false) {
            Ok(decoded) => {
                for (index, (token, output)) in decoded.iter().zip(&func.outputs).enumerate() {
                    let internal_type =
                        output.internal_type.clone().unwrap_or(InternalType::Other {
                            contract: None,
                            ty: "unknown".to_string(),
                        });

                    let label = if !output.name.is_empty() {
                        output.name.to_string()
                    } else {
                        index.to_string()
                    };

                    returns.insert(
                        label,
                        NestedValue {
                            internal_type: internal_type.to_string(),
                            value: format_token_raw(token),
                        },
                    );
                }
            }
            Err(_) => {
                shell::println(format!("{returned:?}"))?;
            }
        }

        Ok(returns)
    }
}

pub struct PreSimulationState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
    pub execution_artifacts: ExecutionArtifacts,
}

pub struct ExecutionArtifacts {
    pub known_contracts: ContractsByArtifact,
    pub decoder: CallTraceDecoder,
    pub returns: HashMap<String, NestedValue>,
}

impl PreSimulationState {
    pub fn show_json(&self) -> Result<()> {
        let result = &self.execution_result;

        let console_logs = decode_console_logs(&result.logs);
        let output = JsonResult {
            logs: console_logs,
            gas_used: result.gas_used,
            returns: self.execution_artifacts.returns.clone(),
        };
        let j = serde_json::to_string(&output)?;
        shell::println(j)?;

        if !self.execution_result.success {
            return Err(eyre::eyre!(
                "script failed: {}",
                RevertDecoder::new().decode(&self.execution_result.returned[..], None)
            ));
        }

        Ok(())
    }

    pub async fn show_traces(&self) -> Result<()> {
        let verbosity = self.script_config.evm_opts.verbosity;
        let func = &self.execution_data.func;
        let result = &self.execution_result;
        let decoder = &self.execution_artifacts.decoder;

        if !result.success || verbosity > 3 {
            if result.traces.is_empty() {
                warn!(verbosity, "no traces");
            }

            shell::println("Traces:")?;
            for (kind, trace) in &result.traces {
                let should_include = match kind {
                    TraceKind::Setup => verbosity >= 5,
                    TraceKind::Execution => verbosity > 3,
                    _ => false,
                } || !result.success;

                if should_include {
                    shell::println(render_trace_arena(trace, decoder).await?)?;
                }
            }
            shell::println(String::new())?;
        }

        if result.success {
            shell::println(format!("{}", Paint::green("Script ran successfully.")))?;
        }

        if self.script_config.evm_opts.fork_url.is_none() {
            shell::println(format!("Gas used: {}", result.gas_used))?;
        }

        if result.success && !result.returned.is_empty() {
            shell::println("\n== Return ==")?;
            match func.abi_decode_output(&result.returned, false) {
                Ok(decoded) => {
                    for (index, (token, output)) in decoded.iter().zip(&func.outputs).enumerate() {
                        let internal_type =
                            output.internal_type.clone().unwrap_or(InternalType::Other {
                                contract: None,
                                ty: "unknown".to_string(),
                            });

                        let label = if !output.name.is_empty() {
                            output.name.to_string()
                        } else {
                            index.to_string()
                        };
                        shell::println(format!(
                            "{}: {internal_type} {}",
                            label.trim_end(),
                            format_token(token)
                        ))?;
                    }
                }
                Err(_) => {
                    shell::println(format!("{:x?}", (&result.returned)))?;
                }
            }
        }

        let console_logs = decode_console_logs(&result.logs);
        if !console_logs.is_empty() {
            shell::println("\n== Logs ==")?;
            for log in console_logs {
                shell::println(format!("  {log}"))?;
            }
        }

        if !result.success {
            return Err(eyre::eyre!(
                "script failed: {}",
                RevertDecoder::new().decode(&result.returned[..], None)
            ));
        }

        Ok(())
    }

    pub fn run_debugger(&self) -> Result<()> {
        let mut debugger = Debugger::builder()
            .debug_arenas(self.execution_result.debug.as_deref().unwrap_or_default())
            .decoder(&self.execution_artifacts.decoder)
            .sources(self.build_data.build_data.sources.clone())
            .breakpoints(self.execution_result.breakpoints.clone())
            .build();
        debugger.try_run()?;
        Ok(())
    }
}
