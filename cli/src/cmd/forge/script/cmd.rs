use crate::cmd::{Cmd, ScriptSequence};

use ethers::{
    abi::Abi,
    prelude::ArtifactId,
    types::{transaction::eip2718::TypedTransaction, TransactionRequest, U256},
};
use forge::{
    decode::decode_console_logs,
    executor::opts::EvmOpts,
    trace::{identifier::LocalTraceIdentifier, CallTraceDecoderBuilder, TraceKind},
};

use foundry_config::{figment::Figment, Config};

use super::*;
use std::collections::{BTreeMap, VecDeque};
use yansi::Paint;

impl Cmd for ScriptArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        let fork_url =
            evm_opts.fork_url.as_ref().expect("You must provide an RPC URL (see --fork-url).");
        let nonce = if let Some(deployer) = self.deployer {
            foundry_utils::next_nonce(dbg!(deployer), fork_url, None)?
        } else {
            U256::zero()
        };

        let BuildOutput {
            target,
            mut contract,
            mut highlevel_known_contracts,
            mut predeploy_libraries,
            known_contracts: default_known_contracts,
        } = self.build(&config, &evm_opts, self.deployer, nonce)?;

        if !self.force_resume && !self.resume {
            let mut known_contracts = highlevel_known_contracts
                .iter()
                .map(|(id, c)| {
                    (
                        id.clone(),
                        (
                            c.abi.clone(),
                            c.deployed_bytecode
                                .clone()
                                .into_bytes()
                                .expect("not bytecode")
                                .to_vec(),
                        ),
                    )
                })
                .collect::<BTreeMap<ArtifactId, (Abi, Vec<u8>)>>();

            // execute once with default sender
            let mut result =
                self.execute(contract, &evm_opts, self.deployer, &predeploy_libraries, &config)?;

            let mut new_sender = None;

            if self.deployer.is_none() {
                if let Some(ref txs) = result.transactions {
                    // If we did any linking with assumed predeploys, we cant support multiple
                    // deployers.
                    //
                    // If we didn't do any linking with predeploys, then none of the contracts will
                    // change their code based on the sender, so we can skip relinking +
                    // reexecuting.
                    if !predeploy_libraries.is_empty() {
                        for tx in txs.iter() {
                            match tx {
                                TypedTransaction::Legacy(tx) => {
                                    if tx.to.is_none() {
                                        let sender = tx.from.expect("no sender");
                                        if let Some(ns) = new_sender {
                                            if sender != ns {
                                                panic!("You have more than one deployer who could deploy libraries. Set `--deployer` to define the only one.")
                                            }
                                        } else if sender != evm_opts.sender {
                                            new_sender = Some(sender);
                                        }
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                }
            }

            // reexecute with the correct deployer after relinking contracts
            if let Some(new_sender) = new_sender {
                // if we had a new sender that requires relinking, we need to
                // get the nonce mainnet for accurate addresses for predeploy libs
                let nonce = foundry_utils::next_nonce(new_sender, fork_url, None)?;

                // relink with new sender
                let BuildOutput {
                    target: _,
                    contract: c2,
                    highlevel_known_contracts: hkc,
                    predeploy_libraries: pl,
                    known_contracts: _default_known_contracts,
                } = self.link(default_known_contracts, new_sender, nonce)?;

                contract = c2;
                highlevel_known_contracts = hkc;
                predeploy_libraries = pl;

                known_contracts = highlevel_known_contracts
                    .iter()
                    .map(|(id, c)| {
                        (
                            id.clone(),
                            (
                                c.abi.clone(),
                                c.deployed_bytecode
                                    .clone()
                                    .into_bytes()
                                    .expect("not bytecode")
                                    .to_vec(),
                            ),
                        )
                    })
                    .collect::<BTreeMap<ArtifactId, (Abi, Vec<u8>)>>();

                let lib_deploy = predeploy_libraries
                    .iter()
                    .enumerate()
                    .map(|(i, bytes)| {
                        TypedTransaction::Legacy(TransactionRequest {
                            from: Some(new_sender),
                            data: Some(bytes.clone()),
                            nonce: Some(nonce + i),
                            ..Default::default()
                        })
                    })
                    .collect();

                result.transactions = Some(lib_deploy);

                let result2 = self.execute(
                    contract,
                    &evm_opts,
                    Some(new_sender),
                    &predeploy_libraries,
                    &config,
                )?;

                result.success &= result2.success;

                result.gas = result2.gas;
                result.logs = result2.logs;
                result.traces.extend(result2.traces);
                result.debug = result2.debug;
                result.labeled_addresses.extend(result2.labeled_addresses);
                match (&mut result.transactions, result2.transactions) {
                    (Some(txs), Some(new_txs)) => {
                        new_txs.iter().for_each(|tx| {
                            txs.push_back(TypedTransaction::Legacy(into_legacy(tx.clone())));
                        });
                    }
                    (None, Some(new_txs)) => {
                        result.transactions = Some(new_txs);
                    }
                    _ => {}
                }
            } else {
                let mut lib_deploy: VecDeque<TypedTransaction> = predeploy_libraries
                    .iter()
                    .enumerate()
                    .map(|(i, bytes)| {
                        TypedTransaction::Legacy(TransactionRequest {
                            from: self.deployer,
                            data: Some(bytes.clone()),
                            nonce: Some(nonce + i),
                            ..Default::default()
                        })
                    })
                    .collect();

                // prepend predeploy libraries
                if let Some(txs) = &mut result.transactions {
                    txs.iter().for_each(|tx| {
                        lib_deploy.push_back(TypedTransaction::Legacy(into_legacy(tx.clone())));
                    });
                    *txs = lib_deploy;
                }
            }

            // Identify addresses in each trace
            let local_identifier = LocalTraceIdentifier::new(&known_contracts);
            let mut decoder = CallTraceDecoderBuilder::new()
                .with_labels(result.labeled_addresses.clone())
                .build();
            for (_, trace) in &mut result.traces {
                decoder.identify(trace, &local_identifier);
            }

            if verbosity >= 3 {
                if result.traces.is_empty() {
                    eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
                }

                if !result.success && verbosity == 3 || verbosity > 3 {
                    println!("Full Script Traces:");
                    for (kind, trace) in &mut result.traces {
                        let should_include = match kind {
                            TraceKind::Setup => {
                                (verbosity >= 5) || (verbosity == 4 && !result.success)
                            }
                            TraceKind::Execution => verbosity > 3 || !result.success,
                            _ => false,
                        };

                        if should_include {
                            decoder.decode(trace);
                            println!("{}", trace);
                        }
                    }
                    println!();
                }
            }

            if result.success {
                println!("{}", Paint::green("Dry running script was successful."));
            } else {
                println!("{}", Paint::red("Dry running script failed."));
            }

            println!("Gas used: {}", result.gas);
            println!("== Logs ==");
            let console_logs = decode_console_logs(&result.logs);
            if !console_logs.is_empty() {
                for log in console_logs {
                    println!("  {}", log);
                }
            }

            println!("==========================");
            println!("Simulated On-chain Traces:\n");
            if let Some(txs) = result.transactions {
                if let Ok(gas_filled_txs) =
                    self.execute_transactions(txs, &evm_opts, &config, &mut decoder)
                {
                    println!("\n\n==========================");
                    if !result.success {
                        panic!("\nSIMULATION FAILED");
                    } else {
                        let txs = gas_filled_txs;
                        let mut deployment_sequence =
                            ScriptSequence::new(txs, &self.sig, &target, &config.out)?;
                        deployment_sequence.save()?;

                        if self.execute {
                            self.send_transactions(&mut deployment_sequence)?;
                        } else {
                            println!("\nSIMULATION COMPLETE. To broadcast these transactions, add --execute and wallet configuration(s) to the previous command. See forge script --help for more.");
                        }
                    }
                } else {
                    panic!("One or more transactions failed when simulating the on-chain version. Check the trace by re-running with `-vvv`")
                }
            } else {
                panic!("No onchain transactions generated in script");
            }
        } else {
            let mut deployment_sequence = ScriptSequence::load(&self.sig, &target, &config.out)?;
            self.send_transactions(&mut deployment_sequence)?;
        }

        Ok(())
    }
}
