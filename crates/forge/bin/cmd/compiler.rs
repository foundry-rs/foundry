use clap::{ArgAction, Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_compilers::Graph;
use foundry_config::Config;
use semver::Version;
use std::{collections::BTreeMap, path::PathBuf};

/// CLI arguments for `forge compiler`.
#[derive(Debug, Parser)]
pub struct CompilerArgs {
    #[command(subcommand)]
    pub sub: CompilerSubcommands,
}

impl CompilerArgs {
    pub async fn run(self) -> Result<()> {
        match self.sub {
            CompilerSubcommands::Resolve(args) => args.run().await,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CompilerSubcommands {
    #[command(visible_alias = "r")]
    /// Retrieves the resolved version(s) of the compiler within the project.
    Resolve(ResolveArgs),
}

/// CLI arguments for `forge compiler resolve`.
#[derive(Debug, Parser)]
pub struct ResolveArgs {
    /// The root directory
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// Verbosity of the output.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Verbosity levels:
    /// - 2: Print source paths.
    #[arg(long, short, verbatim_doc_comment, action = ArgAction::Count, help_heading = "Display options")]
    pub verbosity: u8,

    /// Print as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,
}

impl ResolveArgs {
    pub async fn run(self) -> Result<()> {
        let Self { root, verbosity, json } = self;
        let config = Config::load_with_root(root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let (sources, _) = graph.into_sources_by_version(
            project.offline,
            &project.locked_versions,
            &project.compiler,
        )?;

        let mut output: BTreeMap<String, Vec<(Version, Vec<String>)>> = BTreeMap::new();

        for (language, sources) in sources {
            let mut versions_with_paths: Vec<(Version, Vec<String>)> = sources
                .iter()
                .map(|(version, sources)| {
                    let paths: Vec<String> = sources
                        .iter()
                        .map(|(path_file, _)| {
                            path_file
                                .strip_prefix(&project.paths.root)
                                .unwrap_or(path_file)
                                .to_path_buf()
                                .display()
                                .to_string()
                        })
                        .collect();

                    (version.clone(), paths)
                })
                .collect();

            versions_with_paths.sort_by(|(v1, _), (v2, _)| Version::cmp(v1, v2));

            output.insert(language.to_string(), versions_with_paths);
        }

        if json {
            println!("{}", serde_json::to_string(&output)?);
            return Ok(());
        }

        for (language, versions) in &output {
            if verbosity < 1 {
                println!("{language}:");
            } else {
                println!("{language}:\n");
            }

            for (version, paths) in versions {
                if verbosity >= 1 {
                    println!("{version}:");
                    for (idx, path) in paths.iter().enumerate() {
                        if idx == paths.len() - 1 {
                            println!("└── {path}\n");
                        } else {
                            println!("├── {path}");
                        }
                    }
                } else {
                    println!("- {version}");
                }
            }

            if verbosity < 1 {
                println!();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use foundry_test_utils::{forgetest_init, snapbox::IntoData, str};

    forgetest_init!(can_resolve_path, |prj, cmd| {
        cmd.args(["compiler", "resolve", prj.root().to_str().unwrap()]).assert_success().stdout_eq(
            str![[r#"
Solidity:
- 0.8.27


"#]],
        );
    });

    forgetest_init!(can_list_resolved_compiler_versions, |prj, cmd| {
        cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.27


"#]]);
    });

    forgetest_init!(can_list_resolved_compiler_versions_verbose, |prj, cmd| {
        cmd.args(["compiler", "resolve", "-v"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.27:
├── lib/forge-std/src/Base.sol
├── lib/forge-std/src/Script.sol
├── lib/forge-std/src/StdAssertions.sol
├── lib/forge-std/src/StdChains.sol
├── lib/forge-std/src/StdCheats.sol
├── lib/forge-std/src/StdError.sol
├── lib/forge-std/src/StdInvariant.sol
├── lib/forge-std/src/StdJson.sol
├── lib/forge-std/src/StdMath.sol
├── lib/forge-std/src/StdStorage.sol
├── lib/forge-std/src/StdStyle.sol
├── lib/forge-std/src/StdToml.sol
├── lib/forge-std/src/StdUtils.sol
├── lib/forge-std/src/Test.sol
├── lib/forge-std/src/Vm.sol
├── lib/forge-std/src/console.sol
├── lib/forge-std/src/console2.sol
├── lib/forge-std/src/interfaces/IERC165.sol
├── lib/forge-std/src/interfaces/IERC20.sol
├── lib/forge-std/src/interfaces/IERC721.sol
├── lib/forge-std/src/interfaces/IMulticall3.sol
├── lib/forge-std/src/mocks/MockERC20.sol
├── lib/forge-std/src/mocks/MockERC721.sol
├── lib/forge-std/src/safeconsole.sol
├── script/Counter.s.sol
├── src/Counter.sol
└── test/Counter.t.sol


"#]]);
    });

    forgetest_init!(can_list_resolved_compiler_versions_json, |prj, cmd| {
        cmd.args(["compiler", "resolve", "--json"])
            .assert_success()
            .stdout_eq(str![[r#"
{"Solidity":[["0.8.27",["lib/forge-std/src/Base.sol","lib/forge-std/src/Script.sol","lib/forge-std/src/StdAssertions.sol","lib/forge-std/src/StdChains.sol","lib/forge-std/src/StdCheats.sol","lib/forge-std/src/StdError.sol","lib/forge-std/src/StdInvariant.sol","lib/forge-std/src/StdJson.sol","lib/forge-std/src/StdMath.sol","lib/forge-std/src/StdStorage.sol","lib/forge-std/src/StdStyle.sol","lib/forge-std/src/StdToml.sol","lib/forge-std/src/StdUtils.sol","lib/forge-std/src/Test.sol","lib/forge-std/src/Vm.sol","lib/forge-std/src/console.sol","lib/forge-std/src/console2.sol","lib/forge-std/src/interfaces/IERC165.sol","lib/forge-std/src/interfaces/IERC20.sol","lib/forge-std/src/interfaces/IERC721.sol","lib/forge-std/src/interfaces/IMulticall3.sol","lib/forge-std/src/mocks/MockERC20.sol","lib/forge-std/src/mocks/MockERC721.sol","lib/forge-std/src/safeconsole.sol","script/Counter.s.sol","src/Counter.sol","test/Counter.t.sol"]]]}"#]].is_jsonlines());
    });

    const CONTRACT_A: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.4;

contract ContractA {}
"#;

    const CONTRACT_B: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.11;

contract ContractB {}
"#;

    const VYPER_INTERFACE: &str = r#"
# pragma version ^0.3.10

@external
@view
def number() -> uint256:
    return empty(uint256)

@external
def set_number(new_number: uint256):
    pass

@external
def increment() -> uint256:
    return empty(uint256)
"#;

    const VYPER_CONTRACT: &str = r#"
import ICounter
implements: ICounter

number: public(uint256)

@external
def set_number(new_number: uint256):
    self.number = new_number

@external
def increment() -> uint256:
    self.number += 1
    return self.number
"#;

    forgetest_init!(can_list_resolved_multiple_compiler_versions, |prj, cmd| {
        prj.add_source("ContractA", CONTRACT_A).unwrap();
        prj.add_source("ContractB", CONTRACT_B).unwrap();
        prj.add_raw_source("ICounter.vy", VYPER_INTERFACE).unwrap();
        prj.add_raw_source("Counter.vy", VYPER_CONTRACT).unwrap();

        cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4
- 0.8.11
- 0.8.27

Vyper:
- 0.4.0


"#]]);
    });

    forgetest_init!(can_list_resolved_multiple_compiler_versions_verbose, |prj, cmd| {
        prj.add_source("ContractA", CONTRACT_A).unwrap();
        prj.add_source("ContractB", CONTRACT_B).unwrap();
        prj.add_raw_source("ICounter.vy", VYPER_INTERFACE).unwrap();
        prj.add_raw_source("Counter.vy", VYPER_CONTRACT).unwrap();

        cmd.args(["compiler", "resolve", "-v"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.4:
└── src/ContractA.sol

0.8.11:
└── src/ContractB.sol

0.8.27:
├── lib/forge-std/src/Base.sol
├── lib/forge-std/src/Script.sol
├── lib/forge-std/src/StdAssertions.sol
├── lib/forge-std/src/StdChains.sol
├── lib/forge-std/src/StdCheats.sol
├── lib/forge-std/src/StdError.sol
├── lib/forge-std/src/StdInvariant.sol
├── lib/forge-std/src/StdJson.sol
├── lib/forge-std/src/StdMath.sol
├── lib/forge-std/src/StdStorage.sol
├── lib/forge-std/src/StdStyle.sol
├── lib/forge-std/src/StdToml.sol
├── lib/forge-std/src/StdUtils.sol
├── lib/forge-std/src/Test.sol
├── lib/forge-std/src/Vm.sol
├── lib/forge-std/src/console.sol
├── lib/forge-std/src/console2.sol
├── lib/forge-std/src/interfaces/IERC165.sol
├── lib/forge-std/src/interfaces/IERC20.sol
├── lib/forge-std/src/interfaces/IERC721.sol
├── lib/forge-std/src/interfaces/IMulticall3.sol
├── lib/forge-std/src/mocks/MockERC20.sol
├── lib/forge-std/src/mocks/MockERC721.sol
├── lib/forge-std/src/safeconsole.sol
├── script/Counter.s.sol
├── src/Counter.sol
└── test/Counter.t.sol

Vyper:

0.4.0:
├── src/Counter.vy
└── src/ICounter.vy


"#]]);
    });

    forgetest_init!(can_list_resolved_multiple_compiler_versions_json, |prj, cmd| {
        prj.add_source("ContractA", CONTRACT_A).unwrap();
        prj.add_source("ContractB", CONTRACT_B).unwrap();
        prj.add_raw_source("ICounter.vy", VYPER_INTERFACE).unwrap();
        prj.add_raw_source("Counter.vy", VYPER_CONTRACT).unwrap();

        // {
        //     "Solidity":[
        //         [
        //             "0.8.4",
        //             [
        //                 "src/ContractA.sol"
        //             ]
        //         ],
        //         [
        //             "0.8.11",
        //             [
        //                 "src/ContractB.sol"
        //             ]
        //         ],
        //         [
        //             "0.8.27",
        //             [
        //                 "lib/forge-std/src/Base.sol",
        //                 "lib/forge-std/src/Script.sol",
        //                 "lib/forge-std/src/StdAssertions.sol",
        //                 "lib/forge-std/src/StdChains.sol",
        //                 "lib/forge-std/src/StdCheats.sol",
        //                 "lib/forge-std/src/StdError.sol",
        //                 "lib/forge-std/src/StdInvariant.sol",
        //                 "lib/forge-std/src/StdJson.sol",
        //                 "lib/forge-std/src/StdMath.sol",
        //                 "lib/forge-std/src/StdStorage.sol",
        //                 "lib/forge-std/src/StdStyle.sol",
        //                 "lib/forge-std/src/StdToml.sol",
        //                 "lib/forge-std/src/StdUtils.sol",
        //                 "lib/forge-std/src/Test.sol",
        //                 "lib/forge-std/src/Vm.sol",
        //                 "lib/forge-std/src/console.sol",
        //                 "lib/forge-std/src/console2.sol",
        //                 "lib/forge-std/src/interfaces/IERC165.sol",
        //                 "lib/forge-std/src/interfaces/IERC20.sol",
        //                 "lib/forge-std/src/interfaces/IERC721.sol",
        //                 "lib/forge-std/src/interfaces/IMulticall3.sol",
        //                 "lib/forge-std/src/mocks/MockERC20.sol",
        //                 "lib/forge-std/src/mocks/MockERC721.sol",
        //                 "lib/forge-std/src/safeconsole.sol",
        //                 "script/Counter.s.sol",
        //                 "src/Counter.sol",
        //                 "test/Counter.t.sol"
        //             ]
        //         ]
        //     ],
        //     "Vyper":[
        //         [
        //             "0.4.0",
        //             [
        //                 "src/Counter.vy",
        //                 "src/ICounter.vy"
        //             ]
        //         ]
        //     ]
        // }
        cmd.args(["compiler", "resolve", "--json"])
            .assert_success()
            .stdout_eq(str![[r#"
{"Solidity":[["0.8.4",["src/ContractA.sol"]],["0.8.11",["src/ContractB.sol"]],["0.8.27",["lib/forge-std/src/Base.sol","lib/forge-std/src/Script.sol","lib/forge-std/src/StdAssertions.sol","lib/forge-std/src/StdChains.sol","lib/forge-std/src/StdCheats.sol","lib/forge-std/src/StdError.sol","lib/forge-std/src/StdInvariant.sol","lib/forge-std/src/StdJson.sol","lib/forge-std/src/StdMath.sol","lib/forge-std/src/StdStorage.sol","lib/forge-std/src/StdStyle.sol","lib/forge-std/src/StdToml.sol","lib/forge-std/src/StdUtils.sol","lib/forge-std/src/Test.sol","lib/forge-std/src/Vm.sol","lib/forge-std/src/console.sol","lib/forge-std/src/console2.sol","lib/forge-std/src/interfaces/IERC165.sol","lib/forge-std/src/interfaces/IERC20.sol","lib/forge-std/src/interfaces/IERC721.sol","lib/forge-std/src/interfaces/IMulticall3.sol","lib/forge-std/src/mocks/MockERC20.sol","lib/forge-std/src/mocks/MockERC721.sol","lib/forge-std/src/safeconsole.sol","script/Counter.s.sol","src/Counter.sol","test/Counter.t.sol"]]],"Vyper":[["0.4.0",["src/Counter.vy","src/ICounter.vy"]]]}
"#]].is_jsonlines());
    });
}
