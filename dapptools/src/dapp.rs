use ethers::prelude::Provider;
use evm_adapters::sputnik::ForkMemoryBackend;
use regex::Regex;
use structopt::StructOpt;

use dapp::MultiContractRunnerBuilder;
use dapp_solc::SolcBuilder;

use ansi_term::Colour;

mod dapp_opts;
use dapp_opts::{BuildOpts, EvmType, Opts, Subcommands};

use std::convert::TryFrom;

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
        } => {
            // get the remappings / paths
            let remappings = utils::merge(remappings, remappings_env);
            let lib_paths = utils::default_path(lib_paths)?;

            // prepare the builder
            let builder = MultiContractRunnerBuilder::default()
                .contracts(&contracts)
                .remappings(&remappings)
                .libraries(&lib_paths)
                .out_path(out_path)
                .fuzzer(proptest::test_runner::TestRunner::default())
                .skip_compilation(no_compile);

            // run the tests depending on the chosen EVM
            match evm_type {
                #[cfg(feature = "sputnik-evm")]
                EvmType::Sputnik => {
                    use evm_adapters::sputnik::Executor;
                    use sputnik::backend::MemoryBackend;
                    let cfg = evm_version.sputnik_cfg();

                    if let Some(url) = fork_url {
                        let provider = Provider::try_from(url.as_str())?;
                        // TODO: Replace Default with something that can be read from disk, e.g.
                        // some pre-loaded state snapshot from another time?
                        let backend =
                            ForkMemoryBackend::new(provider, fork_block_number, Default::default());
                        let evm = Executor::new(env.gas_limit, &cfg, &backend);

                        test(builder, evm, pattern, json)?;
                    } else {
                        let vicinity = env.sputnik_state();
                        let backend = MemoryBackend::new(&vicinity, Default::default());
                        let evm = Executor::new(env.gas_limit, &cfg, &backend);
                        test(builder, evm, pattern, json)?;
                    }
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
    }

    Ok(())
}

fn test<S, E: Clone + evm_adapters::Evm<S>>(
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
                    Colour::Red.paint("[FAIL]")
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
