use std::path::PathBuf;

use alloy_primitives::map::HashMap;
use foundry_compilers::artifacts::Severity;
use foundry_compilers::compile::resolc;
use foundry_compilers::compile::resolc::resolc_artifact_output::ResolcArtifactOutput;
use foundry_compilers::compilers::resolc::{Resolc, ResolcOptimizer, ResolcSettings};
use foundry_compilers::solc::Solc;
use foundry_compilers::solc::SolcCompiler;
use foundry_compilers::{error::SolcError, solc::SolcLanguage, ProjectPathsConfig};
use foundry_compilers::{Project, ProjectBuilder};
use foundry_config::Config;
use foundry_config::{SkipBuildFilters, SolcReq};
use semver::Version;
pub struct ResolcCompiler();
impl ResolcCompiler {
    pub fn config_ensure_resolc(
        resolc: Option<&SolcReq>,
        offline: bool,
    ) -> Result<Option<PathBuf>, SolcError> {
        if let Some(ref resolc) = resolc {
            let resolc = match resolc {
                SolcReq::Version(version) => {
                    let mut resolc = Resolc::find_installed_version(version)?;
                    if resolc.is_none() {
                        if offline {
                            return Err(SolcError::msg(format!(
                                "can't install missing resolc {version} in offline mode"
                            )));
                        }
                        Resolc::blocking_install(version)?;
                        resolc = Resolc::find_installed_version(version)?;
                    }
                    resolc
                }
                SolcReq::Local(resolc) => {
                    if !resolc.is_file() {
                        return Err(SolcError::msg(format!(
                            "`resolc` {} does not exist",
                            resolc.display()
                        )));
                    }
                    Some(resolc.clone())
                }
            };
            return Ok(resolc);
        }

        Ok(None)
    }
    pub fn config_project_paths(config: &Config) -> ProjectPathsConfig<SolcLanguage> {
        let builder = ProjectPathsConfig::builder()
            .cache(config.cache_path.clone())
            .sources(&config.src.clone())
            .tests(&config.test)
            .scripts(&config.script)
            .artifacts(config.root.clone())
            .libs(config.libs.iter())
            .remappings(config.get_all_remappings())
            .allowed_path(&config.root)
            .allowed_paths(&config.libs)
            .allowed_paths(&config.allow_paths)
            .include_paths(&config.include_paths);

        builder.build_with_root(&config.root)
    }
    fn config_solc_compiler(config: &Config) -> Result<SolcCompiler, SolcError> {
        if let Some(path) = &config.resolc_config.solc_path {
            if !path.is_file() {
                return Err(SolcError::msg(format!("`solc` {} does not exist", path.display())));
            }
            let version = Resolc::get_solc_version_info(path)?.version;
            let solc = Solc::new_with_version(
                path,
                Version::new(version.major, version.minor, version.patch),
            );
            return Ok(SolcCompiler::Specific(solc));
        }

        if let Some(ref solc) = config.solc {
            let solc = match solc {
                SolcReq::Version(version) => {
                    let maybe_solc = Resolc::find_installed_version(&version)?;
                    let path = if let Some(solc) = maybe_solc {
                        solc
                    } else {
                        Resolc::blocking_install(&version)?
                    };
                    Solc::new_with_version(
                        path,
                        Version::new(version.major, version.minor, version.patch),
                    )
                }
                SolcReq::Local(path) => {
                    if !path.is_file() {
                        return Err(SolcError::msg(format!(
                            "`solc` {} does not exist",
                            path.display()
                        )));
                    }
                    let version = Resolc::get_solc_version_info(path)?.version;
                    Solc::new_with_version(
                        path,
                        Version::new(version.major, version.minor, version.patch),
                    )
                }
            };
            Ok(SolcCompiler::Specific(solc))
        } else {
            Ok(SolcCompiler::AutoDetect)
        }
    }

    pub fn solc_to_resolc_settings(config: &Config) -> Result<ResolcSettings, SolcError> {
        Ok(ResolcSettings::new(
            ResolcOptimizer::new(config.optimizer, config.optimizer_runs as u64),
            HashMap::<String, HashMap<String, Vec<String>>>::default(),
        ))
    }

    pub fn create_project(
        config: &Config,
    ) -> Result<Project<Resolc, ResolcArtifactOutput>, SolcError> {
        let mut builder = ProjectBuilder::<Resolc>::default()
            .artifacts(ResolcArtifactOutput {})
            .settings(Self::solc_to_resolc_settings(&config)?)
            .paths(ResolcCompiler::config_project_paths(&config))
            .ignore_error_codes(config.ignored_error_codes.iter().copied().map(Into::into))
            .ignore_paths(config.ignored_file_paths.clone())
            .set_compiler_severity_filter(if config.deny_warnings {
                Severity::Warning
            } else {
                Severity::Error
            })
            .set_offline(config.offline)
            .set_cached(config.cache)
            .set_build_info(config.build_info)
            .set_no_artifacts(false);
        if !config.skip.is_empty() {
            let filter = SkipBuildFilters::new(config.skip.clone(), config.root.clone());
            builder = builder.sparse_output(filter);
        }
        let resolc = if let Some(resolc) =
            Self::config_ensure_resolc(config.resolc_config.resolc.as_ref(), config.offline)?
        {
            resolc
        } else if !config.offline {
            // ideally here we want to fetch the latest version from github but
            // for now we can hardcode the latest version
            let default_version = Version::parse("0.1.0-dev.6").unwrap();
            let mut resolc = Resolc::find_installed_version(&default_version)?;
            if resolc.is_none() {
                Resolc::blocking_install(&default_version)?;
                resolc = Resolc::find_installed_version(&default_version)?;
            }
            resolc.unwrap_or_else(|| panic!("Could not install resolc v{}", default_version))
        } else {
            "resolc".into()
        };
        // the below will never panic so calling unwrap is ok
        // We might need to revise the code for calling ::new
        let resolc_compiler = Resolc::new(resolc).unwrap();
        let project = builder.build(resolc_compiler)?;
        if config.force {
            config.cleanup(&project)?;
        }
        Ok(project)
    }
}
