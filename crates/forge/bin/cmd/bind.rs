use clap::{Parser, ValueHint};
use ethers_contract_abigen::{
    Abigen, ContractFilter, ExcludeContracts, MultiAbigen, SelectContracts,
};
use eyre::{Result, WrapErr};
use forge::sol_macro_gen::MultiSolMacroGen;
use foundry_cli::{opts::CoreBuildArgs, utils::LoadConfig};
use foundry_common::{compile::ProjectCompiler, fs::json_files};
use foundry_config::impl_figment_convert;
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

    /// Create bindings only for contracts whose names do not match the specified filter(s)
    #[arg(long, conflicts_with = "select")]
    pub skip: Vec<regex::Regex>,

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
    #[arg(long)]
    alloy: bool,

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

        if !self.alloy {
            eprintln!("Warning: ethers bindings are deprecated and will be removed in the future");
            /*
            eprintln!(
                "Warning: `--ethers` (default) bindings are deprecated and will be removed in the future. \
                 Consider using `--alloy` instead."
            );
            */
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
    fn get_filter(&self) -> ContractFilter {
        if self.select_all {
            return ContractFilter::All
        }
        if !self.select.is_empty() {
            return SelectContracts::default().extend_regex(self.select.clone()).into()
        }
        if !self.skip.is_empty() {
            return ExcludeContracts::default().extend_regex(self.skip.clone()).into()
        }
        // This excludes all Test/Script and forge-std contracts
        ExcludeContracts::default()
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
            .into()
    }

    /// Returns an iterator over the JSON files and the contract name in the `artifacts` directory.
    fn get_json_files(&self, artifacts: &Path) -> impl Iterator<Item = (String, PathBuf)> {
        let filter = self.get_filter();
        json_files(artifacts)
            .filter_map(|path| {
                // Ignore the build info JSON.
                if path.to_str()?.contains("/build-info/") {
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
            .filter(move |(name, _path)| filter.is_match(name))
    }

    /// Instantiate the multi-abigen
    fn get_multi(&self, artifacts: &Path) -> Result<MultiAbigen> {
        let abigens = self
            .get_json_files(artifacts)
            .map(|(name, path)| {
                trace!(?path, "parsing Abigen from file");
                let abi = Abigen::new(name, path.to_str().unwrap())
                    .wrap_err_with(|| format!("failed to parse Abigen from file: {:?}", path));
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

    /// Check that the existing bindings match the expected abigen output
    fn check_existing_bindings(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        if !self.alloy {
            return self.check_ethers(artifacts, bindings_root);
        }

        // TODO(yash): check alloy bindings
        Ok(())
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

    /// Generate the bindings
    fn generate_bindings(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        if !self.alloy {
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

        // TODO (yash): complete this

        if !self.module {
            tracing::info!(target: "forge::bind", "Generating crate with alloy bindings");
            // TODO (yash): generate crate
            solmacrogen.write_to_crate(&self.crate_name.as_str(), &self.crate_version.as_str(), bindings_root);
        } else {
            // TODO (yash): generate module
            tracing::info!(target: "forge::bind", "Generating module with alloy bindings");
        }

        Ok(())
    }

    fn get_solmacrogen(&self, artifacts: &Path) -> Result<MultiSolMacroGen> {
        Ok(MultiSolMacroGen::new(artifacts))
    }
}
