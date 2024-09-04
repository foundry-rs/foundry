use clap::{Parser, ValueHint};
use ethers_contract_abigen::{
    Abigen, ContractFilter, ExcludeContracts, MultiAbigen, SelectContracts,
};
use eyre::{Result, WrapErr};
use forge_sol_macro_gen::{MultiSolMacroGen, SolMacroGen};
use foundry_cli::{opts::CoreBuildArgs, utils::LoadConfig};
use foundry_common::{compile::ProjectCompiler, fs::json_files};
use foundry_config::impl_figment_convert;
use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
};

impl_figment_convert!(BindArgs, build_args);

const DEFAULT_CRATE_NAME: &str = "foundry-contracts";
const DEFAULT_CRATE_VERSION: &str = "0.1.0";

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
    #[arg(long, conflicts_with = "ethers")]
    alloy: bool,

    /// Specify the alloy version.
    #[arg(long, value_name = "ALLOY_VERSION")]
    alloy_version: Option<String>,

    /// Generate bindings for the `ethers` library, instead of `alloy` (default, deprecated).
    #[arg(long)]
    ethers: bool,

    #[command(flatten)]
    build_args: CoreBuildArgs,
}

impl BindArgs {
    pub fn run(self) -> Result<()> {
        if !self.skip_build {
            let project = self.build_args.project()?;
            let _ = ProjectCompiler::new().compile(&project)?;
        }

        if self.ethers {
            eprintln!(
                "Warning: `--ethers` bindings are deprecated and will be removed in the future. \
                 Consider using `--alloy` (default) instead."
            );
        }

        let config = self.try_load_config_emit_warnings()?;
        let artifacts = config.out;
        let bindings_root = self.bindings.clone().unwrap_or_else(|| artifacts.join("bindings"));

        if bindings_root.exists() {
            if !self.overwrite {
                println!("Bindings found. Checking for consistency.");
                return self.check_existing_bindings(&artifacts, &bindings_root);
            }

            trace!(?artifacts, "Removing existing bindings");
            fs::remove_dir_all(&bindings_root)?;
        }

        self.generate_bindings(&artifacts, &bindings_root)?;

        println!("Bindings have been generated to {}", bindings_root.display());
        Ok(())
    }

    /// Returns the filter to use for `MultiAbigen`
    fn get_filter(&self) -> Result<ContractFilter> {
        if self.select_all {
            return Ok(ContractFilter::All)
        }
        if !self.select.is_empty() {
            return Ok(SelectContracts::default().extend_regex(self.select.clone()).into())
        }
        if let Some(skip) = self.build_args.skip.as_ref().filter(|s| !s.is_empty()) {
            return Ok(ExcludeContracts::default()
                .extend_regex(
                    skip.clone()
                        .into_iter()
                        .map(|s| Regex::new(s.file_pattern()))
                        .collect::<Result<Vec<_>, _>>()?,
                )
                .into())
        }
        // This excludes all Test/Script and forge-std contracts
        Ok(ExcludeContracts::default()
            .extend_pattern([
                ".*Test.*",
                ".*Script",
                "console[2]?",
                "CommonBase",
                "Components",
                "[Ss]td(Chains|Math|Error|Json|Utils|Cheats|Style|Invariant|Assertions|Toml|Storage(Safe)?)",
                "[Vv]m.*",
            ])
            .extend_names(["IMulticall3"])
            .into())
    }

    fn get_alloy_filter(&self) -> Result<Filter> {
        if self.select_all {
            // Select all json files
            return Ok(Filter::All);
        }
        if !self.select.is_empty() {
            // Return json files that match the select regex
            return Ok(Filter::Select(self.select.clone()));
        }

        if let Some(skip) = self.build_args.skip.as_ref().filter(|s| !s.is_empty()) {
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

    /// Returns an iterator over the JSON files and the contract name in the `artifacts` directory.
    fn get_json_files(&self, artifacts: &Path) -> Result<impl Iterator<Item = (String, PathBuf)>> {
        let filter = self.get_filter()?;
        let alloy_filter = self.get_alloy_filter()?;
        let is_alloy = !self.ethers;
        Ok(json_files(artifacts)
            .filter_map(|path| {
                // Ignore the build info JSON.
                if path.to_str()?.contains("build-info") {
                    return None;
                }

                // We don't want `.metadata.json` files.
                let stem = path.file_stem()?.to_str()?;
                if stem.ends_with(".metadata") {
                    return None;
                }

                let name = stem.split('.').next().unwrap();

                // Best effort identifier cleanup.
                let name = name.replace(char::is_whitespace, "").replace('-', "_");

                Some((name, path))
            })
            .filter(
                move |(name, _path)| {
                    if is_alloy {
                        alloy_filter.is_match(name)
                    } else {
                        filter.is_match(name)
                    }
                },
            ))
    }

    /// Instantiate the multi-abigen
    fn get_multi(&self, artifacts: &Path) -> Result<MultiAbigen> {
        let abigens = self
            .get_json_files(artifacts)?
            .map(|(name, path)| {
                trace!(?path, "parsing Abigen from file");
                let abi = Abigen::new(name, path.to_str().unwrap())
                    .wrap_err_with(|| format!("failed to parse Abigen from file: {path:?}"));
                if !self.skip_extra_derives {
                    abi?.add_derive("serde::Serialize")?.add_derive("serde::Deserialize")
                } else {
                    abi
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let multi = MultiAbigen::from_abigens(abigens);
        eyre::ensure!(!multi.is_empty(), "No contract artifacts found");
        Ok(multi)
    }

    fn get_solmacrogen(&self, artifacts: &Path) -> Result<MultiSolMacroGen> {
        let mut dup = std::collections::HashSet::<String>::new();
        let instances = self
            .get_json_files(artifacts)?
            .filter_map(|(name, path)| {
                trace!(?path, "parsing SolMacroGen from file");
                if dup.insert(name.clone()) {
                    Some(SolMacroGen::new(path, name))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let multi = MultiSolMacroGen::new(artifacts, instances);
        eyre::ensure!(!multi.instances.is_empty(), "No contract artifacts found");
        Ok(multi)
    }

    /// Check that the existing bindings match the expected abigen output
    fn check_existing_bindings(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        if self.ethers {
            return self.check_ethers(artifacts, bindings_root);
        }

        self.check_alloy(artifacts, bindings_root)
    }

    fn check_ethers(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        let bindings = self.get_multi(artifacts)?.build()?;
        println!("Checking bindings for {} contracts.", bindings.len());
        if !self.module {
            bindings
                .ensure_consistent_crate(
                    &self.crate_name,
                    &self.crate_version,
                    bindings_root,
                    self.single_file,
                    !self.skip_cargo_toml,
                )
                .map_err(|err| {
                    if !self.skip_cargo_toml && err.to_string().contains("Cargo.toml") {
                        err.wrap_err("To skip Cargo.toml consistency check, pass --skip-cargo-toml")
                    } else {
                        err
                    }
                })?;
        } else {
            bindings.ensure_consistent_module(bindings_root, self.single_file)?;
        }
        println!("OK.");
        Ok(())
    }

    fn check_alloy(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        let mut bindings = self.get_solmacrogen(artifacts)?;
        bindings.generate_bindings()?;
        println!("Checking bindings for {} contracts", bindings.instances.len());
        bindings.check_consistency(
            &self.crate_name,
            &self.crate_version,
            bindings_root,
            self.single_file,
            !self.skip_cargo_toml,
            self.module,
            self.alloy_version.clone(),
        )?;
        println!("OK.");
        Ok(())
    }

    /// Generate the bindings
    fn generate_bindings(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        if self.ethers {
            return self.generate_ethers(artifacts, bindings_root);
        }

        self.generate_alloy(artifacts, bindings_root)
    }

    fn generate_ethers(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        let mut bindings = self.get_multi(artifacts)?.build()?;
        println!("Generating bindings for {} contracts", bindings.len());
        if !self.module {
            trace!(single_file = self.single_file, "generating crate");
            if !self.skip_extra_derives {
                bindings = bindings.dependencies([r#"serde = "1""#])
            }
            bindings.write_to_crate(
                &self.crate_name,
                &self.crate_version,
                bindings_root,
                self.single_file,
            )
        } else {
            trace!(single_file = self.single_file, "generating module");
            bindings.write_to_module(bindings_root, self.single_file)
        }
    }

    fn generate_alloy(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        let mut solmacrogen = self.get_solmacrogen(artifacts)?;
        println!("Generating bindings for {} contracts", solmacrogen.instances.len());

        if !self.module {
            trace!(single_file = self.single_file, "generating crate");
            solmacrogen.write_to_crate(
                &self.crate_name,
                &self.crate_version,
                bindings_root,
                self.single_file,
                self.alloy_version.clone(),
            )?;
        } else {
            trace!(single_file = self.single_file, "generating module");
            solmacrogen.write_to_module(bindings_root, self.single_file)?;
        }

        Ok(())
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
