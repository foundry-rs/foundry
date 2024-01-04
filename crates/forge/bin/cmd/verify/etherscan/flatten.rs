use super::{EtherscanSourceProvider, VerifyArgs};
use eyre::{Context, Result};
use foundry_block_explorers::verify::CodeFormat;
use foundry_compilers::{
    artifacts::{BytecodeHash, Source},
    AggregatedCompilerOutput, CompilerInput, Project, Solc,
};
use semver::{BuildMetadata, Version};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug)]
pub struct EtherscanFlattenedSource;
impl EtherscanSourceProvider for EtherscanFlattenedSource {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> Result<(String, String, CodeFormat)> {
        let metadata = project.solc_config.settings.metadata.as_ref();
        let bch = metadata.and_then(|m| m.bytecode_hash).unwrap_or_default();

        eyre::ensure!(
            bch == BytecodeHash::Ipfs,
            "When using flattened source, bytecodeHash must be set to ipfs because Etherscan uses IPFS in its Compiler Settings when re-compiling your code. BytecodeHash is currently: {}. Hint: Set the bytecodeHash key in your foundry.toml :)",
            bch,
        );

        let source = project.flatten(target).wrap_err("Failed to flatten contract")?;

        if !args.force {
            // solc dry run of flattened code
            self.check_flattened(source.clone(), version, target).map_err(|err| {
                eyre::eyre!(
                    "Failed to compile the flattened code locally: `{}`\
            To skip this solc dry, have a look at the `--force` flag of this command.",
                    err
                )
            })?;
        }

        let name = args.contract.name.clone();
        Ok((source, name, CodeFormat::SingleFile))
    }
}

impl EtherscanFlattenedSource {
    /// Attempts to compile the flattened content locally with the compiler version.
    ///
    /// This expects the completely flattened `content´ and will try to compile it using the
    /// provided compiler. If the compiler is missing it will be installed.
    ///
    /// # Errors
    ///
    /// If it failed to install a missing solc compiler
    ///
    /// # Exits
    ///
    /// If the solc compiler output contains errors, this could either be due to a bug in the
    /// flattening code or could to conflict in the flattened code, for example if there are
    /// multiple interfaces with the same name.
    fn check_flattened(
        &self,
        content: impl Into<String>,
        version: &Version,
        contract_path: &Path,
    ) -> Result<()> {
        let version = strip_build_meta(version.clone());
        let solc = Solc::find_svm_installed_version(version.to_string())?
            .unwrap_or(Solc::blocking_install(&version)?);

        let input = CompilerInput {
            language: "Solidity".to_string(),
            sources: BTreeMap::from([("contract.sol".into(), Source::new(content))]),
            settings: Default::default(),
        };

        let out = solc.compile(&input)?;
        if out.has_error() {
            let mut o = AggregatedCompilerOutput::default();
            o.extend(version, out);
            let diags = o.diagnostics(&[], Default::default());

            eyre::bail!(
                "\
Failed to compile the flattened code locally.
This could be a bug, please inspect the output of `forge flatten {}` and report an issue.
To skip this solc dry, pass `--force`.
Diagnostics: {diags}",
                contract_path.display()
            );
        }

        Ok(())
    }
}

/// Strips [BuildMetadata] from the [Version]
///
/// **Note:** this is only for local compilation as a dry run, therefore this will return a
/// sanitized variant of the specific version so that it can be installed. This is merely
/// intended to ensure the flattened code can be compiled without errors.
fn strip_build_meta(version: Version) -> Version {
    if version.build != BuildMetadata::EMPTY {
        Version::new(version.major, version.minor, version.patch)
    } else {
        version
    }
}
