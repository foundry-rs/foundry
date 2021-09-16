use structopt::StructOpt;

use dapp::MultiContractRunner;

use ansi_term::Colour;

mod dapp_opts;
use dapp_opts::{BuildOpts, Opts, Subcommands};

mod utils;

fn main() -> eyre::Result<()> {
    utils::subscriber();

    let opts = Opts::from_args();
    match opts.sub {
        Subcommands::Test {
            opts:
                BuildOpts {
                    contracts,
                    remappings,
                    remappings_env,
                    lib_paths,
                    out_path,
                    evm_version,
                    no_compile,
                },
            env,
            json,
            pattern,
        } => {
            let cfg = evm_version.cfg();
            let remappings = utils::merge(remappings, remappings_env);
            let lib_paths = utils::default_path(lib_paths)?;

            let runner = MultiContractRunner::new(
                &contracts,
                remappings,
                lib_paths,
                out_path,
                &cfg,
                env.gas_limit,
                env.vicinity(),
                no_compile,
            )?;
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
                        println!("{} {} (gas: {})", status, name, result.gas_used);
                    }
                }
            }
        }
        Subcommands::Build {
            opts:
                BuildOpts {
                    contracts,
                    remappings,
                    remappings_env,
                    lib_paths,
                    out_path,
                    evm_version: _,
                    no_compile,
                },
        } => {
            // build the contracts
            let remappings = utils::merge(remappings, remappings_env);
            let lib_paths = utils::default_path(lib_paths)?;
            // TODO: Do we also want to include the file path in the contract map so
            // that we're more compatible with dapptools' artifact?
            let contracts = MultiContractRunner::build(
                &contracts,
                remappings,
                lib_paths,
                out_path.clone(),
                no_compile,
            )?;

            let out_file = utils::open_file(out_path)?;

            // dump as json
            serde_json::to_writer(out_file, &contracts)?;
        }
    }

    Ok(())
}
