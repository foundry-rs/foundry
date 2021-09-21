use ethers::core::utils::{CompiledContract, Solc};
use eyre::Result;
use semver::{Version, VersionReq};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

/// Supports building contracts
#[derive(Clone, Debug)]
pub struct SolcBuilder<'a> {
    contracts: &'a str,
    remappings: &'a [String],
    lib_paths: &'a [String],
    versions: Vec<Version>,
    releases: Vec<Version>,
}

impl<'a> SolcBuilder<'a> {
    pub fn new(
        contracts: &'a str,
        remappings: &'a [String],
        lib_paths: &'a [String],
    ) -> Result<Self> {
        let versions = svm::installed_versions().unwrap_or_default();
        // Try to download the releases, if it fails default to empty
        let releases = match tokio::runtime::Runtime::new()?.block_on(svm::all_versions()) {
            Ok(inner) => inner,
            Err(err) => {
                tracing::error!("Failed to get upstream releases: {}", err);
                Vec::new()
            }
        };
        Ok(Self {
            contracts,
            remappings,
            lib_paths,
            versions,
            releases,
        })
    }

    /// Builds all provided contract files with the specified compiler version.
    /// Assumes that the lib-paths and remappings have already been specified.
    #[tracing::instrument(skip(self, files))]
    pub fn build(
        &self,
        version: String,
        files: Vec<String>,
    ) -> Result<HashMap<String, CompiledContract>> {
        let mut compiler_path = installed_version_paths()?
            .iter()
            .find(|name| name.to_string_lossy().contains(&version))
            .unwrap()
            .clone();
        compiler_path.push(format!("solc-{}", &version));

        // tracing::trace!(?files);
        let mut solc = Solc::new_with_paths(files).solc_path(compiler_path);
        let lib_paths = self
            .lib_paths
            .iter()
            .filter(|path| PathBuf::from(path).exists())
            .map(|path| {
                std::fs::canonicalize(path)
                    .unwrap()
                    .into_os_string()
                    .into_string()
                    .unwrap()
            })
            .collect::<Vec<_>>()
            .join(",");

        // tracing::trace!(?lib_paths);
        solc = solc.args(std::array::IntoIter::new(["--allow-paths", &lib_paths]));

        // tracing::trace!(?self.remappings);
        if !self.remappings.is_empty() {
            solc = solc.args(self.remappings)
        }

        Ok(solc.build()?)
    }

    /// Builds all contracts with their corresponding compiler versions
    #[tracing::instrument(skip(self))]
    pub fn build_all(&mut self) -> Result<HashMap<String, CompiledContract>> {
        let contracts_by_version = self.contract_versions()?;

        let start = Instant::now();
        let res = contracts_by_version.into_iter().try_fold(
            HashMap::new(),
            |mut map, (version, files)| {
                let res = self.build(version, files)?;
                map.extend(res);
                Ok::<_, eyre::Error>(map)
            },
        );
        let duration = Instant::now().duration_since(start);
        tracing::info!(compilation_time = ?duration);

        res
    }
    /// Given a Solidity file, it detects the latest compiler version which can be used
    /// to build it, and returns it along with its canonicalized path. If the required
    /// compiler version is not installed, it also proceeds to install it.
    fn detect_version(&mut self, fname: PathBuf) -> Result<Option<(Version, String)>> {
        let path = std::fs::canonicalize(&fname)?;

        // detects the required solc version
        let sol_version = Self::version_req(&path)?;

        let path_str = path
            .into_os_string()
            .into_string()
            .map_err(|_| eyre::eyre!("invalid path, maybe not utf-8?"))?;

        // use the installed one, install it if it does not exist
        let res = Self::find_matching_installation(&self.versions, &sol_version)
            .or_else(|| {
                // Check upstream for a matching install
                Self::find_matching_installation(&self.releases, &sol_version).map(|version| {
                    println!("Installing {}", version);
                    // Blocking call to install it over RPC.
                    tokio::runtime::Runtime::new()
                        .unwrap()
                        .block_on(svm::install(&version))
                        .unwrap();
                    self.versions.push(version.clone());
                    println!("Done!");
                    version
                })
            })
            .map(|version| (version, path_str));

        Ok(res)
    }

    /// Gets a map of compiler version -> vec[contract paths]
    fn contract_versions(&mut self) -> Result<HashMap<String, Vec<String>>> {
        // Group contracts in the nones with the same version pragma
        let files = glob::glob(self.contracts)?;

        // get all the corresponding contract versions
        Ok(files
            .filter_map(|fname| fname.ok())
            .filter_map(|fname| self.detect_version(fname).ok().flatten())
            .fold(HashMap::new(), |mut map, (version, path)| {
                let entry = map.entry(version.to_string()).or_insert_with(Vec::new);
                entry.push(path);
                map
            }))
    }

    /// Parses the given Solidity file looking for the `pragma` definition and
    /// returns the corresponding SemVer version requirement.
    fn version_req(path: &Path) -> Result<VersionReq> {
        let file = BufReader::new(File::open(path)?);
        let version = file
            .lines()
            .map(|line| line.unwrap())
            .find(|line| line.starts_with("pragma"))
            .ok_or_else(|| eyre::eyre!("{:?} has no version", path))?;
        let version = version
            .replace("pragma solidity ", "")
            .replace(";", "")
            // needed to make it valid semver for things like
            // >=0.4.0 <0.5.0
            .replace(" ", ",");

        Ok(VersionReq::parse(&version)?)
    }

    /// Find a matching local installation for the specified required version
    fn find_matching_installation(
        versions: &[Version],
        required_version: &VersionReq,
    ) -> Option<Version> {
        versions
            .iter()
            // filter these out, unneeded artifact from solc-vm-rs
            // .filter(|&version| version != ".global-version")
            .find(|version| required_version.matches(version))
            .cloned()
    }
}

fn installed_version_paths() -> Result<Vec<PathBuf>> {
    let home_dir = svm::SVM_HOME.clone();
    let mut versions = vec![];
    for version in std::fs::read_dir(home_dir)? {
        let version = version?;
        versions.push(version.path());
    }

    versions.sort();
    Ok(versions)
}
