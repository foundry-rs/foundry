use ethers::{
    providers::Provider,
    solc::{remappings::Remapping, ArtifactOutput, Project},
};
use evm_adapters::{
    sputnik::{vicinity, ForkMemoryBackend},
    FAUCET_ACCOUNT,
};
use regex::Regex;
use sputnik::backend::Backend;
use structopt::StructOpt;

use dapp::MultiContractRunnerBuilder;

use ansi_term::Colour;
use ethers::types::U256;

mod dapp_opts;
use dapp_opts::{EvmType, Opts, Subcommands};

use crate::dapp_opts::FullContractInfo;
use std::{collections::HashMap, convert::TryFrom, path::Path, sync::Arc};

mod cmd;
mod utils;

#[tracing::instrument(err)]
fn main() -> eyre::Result<()> {
    utils::subscriber();

    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test {
            opts,
            env,
            json,
            pattern,
            evm_type,
            fork_url,
            fork_block_number,
            initial_balance,
            sender,
            ffi,
            verbosity,
        } => {
            // Setup the fuzzer
            // TODO: Add CLI Options to modify the persistence
            let cfg =
                proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
            let fuzzer = proptest::test_runner::TestRunner::new(cfg);

            // Set up the project
            let project = Project::try_from(&opts)?;

            // prepare the test builder
            let builder = MultiContractRunnerBuilder::default()
                .fuzzer(fuzzer)
                .initial_balance(initial_balance)
                .sender(sender);

            // run the tests depending on the chosen EVM
            match evm_type {
                #[cfg(feature = "sputnik-evm")]
                EvmType::Sputnik => {
                    use evm_adapters::sputnik::Executor;
                    use sputnik::backend::MemoryBackend;
                    let mut cfg = opts.evm_version.sputnik_cfg();

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
                        let init_state = backend.state().clone();
                        let backend = ForkMemoryBackend::new(
                            provider,
                            backend,
                            fork_block_number,
                            init_state,
                        );
                        Box::new(backend)
                    } else {
                        Box::new(backend)
                    };
                    let backend = Arc::new(backend);

                    let evm = Executor::new_with_cheatcodes(backend, env.gas_limit, &cfg, ffi);

                    test(builder, project, evm, pattern, json, verbosity)?;
                }
                #[cfg(feature = "evmodin-evm")]
                EvmType::EvmOdin => {
                    use evm_adapters::evmodin::EvmOdin;
                    use evmodin::tracing::NoopTracer;

                    let revision = opts.evm_version.evmodin_cfg();

                    // TODO: Replace this with a proper host. We'll want this to also be
                    // provided generically when we add the Forking host(s).
                    let host = env.evmodin_state();

                    let evm = EvmOdin::new(host, env.gas_limit, revision, NoopTracer);
                    test(builder, project, evm, pattern, json, verbosity)?;
                }
            }
        }
        Subcommands::Build { opts } => {
            let project = Project::try_from(&opts)?;
            let output = project.compile()?;
            if output.is_unchanged() {
                println!("no files changed, compilation skippped.");
            } else if output.has_compiler_errors() {
                // return the diagnostics error back to the user.
                eyre::bail!(output.to_string())
            } else {
                println!("success.");
            }
        }
        Subcommands::VerifyContract { contract, address, constructor_args } => {
            let FullContractInfo { path, name } = contract;
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(cmd::verify::run(path, name, address, constructor_args))?;
        }
        Subcommands::Create { contract: _, verify: _ } => {
            unimplemented!("Not yet implemented")
        }
        Subcommands::Update { lib } => {
            // TODO: Should we add some sort of progress bar here? Would be nice
            // but not a requirement.
            // open the repo
            let repo = git2::Repository::open(".")?;

            // if a lib is specified, open it
            if let Some(lib) = lib {
                println!("FOUND");
                println!("Updating submodule {:?}", lib);
                repo.find_submodule(
                    &lib.into_os_string().into_string().expect("invalid submodule path"),
                )?
                .update(true, None)?;
            } else {
                std::process::Command::new("git")
                    .args(&["submodule", "update", "--init", "--recursive"])
                    .spawn()?;
            }
        }
        // TODO: Make it work with updates?
        Subcommands::Install { dependencies } => {
            let repo = git2::Repository::open(".")?;
            let libs = std::path::Path::new("lib");

            dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
                let path = libs.join(&dep.name);
                println!(
                    "Installing {} in {:?}, (url: {}, tag: {:?})",
                    dep.name, path, dep.url, dep.tag
                );

                // get the submodule and clone it
                let mut submodule = repo.submodule(&dep.url, &path, true)?;
                submodule.clone(None)?;

                // get the repo & checkout the provided tag if necessary
                // ref: https://stackoverflow.com/a/67240436
                let submodule = submodule.open()?;
                if let Some(ref tag) = dep.tag {
                    let (object, reference) = submodule.revparse_ext(tag)?;
                    submodule.checkout_tree(&object, None).expect("Failed to checkout");

                    match reference {
                        // gref is an actual reference like branches or tags
                        Some(gref) => submodule.set_head(gref.name().unwrap()),
                        // this is a commit, not a reference
                        None => submodule.set_head_detached(object.id()),
                    }
                    .expect("Failed to set HEAD");
                }

                std::process::Command::new("git")
                    .args(&["submodule", "update", "--init", "--recursive"])
                    .spawn()?;

                // commit the submodule's installation
                let sig = repo.signature()?;
                let mut index = repo.index()?;
                // stage `.gitmodules` and the lib
                index.add_path(Path::new(".gitmodules"))?;
                index.add_path(&path)?;
                // need to write the index back to disk
                index.write()?;

                let id = index.write_tree().unwrap();
                let tree = repo.find_tree(id).unwrap();

                let message = if let Some(ref tag) = dep.tag {
                    format!("turbodapp install: {}\n\n{}", dep.name, tag)
                } else {
                    format!("turbodapp install: {}", dep.name)
                };

                // committing to the parent may make running the installation step
                // in parallel challenging..
                let parent = repo
                    .head()?
                    .resolve()?
                    .peel(git2::ObjectType::Commit)?
                    .into_commit()
                    .map_err(|err| eyre::eyre!("cannot get parent commit: {:?}", err))?;
                repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])?;

                Ok(())
            })?
        }
        Subcommands::Remappings { lib_paths, root } => {
            let root = root.unwrap_or_else(|| std::env::current_dir().unwrap());
            let root = std::fs::canonicalize(root)?;

            let lib_paths = if lib_paths.is_empty() { vec![root.join("lib")] } else { lib_paths };
            let remappings: Vec<_> = lib_paths
                .iter()
                .map(|path| Remapping::find_many(&path).unwrap())
                .flatten()
                .collect();
            remappings.iter().for_each(|x| println!("{}", x));
        }
    }

    Ok(())
}

fn test<A: ArtifactOutput + 'static, S: Clone, E: evm_adapters::Evm<S>>(
    builder: MultiContractRunnerBuilder,
    project: Project<A>,
    evm: E,
    pattern: Regex,
    json: bool,
    verbosity: u8,
) -> eyre::Result<HashMap<String, HashMap<String, dapp::TestResult>>> {
    let mut runner = builder.build(project, evm)?;

    let mut exit_code = 0;

    let results = runner.test(pattern)?;

    if json {
        let res = serde_json::to_string(&results)?;
        println!("{}", res);
    } else {
        // Dapptools-style printing of test results
        for (i, (contract_name, tests)) in results.iter().enumerate() {
            if i > 0 {
                println!()
            }
            if !tests.is_empty() {
                let term = if tests.len() > 1 { "tests" } else { "test" };
                println!("Running {} {} for {}", tests.len(), term, contract_name);
            }

            for (name, result) in tests {
                let status = if result.success {
                    Colour::Green.paint("[PASS]")
                } else {
                    // if an error is found, return a -1 exit code
                    exit_code = -1;
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

            if verbosity > 1 {
                println!();

                for (name, result) in tests {
                    let status = if result.success { "Success" } else { "Failure" };
                    println!("{}: {}", status, name);
                    println!();

                    for log in &result.logs {
                        println!("  {}", log);
                    }

                    println!();
                }
            }
        }
    }

    std::process::exit(exit_code);
}
