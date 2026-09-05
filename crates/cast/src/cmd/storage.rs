use crate::opts::parse_slot;
use alloy_ens::NameOrAddress;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use comfy_table::{
    Table,
    presets::{ASCII_FULL, ASCII_MARKDOWN},
};
use eyre::Result;
use foundry_cli::{
    opts::{BuildOpts, EtherscanOpts, RpcOpts},
    utils,
    utils::LoadConfig,
};
use foundry_common::{
    abi::find_source,
    compile::{ProjectCompiler, etherscan_project},
    shell,
};
use foundry_compilers::{
    Artifact, ArtifactId, Project, ProjectCompileOutput,
    artifacts::{ConfigurableContractArtifact, Contract, StorageLayout},
    compilers::{
        Compiler,
        solc::{Solc, SolcCompiler},
    },
};
use foundry_config::{
    Config,
    figment::{self, Metadata, Profile, value::Dict},
    impl_figment_convert_cast,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// The minimum Solc version for outputting storage layouts.
///
/// <https://github.com/ethereum/solidity/blob/develop/Changelog.md#065-2020-04-06>
const MIN_SOLC: Version = Version::new(0, 6, 5);

/// CLI arguments for `cast storage`.
#[derive(Clone, Debug, Parser)]
pub struct StorageArgs {
    /// The contract address.
    #[arg(value_parser = NameOrAddress::from_str)]
    address: NameOrAddress,

    /// The storage slot number. If not provided, it gets the full storage layout.
    #[arg(value_parser = parse_slot)]
    base_slot: Option<B256>,

    /// The storage offset from the base slot. If not provided, it is assumed to be zero.
    #[arg(value_parser = str::parse::<U256>, default_value_t = U256::ZERO)]
    offset: U256,

    /// The known proxy address. If provided, the storage layout is retrieved from this address.
    #[arg(long,value_parser = NameOrAddress::from_str)]
    proxy: Option<NameOrAddress>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short)]
    block: Option<BlockId>,

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    build: BuildOpts,

    /// Specify the solc version to compile with. Overrides detected version.
    #[arg(long, value_parser = Version::parse)]
    solc_version: Option<Version>,
}

impl_figment_convert_cast!(StorageArgs);

impl figment::Provider for StorageArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("StorageArgs")
    }

    fn data(&self) -> Result<figment::value::Map<Profile, Dict>, figment::Error> {
        let mut map = self.build.data()?;
        let dict = map.get_mut(&Config::selected_profile()).unwrap();
        dict.extend(self.rpc.dict());
        dict.extend(self.etherscan.dict());
        Ok(map)
    }
}

impl StorageArgs {
    pub async fn run(self) -> Result<()> {
        let config = self.load_config()?;

        let Self { address, base_slot, offset, block, build, .. } = self;
        let provider = utils::get_provider(&config)?;
        let address = address.resolve(&provider).await?;

        // Slot was provided, perform a simple RPC call
        if let Some(slot) = base_slot {
            let slot = U256::from_be_bytes(slot.0).saturating_add(offset);
            sh_println!(
                "{}",
                B256::from(
                    provider
                        .get_storage_at(address, slot)
                        .block_id(block.unwrap_or_default())
                        .await?
                )
            )?;
            return Ok(());
        }

        // No slot was provided: get deployed bytecode at given address
        let address_code =
            provider.get_code_at(address).block_id(block.unwrap_or_default()).await?;
        if address_code.is_empty() {
            eyre::bail!("Provided address has no deployed code and thus no storage");
        }

        // Check if we're in a forge project and if we can find the address' code
        let project = build.project()?;
        if project.paths.has_input_files()
            && let Some(artifact) =
                compile_local_storage_layout(&project, &address_code, shell::is_json())?
        {
            return fetch_and_print_storage(provider, address, block, &artifact).await;
        }

        let chain = utils::get_chain(config.chain, &provider).await?;
        let client = match config.get_etherscan_config_with_chain(Some(chain))? {
            Some(etherscan_config) => {
                etherscan_config.into_client_with_no_proxy(config.eth_rpc_no_proxy)?
            }
            None => {
                let api_key = self.etherscan.key().ok_or_else(|| {
                    eyre::eyre!("You must provide an Etherscan API key if you're fetching a remote contract's storage.")
                })?;
                foundry_block_explorers::Client::new(chain, api_key)?
            }
        };
        let source_address = match self.proxy {
            Some(proxy) => proxy.resolve(&provider).await?,
            None => address,
        };
        let source = find_source(client, source_address).await?;
        let metadata = source.items.first().unwrap();
        if metadata.is_vyper() {
            eyre::bail!("Contract at provided address is not a valid Solidity contract");
        }

        // Create or reuse a persistent cache for Etherscan sources; fall back to a temp dir.
        let mut root_path = Config::foundry_etherscan_chain_cache_dir(chain)
            .map(|cache_root| cache_root.join("sources").join(address.to_string()));
        if let Some(path) = &root_path
            && let Err(err) = std::fs::create_dir_all(path)
        {
            sh_warn!("Could not create etherscan cache dir, falling back to temp: {err}")?;
            root_path = None;
        }
        let _temp_dir;
        let root_path = match root_path {
            Some(path) => path,
            None => {
                _temp_dir = tempfile::tempdir()?;
                _temp_dir.path().to_path_buf()
            }
        };
        let mut project = etherscan_project(metadata, &root_path)?;
        add_storage_layout_output(&mut project);

        // Decide on compiler to use (user override -> metadata -> autodetect).
        let meta_version = metadata.compiler_version()?;
        let auto_detect = self.solc_version.is_none() && meta_version < MIN_SOLC;
        project.compiler.solc = Some(match self.solc_version {
            Some(user_version) => {
                if user_version < MIN_SOLC {
                    sh_warn!(
                        "The provided --solc-version is {user_version} while the minimum version for \
                         storage layouts is {MIN_SOLC} and as a result the output may be empty."
                    )?;
                }
                SolcCompiler::Specific(Solc::find_or_install(&user_version)?)
            }
            None if auto_detect => SolcCompiler::AutoDetect,
            None => SolcCompiler::Specific(Solc::find_or_install(&meta_version)?),
        });

        let find_artifact = |out: &ProjectCompileOutput| {
            out.artifacts()
                .find(|(name, _)| name == &metadata.contract_name)
                .map(|(_, artifact)| artifact.clone())
                .ok_or_else(|| eyre::eyre!("Could not find artifact"))
        };
        let out = ProjectCompiler::new().quiet(true).compile(&project)?;
        let mut artifact = find_artifact(&out)?;
        if auto_detect && artifact.storage_layout.as_ref().is_none_or(|l| l.storage.is_empty()) {
            // Try recompiling with the minimum version.
            sh_warn!(
                "The requested contract was compiled with {meta_version} while the minimum version \
                 for storage layouts is {MIN_SOLC} and as a result the output may be empty.",
            )?;
            project.compiler.solc = Some(SolcCompiler::Specific(Solc::find_or_install(&MIN_SOLC)?));
            if let Ok(out) = ProjectCompiler::new().quiet(true).compile(&project) {
                artifact = find_artifact(&out)?;
            }
        }

        fetch_and_print_storage(provider, address, block, &artifact).await
    }
}

/// Finds the local artifact matching `address_code` and produces its storage layout.
///
/// Human-readable output compiles only the target's source and imports when safe. JSON and unsafe
/// cases retain the full-project compile to preserve existing behavior.
fn compile_local_storage_layout(
    project: &Project,
    address_code: &Bytes,
    json: bool,
) -> Result<Option<ConfigurableContractArtifact>> {
    // The JSON output exposes compiler-assigned AST IDs, which change when the compilation unit is
    // reduced to the target's dependency graph. Preserve those IDs by retaining the full compile.
    let full_compile = json
        || project.build_info
        || !project.cache_path().is_file()
        || !project.paths.artifacts.is_dir();
    if !full_compile {
        let output = ProjectCompiler::new().quiet(false).compile(project)?;
        let Some((target, artifact)) =
            output.into_artifacts().find(|(_, artifact)| has_deployed_code(artifact, address_code))
        else {
            return Ok(None);
        };
        if artifact.storage_layout.is_some() {
            return Ok(Some(artifact));
        }
        if let Ok(output) = compile_target_storage_layout(project, &target)
            && let Some(artifact) = find_target_artifact(output, &target, address_code)
        {
            return Ok(Some(artifact));
        }
    }

    let output = compile_full_storage_layout(project, json)?;
    Ok(output
        .into_artifacts()
        .find_map(|(_, artifact)| has_deployed_code(&artifact, address_code).then_some(artifact)))
}

fn has_deployed_code(artifact: &ConfigurableContractArtifact, code: &Bytes) -> bool {
    artifact.get_deployed_bytecode_bytes().as_deref() == Some(code)
}

fn find_target_artifact(
    output: ProjectCompileOutput,
    target: &ArtifactId,
    address_code: &Bytes,
) -> Option<ConfigurableContractArtifact> {
    output.into_artifacts().find_map(|(id, artifact)| {
        (same_artifact(&id, target)
            && artifact.storage_layout.is_some()
            && has_deployed_code(&artifact, address_code))
        .then_some(artifact)
    })
}

fn compile_target_storage_layout(
    project: &Project,
    target: &ArtifactId,
) -> Result<ProjectCompileOutput> {
    let mut project = project.clone();
    project.no_artifacts = true;
    add_storage_layout_output(&mut project);
    ProjectCompiler::new().quiet(true).files([target.source.clone()]).compile(&project)
}

fn compile_full_storage_layout(project: &Project, quiet: bool) -> Result<ProjectCompileOutput> {
    let mut project = project.clone();
    add_storage_layout_output(&mut project);
    ProjectCompiler::new().quiet(quiet).compile(&project)
}

/// Returns whether two artifact IDs identify the same contract across compiler runs.
///
/// Changing the output selection changes the build ID, and compiling fewer files can change an
/// artifact path that was disambiguated due to a name collision. Neither can be used to match the
/// normal compile against the targeted storage-layout compile.
fn same_artifact(left: &ArtifactId, right: &ArtifactId) -> bool {
    left.name == right.name
        && left.source == right.source
        && left.version == right.version
        && left.profile == right.profile
}

/// Represents the value of a storage slot `eth_getStorageAt` call.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StorageValue {
    /// The slot number.
    slot: B256,
    /// The value as returned by `eth_getStorageAt`.
    raw_slot_value: B256,
}

impl StorageValue {
    /// Returns the value of the storage slot, applying the offset if necessary.
    fn value(&self, offset: i64, number_of_bytes: Option<usize>) -> B256 {
        let offset = offset as usize;
        let end = number_of_bytes.map_or(32, |n| (offset + n).min(32));
        // Reverse range, because the value is stored in big endian.
        B256::left_padding_from(&self.raw_slot_value[32 - end..32 - offset])
    }
}

/// Represents the storage layout of a contract and its values.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StorageReport {
    #[serde(flatten)]
    layout: StorageLayout,
    values: Vec<B256>,
}

async fn fetch_and_print_storage<P: Provider<AnyNetwork>>(
    provider: P,
    address: Address,
    block: Option<BlockId>,
    artifact: &ConfigurableContractArtifact,
) -> Result<()> {
    let Some(layout) = artifact.storage_layout.as_ref().filter(|l| !l.storage.is_empty()) else {
        sh_warn!("Storage layout is empty.")?;
        return Ok(());
    };
    let values = futures::future::try_join_all(layout.storage.iter().map(|storage_slot| async {
        let slot = B256::from(U256::from_str(&storage_slot.slot)?);
        let raw_slot_value = provider
            .get_storage_at(address, slot.into())
            .block_id(block.unwrap_or_default())
            .await?;
        let storage_type = layout.types.get(&storage_slot.storage_type);
        let value = StorageValue { slot, raw_slot_value: raw_slot_value.into() }.value(
            storage_slot.offset,
            storage_type.and_then(|t| t.number_of_bytes.parse::<usize>().ok()),
        );
        Ok::<_, eyre::Report>(value)
    }))
    .await?;

    if shell::is_json() {
        let report = StorageReport { layout: layout.clone(), values };
        sh_println!("{}", serde_json::to_string_pretty(&serde_json::to_value(report)?)?)?;
        return Ok(());
    }

    let mut table = Table::new();
    table.load_style(if shell::is_markdown() {
        ASCII_MARKDOWN
    } else {
        ASCII_FULL.with_rounded_corners()
    });
    table.set_header(["Name", "Type", "Slot", "Offset", "Bytes", "Value", "Hex Value", "Contract"]);
    for (slot, value) in layout.storage.iter().zip(values) {
        let storage_type = layout.types.get(&slot.storage_type);
        table.add_row([
            slot.label.as_str(),
            storage_type.map_or("?", |t| &t.label),
            &slot.slot,
            &slot.offset.to_string(),
            storage_type.map_or("?", |t| &t.number_of_bytes),
            &U256::from_be_bytes(value.0).to_string(),
            &value.to_string(),
            &slot.contract,
        ]);
    }
    sh_println!("\n{table}\n")?;
    Ok(())
}

fn add_storage_layout_output<C: Compiler<CompilerContract = Contract>>(project: &mut Project<C>) {
    project.artifacts.additional_values.storage_layout = true;
    project.update_output_selection(|selection| {
        for contract_selection in selection.0.values_mut() {
            for selection in contract_selection.values_mut() {
                selection.push("storageLayout".to_string());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::PathStyle;
    use foundry_config::{CompilationRestrictions, SettingsOverrides, filter::GlobMatcher};
    use foundry_test_utils::{
        TestProject,
        util::{OTHER_SOLC_VERSION, SOLC_VERSION},
    };
    use std::path::Path;

    fn test_project(name: &str) -> TestProject {
        let project = TestProject::new(name, PathStyle::Dapptools);
        foundry_test_utils::util::initialize(project.root());
        project
    }

    fn load_project(project: &TestProject) -> Project {
        load_project_with_config(project, Config::with_root(project.root()))
    }

    fn load_project_with_config(project: &TestProject, config: Config) -> Project {
        let project = config.canonic_at(project.root()).project().unwrap();
        assert!(project.paths.has_input_files(), "{:?}", project.paths);
        project
    }

    /// Compiles `project` and returns the artifact id and deployed code of `name` in `source`.
    fn compile_target(project: &Project, source: &Path, name: &str) -> (ArtifactId, Bytes) {
        let output = ProjectCompiler::new().quiet(true).compile(project).unwrap();
        let (target, artifact) =
            output.artifact_ids().find(|(id, _)| id.source == source && id.name == name).unwrap();
        (target, artifact.get_deployed_bytecode_bytes().unwrap().into_owned())
    }

    #[test]
    fn local_storage_layout_targets_exact_artifact_and_imports() {
        let prj = test_project("cast-storage-target");
        let base_path = prj.add_source("Base", "contract Base { uint256 baseValue; }");
        let unrelated_path = prj.add_source("Target", "contract Target { uint256 unrelated; }");
        let target_path = prj.add_source(
            "nested/Target",
            r#"
import "src/Base.sol";

contract Target is Base {
    uint256 value;

    function marker() external pure returns (bool) {
        return true;
    }
}
"#,
        );
        let project = load_project(&prj);
        let (target, address_code) = compile_target(&project, &target_path, "Target");
        let artifact_before = std::fs::read(&target.path).unwrap();
        let cache_before = std::fs::read(project.cache_path()).unwrap();

        let output = compile_target_storage_layout(&project, &target).unwrap();
        assert!(output.artifact_ids().any(|(id, _)| id.source == base_path));
        assert!(!output.artifact_ids().any(|(id, _)| id.source == unrelated_path));
        let artifact = find_target_artifact(output, &target, &address_code).unwrap();
        let labels = artifact
            .storage_layout
            .as_ref()
            .unwrap()
            .storage
            .iter()
            .map(|slot| slot.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["baseValue", "value"]);

        let artifact =
            compile_local_storage_layout(&project, &address_code, false).unwrap().unwrap();
        assert!(artifact.storage_layout.is_some());
        assert_eq!(std::fs::read(&target.path).unwrap(), artifact_before);
        assert_eq!(std::fs::read(project.cache_path()).unwrap(), cache_before);
    }

    #[test]
    fn local_storage_layout_rechecks_bytecode_after_source_change() {
        let prj = test_project("cast-storage-source-change");
        let target_path = prj.add_source("Target", "contract Target { uint256 originalValue; }");
        let project = load_project(&prj);
        let (target, address_code) = compile_target(&project, &target_path, "Target");

        std::fs::write(
            &target_path,
            format!(
                "// SPDX-License-Identifier: MIT\npragma solidity ={SOLC_VERSION};\ncontract Target {{ uint256 changedValue; }}\n"
            ),
        )
        .unwrap();

        let output = compile_target_storage_layout(&project, &target).unwrap();
        assert!(find_target_artifact(output, &target, &address_code).is_none());
        assert!(compile_local_storage_layout(&project, &address_code, false).unwrap().is_none());
    }

    #[test]
    fn local_storage_layout_preserves_compiler_profile() {
        let prj = test_project("cast-storage-profile");
        let target_path = prj.add_source("Profiled", "contract Profiled { uint256 value; }");
        let mut config = Config::with_root(prj.root());
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "optimized".to_string(),
            via_ir: Some(true),
            evm_version: None,
            optimizer: Some(true),
            optimizer_runs: Some(1),
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: GlobMatcher::from_str("src/Profiled.sol").unwrap(),
            version: None,
            via_ir: Some(true),
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: Some(1),
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
        let project = load_project_with_config(&prj, config);
        let (target, address_code) = compile_target(&project, &target_path, "Profiled");
        assert_eq!(target.profile, "optimized");

        let output = compile_target_storage_layout(&project, &target).unwrap();
        let (compiled, _) = output
            .artifact_ids()
            .find(|(id, artifact)| {
                same_artifact(id, &target) && has_deployed_code(artifact, &address_code)
            })
            .unwrap();
        assert_eq!(compiled.version, target.version);
        assert_eq!(compiled.profile, target.profile);
        assert!(find_target_artifact(output, &target, &address_code).is_some());
    }

    #[test]
    fn local_storage_layout_preserves_compiler_version_in_multi_version_project() {
        let prj = test_project("cast-storage-multi-version");
        let old_path = prj.add_raw_source(
            "Old",
            &format!(
                "// SPDX-License-Identifier: MIT\npragma solidity ={OTHER_SOLC_VERSION};\ncontract Old {{ uint256 oldValue; }}\n"
            ),
        );
        let new_path = prj.add_raw_source(
            "New",
            &format!(
                "// SPDX-License-Identifier: MIT\npragma solidity ={SOLC_VERSION};\ncontract New {{ uint256 newValue; }}\n"
            ),
        );
        let mut config = Config::with_root(prj.root());
        config.solc = None;
        let project = load_project_with_config(&prj, config);
        let (target, address_code) = compile_target(&project, &old_path, "Old");
        assert_eq!(target.version, Version::parse(OTHER_SOLC_VERSION).unwrap());

        let output = compile_target_storage_layout(&project, &target).unwrap();
        assert!(!output.artifact_ids().any(|(id, _)| id.source == new_path));
        assert!(find_target_artifact(output, &target, &address_code).is_some());
        let artifact =
            compile_local_storage_layout(&project, &address_code, false).unwrap().unwrap();
        assert_eq!(artifact.storage_layout.unwrap().storage[0].label, "oldValue");
    }

    #[test]
    fn local_storage_layout_preserves_full_json_ast_ids() {
        let prj = test_project("cast-storage-json-ast-ids");
        prj.add_source("First", "contract First { uint256 first; }");
        let target_path = prj.add_source("Target", "contract Target { uint256 value; }");
        let project = load_project(&prj);
        let (target, address_code) = compile_target(&project, &target_path, "Target");

        let targeted = find_target_artifact(
            compile_target_storage_layout(&project, &target).unwrap(),
            &target,
            &address_code,
        )
        .unwrap();
        let full = compile_local_storage_layout(&project, &address_code, true).unwrap().unwrap();

        assert_ne!(full.storage_layout, targeted.storage_layout);
        assert_eq!(full.storage_layout.unwrap().storage[0].label, "value");
    }

    #[test]
    fn local_storage_layout_uses_full_compile_with_build_info() {
        let prj = test_project("cast-storage-build-info");
        let target_path = prj.add_source("Target", "contract Target { uint256 value; }");
        let mut config = Config::with_root(prj.root());
        config.build_info = true;
        let project = load_project_with_config(&prj, config);
        let (_, address_code) = compile_target(&project, &target_path, "Target");

        let artifact =
            compile_local_storage_layout(&project, &address_code, true).unwrap().unwrap();
        assert!(artifact.storage_layout.is_some());
    }

    #[test]
    fn local_storage_layout_uses_full_compile_without_cache() {
        let prj = test_project("cast-storage-no-cache");
        let target_path = prj.add_source("Target", "contract Target { uint256 value; }");
        let project = load_project(&prj);
        let mut code_project = project.clone();
        code_project.no_artifacts = true;
        let (_, address_code) = compile_target(&code_project, &target_path, "Target");
        assert!(!project.cache_path().exists());

        let artifact =
            compile_local_storage_layout(&project, &address_code, true).unwrap().unwrap();
        assert!(artifact.storage_layout.is_some());
    }

    #[test]
    fn parse_storage_etherscan_api_key() {
        let args =
            StorageArgs::parse_from(["foundry-cli", "addr.eth", "--etherscan-api-key", "dummykey"]);
        assert_eq!(args.etherscan.key(), Some("dummykey".to_string()));

        unsafe {
            std::env::set_var("ETHERSCAN_API_KEY", "FXY");
        }
        let config = args.load_config().unwrap();
        unsafe {
            std::env::remove_var("ETHERSCAN_API_KEY");
        }
        assert_eq!(config.etherscan_api_key, Some("dummykey".to_string()));
        assert_eq!(config.get_etherscan_api_key(None).unwrap(), "dummykey".to_string());
    }
}
