use alloy_primitives::map::HashSet;
use clap::{Parser, ValueHint};
use eyre::Result;
use forge_sol_macro_gen::{MultiSolMacroGen, SolMacroGen};
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use foundry_common::{compile::ProjectCompiler, fs::json_files};
use foundry_config::impl_figment_convert;
use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
};

impl_figment_convert!(BindArgs, build);

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
    #[arg(long, hide = true)]
    alloy: bool,

    /// Specify the alloy version.
    #[arg(long)]
    alloy_version: Option<String>,

    /// Generate bindings for the `ethers` library, instead of `alloy` (removed).
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

        if !self.skip_build {
            let project = self.build.project()?;
            let _ = ProjectCompiler::new().compile(&project)?;
        }

        let config = self.try_load_config_emit_warnings()?;
        let artifacts = config.out;
        let bindings_root = self.bindings.clone().unwrap_or_else(|| artifacts.join("bindings"));

        if bindings_root.exists() {
            if !self.overwrite {
                sh_println!("Bindings found. Checking for consistency.")?;
                return self.check_existing_bindings(&artifacts, &bindings_root);
            }

            trace!(?artifacts, "Removing existing bindings");
            fs::remove_dir_all(&bindings_root)?;
        }

        self.generate_bindings(&artifacts, &bindings_root)?;

        sh_println!("Bindings have been generated to {}", bindings_root.display())?;
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

    /// Returns an iterator over the JSON files and the contract name in the `artifacts` directory.
    fn get_json_files(&self, artifacts: &Path) -> Result<impl Iterator<Item = (String, PathBuf)>> {
        let filter = self.get_filter()?;
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
            .filter(move |(name, _path)| filter.is_match(name)))
    }

    fn get_solmacrogen(&self, artifacts: &Path) -> Result<MultiSolMacroGen> {
        let mut dup = HashSet::<String>::default();
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
        let mut bindings = self.get_solmacrogen(artifacts)?;
        bindings.generate_bindings()?;
        sh_println!("Checking bindings for {} contracts", bindings.instances.len())?;
        bindings.check_consistency(
            &self.crate_name,
            &self.crate_version,
            bindings_root,
            self.single_file,
            !self.skip_cargo_toml,
            self.module,
            self.alloy_version.clone(),
        )?;
        sh_println!("OK.")?;
        Ok(())
    }

    /// Generate the bindings
    fn generate_bindings(&self, artifacts: &Path, bindings_root: &Path) -> Result<()> {
        let mut solmacrogen = self.get_solmacrogen(artifacts)?;
        sh_println!("Generating bindings for {} contracts", solmacrogen.instances.len())?;

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
