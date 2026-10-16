use alloy_json_abi::ToSolConfig;
use alloy_primitives::map::HashSet;
use clap::{Parser, ValueHint};
use eyre::Result;
use forge_sol_macro_gen::{MultiSolMacroGen, SolMacroGen};
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use foundry_common::{
    compile::{ProjectCompiler, compile_abi_project},
    fs::json_files,
};
use foundry_compilers::{
    Graph, Project,
    artifacts::{Contract, SolcLanguage, output_selection::OutputSelection},
    cache::CompilerCache,
    compilers::{Compiler, CompilerOutput, CompilerSettings, CompilerVersion},
    error::SolcError,
    multi::{MultiCompiler, MultiCompilerParser, MultiCompilerSettings},
    solc::SolcCompiler,
};
use foundry_config::{Config, impl_figment_convert};
use regex::Regex;
use semver::Version;
use solar::ast::{Item, ItemKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

impl_figment_convert!(BindArgs, build);

const DEFAULT_CRATE_NAME: &str = "foundry-contracts";
const DEFAULT_CRATE_VERSION: &str = "0.1.0";

type BindingFiles = Vec<(String, PathBuf)>;

/// CLI arguments for `forge bind`.
#[derive(Clone, Debug, Parser)]
pub struct BindArgs {
    /// Path to where the contract artifacts are stored.
    #[arg(
        long = "bindings-path",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    pub bindings: Option<PathBuf>,

    /// Create bindings only for contracts whose names match the specified filter(s)
    #[arg(long)]
    pub select: Vec<regex::Regex>,

    /// Explicitly generate bindings for all contracts
    ///
    /// By default all contracts ending with `Test` or `Script` are excluded.
    #[arg(long, conflicts_with_all = &["select", "skip"])]
    pub select_all: bool,

    /// The name of the Rust crate to generate.
    ///
    /// This should be a valid crates.io crate name,
    /// however, this is not currently validated by this command.
    #[arg(long, default_value = DEFAULT_CRATE_NAME, value_name = "NAME")]
    crate_name: String,

    /// The version of the Rust crate to generate.
    ///
    /// This should be a standard semver version string,
    /// however, this is not currently validated by this command.
    #[arg(long, default_value = DEFAULT_CRATE_VERSION, value_name = "VERSION")]
    crate_version: String,

    /// The description of the Rust crate to generate.
    ///
    /// This will be added to the package.description field in Cargo.toml.
    #[arg(long, default_value = "", value_name = "DESCRIPTION")]
    crate_description: String,

    /// The license of the Rust crate to generate.
    ///
    /// This will be added to the package.license field in Cargo.toml.
    #[arg(long, value_name = "LICENSE", default_value = "")]
    crate_license: String,

    /// Generate the bindings as a module instead of a crate.
    #[arg(long)]
    module: bool,

    /// Overwrite existing generated bindings.
    ///
    /// By default, the command will check that the bindings are correct, and then exit. If
    /// --overwrite is passed, it will instead delete and overwrite the bindings.
    #[arg(long)]
    overwrite: bool,

    /// Generate bindings as a single file.
    #[arg(long)]
    single_file: bool,

    /// Skip Cargo.toml consistency checks.
    #[arg(long)]
    skip_cargo_toml: bool,

    /// Skips running forge build before generating binding
    #[arg(long)]
    skip_build: bool,

    /// Don't add any additional derives to generated bindings
    #[arg(long)]
    skip_extra_derives: bool,

    /// Generate bindings for the `alloy` library, instead of `ethers`.
    #[arg(long, hide = true)]
    alloy: bool,

    /// Specify the `alloy` version on Crates.
    #[arg(long)]
    alloy_version: Option<String>,

    /// Specify the `alloy` revision on GitHub.
    #[arg(long, conflicts_with = "alloy_version")]
    alloy_rev: Option<String>,

    /// Generate bindings for the `ethers` library (removed), instead of `alloy`.
    #[arg(long, hide = true)]
    ethers: bool,

    #[command(flatten)]
    build: BuildOpts,
}

impl BindArgs {
    pub fn run(self) -> Result<()> {
        if self.ethers {
            eyre::bail!("`--ethers` bindings have been removed. Use `--alloy` (default) instead.");
        }

        let config = if self.skip_build {
            self.load_config()?
        } else {
            self.load_config_with_dependencies()?
        };
        let artifacts = config.out.clone();
        let (files, enum_definitions) = if self.skip_build {
            self.cached_bindings(&config)?
        } else {
            let mut project = config.project()?;
            // Compilation may install compiler versions and change subsequent auto-detection.
            let contexts = resolved_artifact_contexts(&project, artifact_graph(&project, None)?)?;
            let output = compile_abi_project(
                &mut project,
                ProjectCompiler::new().external_compilers(&config),
            )?;
            let files =
                self.filter_artifact_paths(output.artifact_ids().filter_map(|(id, _)| {
                    (id.path.starts_with(artifacts.join(".external"))
                        || contexts
                            .get(&id.source)
                            .is_some_and(|contexts| contexts.contains(&(id.version, id.profile))))
                    .then_some(id.path)
                }))?;
            (files, enum_definitions(output.parser()))
        };

        let bindings_root = self.bindings.clone().unwrap_or_else(|| artifacts.join("bindings"));
        let sol_config = ToSolConfig::new().enum_definitions(enum_definitions);

        if bindings_root.exists() {
            if !self.overwrite {
                sh_status!("Bindings found. Checking for consistency.")?;
                let mut bindings = self.get_solmacrogen(&files)?;
                bindings.generate_bindings(!self.skip_extra_derives, &sol_config)?;
                return self.check_existing_bindings(&bindings, &bindings_root);
            }

            trace!(?artifacts, "Removing existing bindings");
            fs::remove_dir_all(&bindings_root)?;
        }

        self.generate_bindings(&files, &bindings_root, &sol_config)?;

        sh_status!("Bindings have been generated to {}", bindings_root.display())?;
        Ok(())
    }

    fn get_filter(&self) -> Result<Filter> {
        if self.select_all {
            // Select all json files
            return Ok(Filter::All);
        }
        if !self.select.is_empty() {
            // Return json files that match the select regex
            return Ok(Filter::Select(self.select.clone()));
        }

        if let Some(skip) = self.build.skip.as_ref().filter(|s| !s.is_empty()) {
            return Ok(Filter::Skip(
                skip.clone()
                    .into_iter()
                    .map(|s| Regex::new(s.file_pattern()))
                    .collect::<Result<Vec<_>, _>>()?,
            ));
        }

        // Exclude defaults
        Ok(Filter::skip_default())
    }

    /// Selects cached artifacts and enriches enums only from a fresh, complete source graph.
    fn cached_bindings(
        &self,
        config: &Config,
    ) -> Result<(BindingFiles, BTreeMap<String, Vec<String>>)> {
        let paths = config.project_paths();
        let cache = CompilerCache::<MultiCompilerSettings>::read_joined(&paths).ok();
        let graph = Graph::<MultiCompilerParser>::resolve(&paths);
        let mut definitions = if let (Some(cache), Ok(graph)) = (&cache, &graph)
            && cache.all_artifacts_exist()
            && graph.nodes.iter().all(|node| {
                cache
                    .entry(node.path())
                    .is_some_and(|entry| entry.content_hash == node.unpack().1.content_hash())
            }) {
            enum_definitions(graph.parser())
        } else {
            BTreeMap::default()
        };
        let files = if let Some(cache) = cache.as_ref().filter(|_| config.cache) {
            let filter = self.get_filter()?;
            let selected = cache
                .files
                .iter()
                .filter(|(_, entry)| {
                    entry.artifacts.keys().any(|name| filter.is_match(&binding_name(name)))
                })
                .map(|(source, _)| source.clone())
                .collect::<HashSet<_>>();
            let mut files = Vec::new();
            if !selected.is_empty() {
                let compiler = CachedCompiler::new(config, cache);
                let mut project = config.create_project_with_compiler(compiler, false, true)?;
                project.update_output_selection(|selection| {
                    *selection = OutputSelection::common_output_selection(["abi".to_string()]);
                });
                // Enum metadata always uses the full graph; artifact selection may be narrower.
                let graph = if project.sparse_output.is_none() {
                    graph.map_err(Into::into)
                } else {
                    artifact_graph(&project, None)
                };
                // Preserve shared compiler versions, falling back to selected inputs if needed.
                let contexts = graph
                    .and_then(|graph| resolved_artifact_contexts(&project, graph))
                    .or_else(|_| {
                        let graph = artifact_graph(&project, Some(&selected))?;
                        resolved_artifact_contexts(&project, graph)
                    })?;
                for (source, contexts) in contexts {
                    if let Some(entry) = cache.entry(&source) {
                        for name in entry
                            .artifacts
                            .keys()
                            .filter(|name| filter.is_match(&binding_name(name)))
                        {
                            let vyper = matches!(
                                source.extension().and_then(|s| s.to_str()),
                                Some("vy" | "vyi")
                            );
                            let unavailable = if vyper {
                                project.compiler.vyper_unknown
                            } else {
                                project.compiler.solc_unknown
                            };
                            eyre::ensure!(
                                !unavailable || entry.artifacts[name].len() <= 1,
                                "Cannot determine compiler version for {}:{name}: {} is unavailable and multiple versions are cached; {}",
                                source.display(),
                                if vyper { "Vyper" } else { "configured solc" },
                                if vyper {
                                    "configure `vyper.path`"
                                } else {
                                    "specify `--use <version>`"
                                },
                            );
                            for (version, profile) in &contexts {
                                let artifact = entry.find_artifact(name, version, profile);
                                eyre::ensure!(
                                    artifact.is_some_and(|artifact| artifact.path.is_file()),
                                    "No cached artifact for {}:{name} with compiler {version} and profile `{profile}`; run `forge build` first",
                                    source.display(),
                                );
                                let settings = project
                                    .settings_profiles()
                                    .find(|(name, _)| name == profile)
                                    .map(|(_, settings)| settings)
                                    .expect("resolved profile is configured");
                                eyre::ensure!(
                                    cache
                                        .profiles
                                        .get(profile)
                                        .is_some_and(|cached| settings.can_use_cached(cached)),
                                    "Cached compiler settings for {}:{name} and profile `{profile}` do not match; run `forge build` first",
                                    source.display(),
                                );
                                files.push(artifact.unwrap().path.clone());
                            }
                        }
                    }
                }
            }
            // External adapter artifacts are not part of the built-in compiler cache.
            files.extend(json_files(&config.out.join(".external")));
            files
        } else {
            // Artifact-only directories have no compiler manifest to resolve version/profile
            // identity.
            json_files(&config.out).collect()
        };
        let files = self.filter_artifact_paths(files)?;
        // Cache-based selection already guarantees membership for built-in artifacts.
        if let Some(cache) = cache.filter(|_| !config.cache)
            && !definitions.is_empty()
        {
            let cached_artifacts = cache
                .entries()
                .flat_map(|entry| entry.artifacts())
                .map(|artifact| &artifact.path)
                .collect::<HashSet<_>>();
            if files.iter().any(|(_, path)| {
                !path.starts_with(config.out.join(".external")) && !cached_artifacts.contains(path)
            }) {
                definitions.clear();
            }
        }
        Ok((files, definitions))
    }

    /// Applies binding filters and preserves artifact-path precedence for duplicate names.
    fn filter_artifact_paths(
        &self,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<BindingFiles> {
        let filter = self.get_filter()?;
        let mut files = paths
            .into_iter()
            .filter_map(|path| {
                if path.to_str()?.contains("build-info")
                    || path.iter().any(|component| component == "target")
                {
                    return None;
                }
                let stem = path.file_stem()?.to_str()?;
                if stem.ends_with(".metadata") {
                    return None;
                }
                let name = binding_name(stem.split('.').next().unwrap());
                filter.is_match(&name).then_some((name, path))
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(files)
    }

    fn get_solmacrogen(&self, files: &[(String, PathBuf)]) -> Result<MultiSolMacroGen> {
        let mut dup = HashSet::<String>::default();
        let instances = files
            .iter()
            .filter_map(|(name, path)| {
                trace!(?path, "parsing SolMacroGen from file");
                dup.insert(name.clone()).then(|| SolMacroGen::new(path.clone(), name.clone()))
            })
            .collect::<Vec<_>>();

        let multi = MultiSolMacroGen::new(instances);
        eyre::ensure!(!multi.instances.is_empty(), "No contract artifacts found");
        Ok(multi)
    }

    /// Check that the existing bindings match the expected abigen output
    fn check_existing_bindings(
        &self,
        bindings: &MultiSolMacroGen,
        bindings_root: &Path,
    ) -> Result<()> {
        sh_status!("Checking bindings for {} contracts", bindings.instances.len())?;
        bindings.check_consistency(
            &self.crate_name,
            &self.crate_version,
            bindings_root,
            self.single_file,
            !self.skip_cargo_toml,
            self.module,
            self.alloy_version.clone(),
            self.alloy_rev.clone(),
        )?;
        sh_status!("OK.")?;
        Ok(())
    }

    /// Generate the bindings
    fn generate_bindings(
        &self,
        files: &[(String, PathBuf)],
        bindings_root: &Path,
        sol_config: &ToSolConfig,
    ) -> Result<()> {
        let mut bindings = self.get_solmacrogen(files)?;
        sh_status!("Generating bindings for {} contracts", bindings.instances.len())?;

        if self.module {
            trace!(single_file = self.single_file, "generating module");
            bindings.write_to_module(
                bindings_root,
                self.single_file,
                !self.skip_extra_derives,
                sol_config,
            )?;
        } else {
            trace!(single_file = self.single_file, "generating crate");
            bindings.write_to_crate(
                &self.crate_name,
                &self.crate_version,
                &self.crate_description,
                &self.crate_license,
                bindings_root,
                self.single_file,
                self.alloy_version.clone(),
                self.alloy_rev.clone(),
                !self.skip_extra_derives,
                sol_config,
            )?;
        }

        Ok(())
    }
}

fn enum_definitions(parser: &MultiCompilerParser) -> BTreeMap<String, Vec<String>> {
    parser.solc().compiler().enter(|compiler| {
        let mut definitions = BTreeMap::default();
        let mut ambiguous = HashSet::default();
        for source in compiler.sources().iter() {
            if let Some(ast) = &source.ast {
                collect_enum_definitions(ast.items.iter(), None, &mut definitions, &mut ambiguous);
            }
        }
        definitions
    })
}

fn collect_enum_definitions<'ast>(
    items: impl Iterator<Item = &'ast Item<'ast>>,
    owner: Option<&str>,
    definitions: &mut BTreeMap<String, Vec<String>>,
    ambiguous: &mut HashSet<String>,
) {
    for item in items {
        match &item.kind {
            ItemKind::Enum(enum_item) => {
                let name = enum_item.name.to_string();
                let key = owner.map_or_else(|| name.clone(), |owner| format!("{owner}.{name}"));
                let variants = enum_item.variants.iter().map(ToString::to_string).collect();
                if definitions.get(&key).is_some_and(|existing| existing != &variants) {
                    definitions.remove(&key);
                    ambiguous.insert(key);
                } else if !ambiguous.contains(&key) {
                    definitions.insert(key, variants);
                }
            }
            ItemKind::Contract(contract) => {
                let owner = contract.name.to_string();
                collect_enum_definitions(
                    contract.body.iter(),
                    Some(&owner),
                    definitions,
                    ambiguous,
                );
            }
            _ => {}
        }
    }
}

pub enum Filter {
    All,
    Select(Vec<regex::Regex>),
    Skip(Vec<regex::Regex>),
}

impl Filter {
    pub fn is_match(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Select(regexes) => regexes.iter().any(|regex| regex.is_match(name)),
            Self::Skip(regexes) => !regexes.iter().any(|regex| regex.is_match(name)),
        }
    }

    pub fn skip_default() -> Self {
        let skip = [
            ".*Test.*",
            ".*Script",
            "console[2]?",
            "CommonBase",
            "Components",
            "[Ss]td(Chains|Math|Error|Json|Utils|Cheats|Style|Invariant|Assertions|Toml|Storage(Safe)?)",
            "[Vv]m.*",
            "IMulticall3",
        ]
        .iter()
        .map(|pattern| regex::Regex::new(pattern).unwrap())
        .collect::<Vec<_>>();

        Self::Skip(skip)
    }
}

/// Resolves compilation inputs, retaining importers of selected library dependencies.
fn artifact_graph<C: Compiler<CompilerContract = Contract>>(
    project: &Project<C>,
    selected: Option<&HashSet<PathBuf>>,
) -> Result<Graph<C::Parser>> {
    let mut sources = project.paths.read_input_files()?;
    if let Some(filter) = &project.sparse_output {
        sources.retain(|source, _| filter.is_match(source));
    }
    if let Some(selected) = selected {
        let dependencies = selected
            .iter()
            .filter(|source| !sources.contains_key(*source))
            .map(PathBuf::as_path)
            .collect::<HashSet<_>>();
        let mut roots = selected.clone();
        if !dependencies.is_empty() {
            // Libraries inherit their importers' compiler versions and profiles. Keep those
            // inputs instead of treating selected libraries as standalone compilation roots.
            let graph = Graph::<C::Parser>::resolve_sources(&project.paths, sources.clone())?;
            roots.extend(
                graph
                    .input_nodes()
                    .filter(|node| {
                        graph
                            .imports(node.path())
                            .iter()
                            .any(|import| dependencies.contains(import))
                    })
                    .map(|node| node.path().to_path_buf()),
            );
        }
        sources.retain(|source, _| roots.contains(source));
    }
    Ok(Graph::resolve_sources(&project.paths, sources)?)
}

/// Resolves source/compiler/profile identities using the same graph as compilation.
fn resolved_artifact_contexts<C: Compiler<CompilerContract = Contract>>(
    project: &Project<C>,
    graph: Graph<C::Parser>,
) -> Result<BTreeMap<PathBuf, BTreeSet<(Version, String)>>> {
    let resolved = graph.into_sources_by_version(project)?;
    let mut contexts = BTreeMap::<_, BTreeSet<_>>::new();
    for (version, sources, (profile, _)) in resolved.sources.into_values().flatten() {
        for source in sources.keys() {
            if resolved.primary_profiles.get(source).is_none_or(|primary| *primary == profile) {
                contexts
                    .entry(source.clone())
                    .or_default()
                    .insert((version.clone(), profile.to_string()));
            }
        }
    }
    Ok(contexts)
}

fn binding_name(name: &str) -> String {
    name.replace(char::is_whitespace, "").replace(['-', '$'], "_")
}

/// Provides compiler versions to the graph without installing or compiling anything.
#[derive(Clone)]
struct CachedCompiler {
    solc: Vec<CompilerVersion>,
    vyper: Vec<CompilerVersion>,
    solc_unknown: bool,
    vyper_unknown: bool,
}

impl CachedCompiler {
    fn new(config: &Config, cache: &CompilerCache<MultiCompilerSettings>) -> Self {
        let cached_versions = |vyper| {
            cache
                .files
                .iter()
                .filter(|(source, _)| {
                    matches!(source.extension().and_then(|s| s.to_str()), Some("vy" | "vyi"))
                        == vyper
                })
                .flat_map(|(_, entry)| {
                    entry.artifacts.values().flat_map(|versions| versions.keys().cloned())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(CompilerVersion::Installed)
                .collect::<Vec<_>>()
        };
        let mut solc_unknown = false;
        let solc = match &config.solc {
            Some(request) => {
                if let Ok(version) = request.try_version() {
                    vec![CompilerVersion::Installed(Version::new(
                        version.major,
                        version.minor,
                        version.patch,
                    ))]
                } else {
                    solc_unknown = true;
                    cached_versions(false)
                }
            }
            // Offline bindings consume existing artifacts without requiring installed compilers.
            None if config.offline => cached_versions(false),
            None => {
                let available =
                    SolcCompiler::AutoDetect.available_versions(&SolcLanguage::Solidity);
                if available.iter().any(CompilerVersion::is_installed) {
                    available
                } else {
                    cached_versions(false)
                }
            }
        };
        let mut vyper_unknown = false;
        let vyper = if let Ok(Some(compiler)) = config.vyper_compiler() {
            compiler.available_versions(&Default::default())
        } else {
            vyper_unknown = true;
            cached_versions(true)
        };
        Self { solc, vyper, solc_unknown, vyper_unknown }
    }
}

impl Compiler for CachedCompiler {
    type Input = <MultiCompiler as Compiler>::Input;
    type CompilationError = <MultiCompiler as Compiler>::CompilationError;
    type CompilerContract = <MultiCompiler as Compiler>::CompilerContract;
    type Parser = <MultiCompiler as Compiler>::Parser;
    type Settings = <MultiCompiler as Compiler>::Settings;
    type Language = <MultiCompiler as Compiler>::Language;

    fn compile(
        &self,
        _input: &Self::Input,
    ) -> std::result::Result<
        CompilerOutput<Self::CompilationError, Self::CompilerContract>,
        SolcError,
    > {
        Err(SolcError::msg("cached artifact selection cannot compile sources"))
    }

    fn available_versions(&self, language: &Self::Language) -> Vec<CompilerVersion> {
        if language.is_solc() { self.solc.clone() } else { self.vyper.clone() }
    }
}
