//! Artifact path resolution and bytecode lookup for both execution engines.

use crate::{CheatsConfig, Result};
use alloy_json_abi::ContractObject;
use alloy_primitives::Bytes;
use foundry_common::{contracts::ContractData, fs};
use foundry_config::fs_permissions::FsAccessKind;
use semver::Version;
use std::path::{Path, PathBuf};

/// Parsed artifact path components.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ParsedArtifactPath<'a> {
    pub(super) file: Option<PathBuf>,
    pub(super) contract_name: Option<&'a str>,
    pub(super) version: Option<Version>,
    pub(super) profile: Option<&'a str>,
}

/// Parses an artifact path string into its components.
///
/// Supports the following formats:
/// - `path/to/contract.sol`
/// - `path/to/contract.sol:ContractName`
/// - `path/to/contract.sol:ContractName:0.8.23`
/// - `path/to/contract.sol:ContractName:profile`
/// - `path/to/contract.sol:0.8.23`
/// - `path/to/contract.sol:profile`
/// - `ContractName`
/// - `ContractName:0.8.23`
/// - `ContractName:profile`
pub(super) fn parse_artifact_path(
    path: &str,
) -> std::result::Result<ParsedArtifactPath<'_>, String> {
    // A Windows drive separator belongs to the file, not the artifact's suffix fields.
    // Recognize it on every host so parsing does not depend on the current platform.
    let unprefixed = path.strip_prefix(r"\\?\").unwrap_or(path);
    let drive_prefix_len = match unprefixed.as_bytes() {
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic() => {
            path.len() - unprefixed.len() + 2
        }
        _ => 0,
    };
    let mut parts = path[drive_prefix_len..].split(':');

    let mut file = None;
    let mut contract_name = None;
    let mut version = None;
    let mut profile = None;

    let path_or_name = parts.next().unwrap();
    let path_or_name = &path[..drive_prefix_len + path_or_name.len()];
    if path_or_name.contains('.') {
        file = Some(PathBuf::from(path_or_name));
        if let Some(name_or_version_or_profile) = parts.next() {
            if name_or_version_or_profile.contains('.')
                || Version::parse(name_or_version_or_profile).is_ok()
            {
                version = Some(name_or_version_or_profile);
            } else {
                contract_name = Some(name_or_version_or_profile);
                if let Some(version_or_profile) = parts.next() {
                    if version_or_profile.contains('.')
                        || Version::parse(version_or_profile).is_ok()
                    {
                        version = Some(version_or_profile);
                    } else {
                        profile = Some(version_or_profile);
                    }
                }
            }
        }
    } else {
        contract_name = Some(path_or_name);
        if let Some(version_or_profile) = parts.next() {
            if version_or_profile.contains('.') || Version::parse(version_or_profile).is_ok() {
                version = Some(version_or_profile);
            } else {
                profile = Some(version_or_profile);
            }
        }
    }

    let version = if let Some(version) = version {
        Some(Version::parse(version).map_err(|e| format!("failed parsing version: {e}"))?)
    } else {
        None
    };

    Ok(ParsedArtifactPath { file, contract_name, version, profile })
}

/// Resolved location of an artifact referenced by a cheatcode path argument.
pub(super) enum ArtifactSource<'a> {
    /// The artifact was matched in the in-memory `available_artifacts` list.
    InMemory(&'a ContractData),
    /// The artifact must be read from the given path on disk.
    Disk(PathBuf),
}

/// Resolves a cheatcode artifact reference to its source.
///
/// Can parse the following input formats:
/// - `path/to/artifact.json`
/// - `path/to/contract.sol`
/// - `path/to/contract.sol:ContractName`
/// - `path/to/contract.sol:ContractName:0.8.23`
/// - `path/to/contract.sol:ContractName:profile`
/// - `path/to/contract.sol:0.8.23`
/// - `path/to/contract.sol:profile`
/// - `ContractName`
/// - `ContractName:0.8.23`
/// - `ContractName:profile`
pub(super) fn get_artifact_source<'a>(
    config: &'a CheatsConfig,
    path: &str,
) -> Result<ArtifactSource<'a>> {
    if path.ends_with(".json") {
        let path = config.ensure_path_allowed(path, FsAccessKind::Read)?;
        return Ok(ArtifactSource::Disk(path));
    }

    let artifacts = config.available_artifacts.as_ref().or(config.artifact_lookup.as_ref());
    let resolve_source = |file: PathBuf| {
        let cwd = config
            .running_artifact
            .as_ref()
            .and_then(|artifact| artifact.source.parent())
            .unwrap_or(&config.paths.root);
        let relative_cwd = cwd.strip_prefix(&config.paths.root).unwrap_or(cwd);
        let has_matching_remapping = config.paths.remappings.iter().any(|remapping| {
            remapping.context.as_ref().is_none_or(|context| relative_cwd.starts_with(context))
                && file.strip_prefix(&remapping.name).is_ok()
        });

        if has_matching_remapping {
            config.paths.resolve_library_import(cwd, &file).map_or(file, |resolved| {
                resolved.strip_prefix(&config.paths.root).unwrap_or(&resolved).to_path_buf()
            })
        } else {
            file
        }
    };
    let exact_identifier = path
        .rsplit_once(':')
        .map(|(source, contract)| (resolve_source(PathBuf::from(source)), contract))
        .filter(|(source, contract)| {
            artifacts.into_iter().flat_map(|artifacts| artifacts.iter()).any(|(id, _)| {
                id.source == *source
                    && id.name.split('.').next().is_some_and(|name| name == *contract)
            })
        });

    let parsed = match parse_artifact_path(path) {
        Ok(parsed) => parsed,
        Err(_) if exact_identifier.is_some() => {
            ParsedArtifactPath { file: None, contract_name: None, version: None, profile: None }
        }
        Err(error) => return Err(fmt_err!("failed to parse artifact path: {error}")),
    };
    let ParsedArtifactPath { file, contract_name, version, profile } = parsed;
    let file = file.map(resolve_source);

    // Use the artifact lookup if present.
    if let Some(artifacts) = artifacts {
        let ambiguous_file_profile =
            file.is_some() && version.is_none() && profile.is_none() && contract_name.is_some();
        let filter_artifacts = |treat_ambiguous_as_profile: bool| -> Vec<_> {
            artifacts
                .iter()
                .filter(|(id, _)| {
                    if let Some((source, contract)) = &exact_identifier {
                        return id.source == *source
                            && id.name.split('.').next().is_some_and(|name| name == *contract);
                    }

                    // name might be in the form of "Counter.0.8.23"
                    let id_name = id.name.split('.').next().unwrap();

                    if let Some(path) = &file
                        && !id.source.ends_with(path)
                    {
                        return false;
                    }
                    if let Some(ref version) = version
                        && (id.version.minor != version.minor
                            || id.version.major != version.major
                            || id.version.patch != version.patch)
                    {
                        return false;
                    }
                    if let Some(profile) = profile
                        && id.profile != profile
                    {
                        return false;
                    }
                    if let Some(name) = contract_name {
                        if treat_ambiguous_as_profile && ambiguous_file_profile {
                            return id.profile == name;
                        }

                        return id_name == name;
                    }

                    true
                })
                .collect()
        };

        let mut filtered = filter_artifacts(false);
        if filtered.is_empty() && ambiguous_file_profile {
            filtered = filter_artifacts(true);
        }

        let artifact = match &filtered[..] {
            [] => None,
            [artifact] => Some(Ok(*artifact)),
            filtered => {
                let mut filtered = filtered.to_vec();
                // If we know the current script/test contract solc version, try to filter by it
                Some(
                    config
                        .running_artifact
                        .as_ref()
                        .and_then(|running| {
                            // Only filter by running version if user did NOT specify a version
                            if exact_identifier.is_some() || version.is_none() {
                                filtered.retain(|(id, _)| id.version == running.version);

                                // Return artifact if only one matched
                                if filtered.len() == 1 {
                                    return Some(filtered[0]);
                                }
                            }

                            // Only filter by running profile if user did NOT specify a profile
                            if exact_identifier.is_some() || profile.is_none() {
                                filtered.retain(|(id, _)| id.profile == running.profile);

                                return (filtered.len() == 1).then(|| filtered[0]);
                            }

                            None
                        })
                        .ok_or_else(|| fmt_err!("multiple matching artifacts found")),
                )
            }
        };

        if let Some(artifact) = artifact {
            return Ok(ArtifactSource::InMemory(artifact?.1));
        }
    }

    // Fallback: construct path manually when no artifacts list or no match found
    let path_in_artifacts = match (file.map(|f| f.to_string_lossy().to_string()), contract_name) {
        (Some(file), Some(contract_name)) => PathBuf::from(format!("{file}/{contract_name}.json")),
        (None, Some(contract_name)) => {
            PathBuf::from(format!("{contract_name}.sol/{contract_name}.json"))
        }
        (Some(file), None) => {
            let name = file.replace(".sol", "");
            PathBuf::from(format!("{file}/{name}.json"))
        }
        _ => bail!("invalid artifact path"),
    };

    let path = config.paths.artifacts.join(path_in_artifacts);
    let path = config.ensure_path_allowed(path, FsAccessKind::Read)?;
    Ok(ArtifactSource::Disk(path))
}

/// Reads an artifact JSON file, mapping I/O errors to a helpful message when the
/// lookup fell through the in-memory artifacts list.
pub(super) fn read_artifact_file(config: &CheatsConfig, path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| {
        if config.available_artifacts.is_some() {
            fmt_err!("no matching artifact found")
        } else {
            e.into()
        }
    })
}

/// Returns the bytecode from a JSON artifact file.
///
/// See [`get_artifact_source`] for the supported path formats.
///
/// This function is safe to use with contracts that have library dependencies.
/// `alloy_json_abi::ContractObject` validates bytecode during JSON parsing and will
/// reject artifacts with unlinked library placeholders.
pub(crate) fn get_artifact_code(
    config: &CheatsConfig,
    path: &str,
    deployed: bool,
) -> Result<Bytes> {
    let maybe_bytecode = match get_artifact_source(config, path)? {
        ArtifactSource::InMemory(data) => {
            if deployed { data.deployed_bytecode() } else { data.bytecode() }.cloned()
        }
        ArtifactSource::Disk(path) => {
            let data = read_artifact_file(config, &path)?;
            let artifact = serde_json::from_str::<ContractObject>(&data)?;
            if deployed { artifact.deployed_bytecode } else { artifact.bytecode }
        }
    };
    maybe_bytecode.ok_or_else(|| fmt_err!("no bytecode for contract; is it abstract or unlinked?"))
}
