use ethers::prelude::Provider;
use evm_adapters::{
    sputnik::{vicinity, ForkMemoryBackend},
    FAUCET_ACCOUNT,
};
use regex::Regex;
use sputnik::backend::Backend;
use structopt::StructOpt;

use dapp::MultiContractRunnerBuilder;
use dapp_solc::SolcBuilder;

use ansi_term::Colour;
use ethers::types::U256;

mod dapp_opts;
use dapp_opts::{BuildOpts, EvmType, Opts, Subcommands};

use crate::dapp_opts::FullContractInfo;
use std::{convert::TryFrom, sync::Arc};

mod cmd;
mod utils;

#[tracing::instrument(err)]
fn main() -> eyre::Result<()> {
    utils::subscriber();

    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test {
            opts:
                BuildOpts { contracts, remappings, remappings_env, lib_paths, out_path, evm_version },
            env,
            json,
            pattern,
            evm_type,
            no_compile,
            fork_url,
            fork_block_number,
            initial_balance,
            deployer,
        } => {
            // get the remappings / paths
            let remappings = utils::merge(remappings, remappings_env);
            let lib_paths = utils::default_path(lib_paths)?;

            // TODO: Add CLI Options to modify the persistence
            let cfg =
                proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
            let fuzzer = proptest::test_runner::TestRunner::new(cfg);

            // prepare the builder
            let builder = MultiContractRunnerBuilder::default()
                .contracts(&contracts)
                .remappings(&remappings)
                .libraries(&lib_paths)
                .out_path(out_path)
                .fuzzer(fuzzer)
                .initial_balance(initial_balance)
                .deployer(deployer)
                .skip_compilation(no_compile);

            // run the tests depending on the chosen EVM
            match evm_type {
                #[cfg(feature = "sputnik-evm")]
                EvmType::Sputnik => {
                    use evm_adapters::sputnik::Executor;
                    use sputnik::backend::MemoryBackend;
                    let mut cfg = evm_version.sputnik_cfg();

                    // We disable the contract size limit by default, because Solidity
                    // test smart contracts are likely to be >24kb
                    cfg.create_contract_limit = None;

                    let vicinity = if let Some(ref url) = fork_url {
                        let provider = Provider::try_from(url.as_str())?;
                        let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
                        rt.block_on(vicinity(&provider, fork_block_number))?
                    } else {
                        env.sputnik_state()
                    };
                    let mut backend = MemoryBackend::new(&vicinity, Default::default());
                    // max out the balance of the faucet
                    let faucet =
                        backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
                    faucet.balance = U256::MAX;

                    let backend: Box<dyn Backend> = if let Some(ref url) = fork_url {
                        let provider = Provider::try_from(url.as_str())?;
                        let backend = ForkMemoryBackend::new(provider, backend, fork_block_number);
                        Box::new(backend)
                    } else {
                        Box::new(backend)
                    };
                    let backend = Arc::new(backend);

                    let evm = Executor::new_with_cheatcodes(backend, env.gas_limit, &cfg);
                    test(builder, evm, pattern, json)?;
                }
                #[cfg(feature = "evmodin-evm")]
                EvmType::EvmOdin => {
                    use evm_adapters::evmodin::EvmOdin;
                    use evmodin::tracing::NoopTracer;

                    let revision = evm_version.evmodin_cfg();

                    // TODO: Replace this with a proper host. We'll want this to also be
                    // provided generically when we add the Forking host(s).
                    let host = env.evmodin_state();

                    let evm = EvmOdin::new(host, env.gas_limit, revision, NoopTracer);
                    test(builder, evm, pattern, json)?;
                }
            }
        }
        Subcommands::Build {
            opts:
                BuildOpts { contracts, remappings, remappings_env, lib_paths, out_path, evm_version: _ },
        } => {
            // build the contracts
            let remappings = utils::merge(remappings, remappings_env);
            let lib_paths = utils::default_path(lib_paths)?;
            // TODO: Do we also want to include the file path in the contract map so
            // that we're more compatible with dapptools' artifact?
            let contracts = SolcBuilder::new(&contracts, &remappings, &lib_paths)?.build_all()?;

            let out_file = utils::open_file(out_path)?;

            // dump as json
            serde_json::to_writer(out_file, &contracts)?;
        }
        Subcommands::VerifyContract { contract, address, constructor_args } => {
            let FullContractInfo { path, name } = contract;
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(cmd::verify::run(path, name, address, constructor_args))?;
        }
        Subcommands::Create { contract: _, verify: _ } => {
            unimplemented!("Not yet implemented")
        }
    }

    Ok(())
}

fn test<S: Clone, E: evm_adapters::Evm<S>>(
    builder: MultiContractRunnerBuilder,
    evm: E,
    pattern: Regex,
    json: bool,
) -> eyre::Result<()> {
    let mut runner = builder.build(evm)?;

    let results = runner.test(pattern)?;

    if json {
        let res = serde_json::to_string(&results)?;
        println!("{}", res);
    } else {
        // Dapptools-style printing
        for (i, (contract_name, tests)) in results.iter().enumerate() {
            if i > 0 {
                println!()
            }
            if !tests.is_empty() {
                println!("Running {} tests for {}", tests.len(), contract_name);
            }

            for (name, result) in tests {
                let status = if result.success {
                    Colour::Green.paint("[PASS]")
                } else {
                    let txt = if let Some(ref reason) = result.reason {
                        format!("[FAIL: {}]", reason)
                    } else {
                        "[FAIL]".to_string()
                    };
                    Colour::Red.paint(txt)
                };
                println!(
                    "{} {} (gas: {})",
                    status,
                    name,
                    result
                        .gas_used
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| "[fuzztest]".to_string())
                );
            }
        }
    }

    Ok(())
}
