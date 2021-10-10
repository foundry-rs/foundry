use ethers::core::utils::{CompiledContract, Solc};
use eyre::Result;
use rayon::prelude::*;
use semver::{Version, VersionReq};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(any(test, feature = "sync"))]
use std::sync::Mutex;
#[cfg(any(test, feature = "sync"))]
static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
#[cfg(any(test, feature = "sync"))]
use ethers::prelude::Lazy;

/// Supports building contracts
#[derive(Debug)]
pub struct SolcBuilder<'a> {
    contracts: &'a str,
    remappings: &'a [String],
    lib_paths: &'a [String],
    releases: Vec<Version>,
}

impl<'a> SolcBuilder<'a> {
    pub fn new(
        contracts: &'a str,
        remappings: &'a [String],
        lib_paths: &'a [String],
    ) -> Result<Self> {
        // Try to download the releases, if it fails default to empty
        let releases = match tokio::runtime::Runtime::new()?.block_on(svm::all_versions()) {
            Ok(inner) => inner,
            Err(err) => {
                tracing::error!("Failed to get upstream releases: {}", err);
                Vec::new()
            }
        };
        Ok(Self { contracts, remappings, lib_paths, releases })
    }

    /// Builds all provided contract files with the specified compiler version.
    /// Assumes that the lib-paths and remappings have already been specified and
    /// that the correct compiler version is provided.
    // FIXME: Does NOT support contracts with the same name.
    #[tracing::instrument(skip(self, files))]
    fn build(
        &self,
        version: &str,
        files: Vec<String>,
    ) -> Result<HashMap<String, CompiledContract>> {
        let compiler_path = find_installed_version_path(version)?
            .ok_or_else(|| eyre::eyre!("version {} not installed", version))?;

        // tracing::trace!(?files);
        let mut solc = Solc::new_with_paths(files).solc_path(compiler_path);
        let lib_paths = self
            .lib_paths
            .iter()
            .filter(|path| PathBuf::from(path).exists())
            .map(|path| {
                std::fs::canonicalize(path).unwrap().into_os_string().into_string().unwrap()
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
    pub fn build_all(&self) -> Result<HashMap<String, CompiledContract>> {
        tracing::info!("starting compilation");
        let contracts_by_version = self.contract_versions()?;
        let start = Instant::now();

        let res = contracts_by_version
            .into_par_iter()
            .try_fold(HashMap::new, |mut map, (version, files)| {
                let res = self.build(&version, files)?;
                map.extend(res);
                Ok::<_, eyre::Error>(map)
            })
            // Need to define the logic for combining the 2 maps in Rayon after the fold
            .reduce(
                || Ok(HashMap::new()),
                |prev: Result<HashMap<_, _>>, map: Result<HashMap<_, _>>| match (prev, map) {
                    (Ok(mut prev), Ok(map)) => {
                        prev.extend(map);
                        Ok(prev)
                    }
                    (Err(err), _) => Err(err),
                    (_, Err(err)) => Err(err),
                },
            );
        let duration = Instant::now().duration_since(start);
        tracing::info!(compilation_time = ?duration);

        res
    }

    /// Given a Solidity file, it detects the latest compiler version which can be used
    /// to build it, and returns it along with its canonicalized path. If the required
    /// compiler version is not installed, it also proceeds to install it.
    #[tracing::instrument(err)]
    fn detect_version(&self, fname: &Path) -> Result<Option<(Version, String)>> {
        let path = std::fs::canonicalize(fname)?;

        // detects the required solc version
        let sol_version = Self::version_req(&path)?;

        let path_str = path
            .into_os_string()
            .into_string()
            .map_err(|_| eyre::eyre!("invalid path, maybe not utf-8?"))?;

        #[cfg(any(test, feature = "sync"))]
        // take the lock in tests, we use this to enforce that
        // a test does not run while a compiler version is being installed
        let _lock = LOCK.lock();

        // load the local / remote versions
        let versions = svm::installed_versions().unwrap_or_default();
        let local_versions = Self::find_matching_installation(&versions, &sol_version);
        let remote_versions = Self::find_matching_installation(&self.releases, &sol_version);

        // if there's a better upstream version than the one we have, install it
        let res = match (local_versions, remote_versions) {
            (Some(local), None) => Some(local),
            (Some(local), Some(remote)) => Some(if remote > local {
                self.install_version(&remote);
                remote
            } else {
                local
            }),
            (None, Some(version)) => {
                self.install_version(&version);
                Some(version)
            }
            // do nothing otherwise
            _ => None,
        }
        .map(|version| (version, path_str));

        Ok(res)
    }

    fn install_version(&self, version: &Version) {
        println!("Installing {}", version);
        // Blocking call to install it over RPC.
        install_blocking(version).expect("could not install solc remotely");
        println!("Done!");
    }

    /// Gets a map of compiler version -> vec[contract paths]
    fn contract_versions(&self) -> Result<HashMap<String, Vec<String>>> {
        // Group contracts in the nones with the same version pragma
        let files = glob::glob(self.contracts)?;
        // tracing::trace!("Compiling files under {}", self.contracts);
        println!("Compiling files under {}", self.contracts);

        // get all the corresponding contract versions
        let contracts = files
            .filter_map(|fname| fname.ok())
            .filter_map(|fname| self.detect_version(&fname).ok().flatten())
            .fold(HashMap::new(), |mut map, (version, path)| {
                let entry = map.entry(version.to_string()).or_insert_with(Vec::new);
                entry.push(path);
                map
            });

        if contracts.is_empty() {
            eyre::bail!(
                "no contracts were compiled. do you have the correct compiler version installed?"
            )
        }

        Ok(contracts)
    }

    /// Parses the given Solidity file looking for the `pragma` definition and
    /// returns the corresponding SemVer version requirement.
    fn version_req(path: &Path) -> Result<VersionReq> {
        let file = BufReader::new(File::open(path)?);
        let version = file
            .lines()
            .map(|line| line.unwrap())
            .find(|line| line.starts_with("pragma solidity"))
            .ok_or_else(|| eyre::eyre!("{:?} has no version", path))?;
        let version = version
            .replace("pragma solidity ", "")
            .replace(";", "")
            // needed to make it valid semver for things like
            // `>=0.4.0 <0.5.0` => `>=0.4.0,<0.5.0`
            .replace(" ", ",");

        // Somehow, Solidity semver without an operator is considered to be "exact",
        // but lack of operator automatically marks the operator as Caret, so we need
        // to manually patch it? :shrug:
        let exact = !matches!(&version[0..1], "*" | "^" | "=" | ">" | "<" | "~");
        let mut version = VersionReq::parse(&version)?;
        if exact {
            version.comparators[0].op = semver::Op::Exact;
        }

        Ok(version)
    }

    /// Find a matching local installation for the specified required version
    fn find_matching_installation(
        versions: &[Version],
        required_version: &VersionReq,
    ) -> Option<Version> {
        // iterate in reverse to find the last match
        versions.iter().rev().find(|version| required_version.matches(version)).cloned()
    }
}

/// Returns the path for an installed version
fn find_installed_version_path(version: &str) -> Result<Option<PathBuf>> {
    let home_dir = svm::SVM_HOME.clone();
    let path = std::fs::read_dir(home_dir)?
        .into_iter()
        .filter_map(|version| version.ok())
        .map(|version_dir| version_dir.path())
        .find(|path| path.to_string_lossy().contains(&version))
        .map(|mut path| {
            path.push(format!("solc-{}", &version));
            path
        });
    Ok(path)
}

/// Blocking call to the svm installer for a specified version
fn install_blocking(version: &Version) -> Result<()> {
    tokio::runtime::Runtime::new().unwrap().block_on(svm::install(version))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use ethers::core::rand::random;
    use svm::SVM_HOME;

    use super::*;
    use std::{io::Write, str::FromStr};

    #[test]
    fn test_find_installed_version_path() {
        // this test does not take the lock by default, so we need to manually
        // add it here.
        let _lock = LOCK.lock();
        let ver = "0.8.6";
        let version = Version::from_str(ver).unwrap();
        if !svm::installed_versions().unwrap().contains(&version) {
            install_blocking(&version).unwrap();
        }
        let res = find_installed_version_path(&version.to_string()).unwrap();
        let expected = SVM_HOME.join(ver).join(format!("solc-{}", ver));
        assert_eq!(res.unwrap(), expected);
    }

    #[test]
    fn does_not_find_not_installed_version() {
        let ver = "1.1.1";
        let version = Version::from_str(ver).unwrap();
        let res = find_installed_version_path(&version.to_string()).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_find_latest_matching_installation() {
        let versions = ["0.4.24", "0.5.1", "0.5.2"]
            .iter()
            .map(|version| Version::from_str(version).unwrap())
            .collect::<Vec<_>>();

        let required = VersionReq::from_str(">=0.4.24").unwrap();

        let got = SolcBuilder::find_matching_installation(&versions, &required).unwrap();
        assert_eq!(got, versions[2]);
    }

    #[test]
    fn test_no_matching_installation() {
        let versions = ["0.4.24", "0.5.1", "0.5.2"]
            .iter()
            .map(|version| Version::from_str(version).unwrap())
            .collect::<Vec<_>>();

        let required = VersionReq::from_str(">=0.6.0").unwrap();
        let got = SolcBuilder::find_matching_installation(&versions, &required);
        assert!(got.is_none());
    }

    // helper for testing solidity file versioning
    struct TempSolidityFile {
        version: String,
        path: PathBuf,
    }

    use std::ops::Deref;

    impl Deref for TempSolidityFile {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.path
        }
    }

    // mkdir -p
    fn mkdir() -> PathBuf {
        let dir = std::env::temp_dir().join(&random::<u64>().to_string()).join("contracts");
        if !dir.exists() {
            std::fs::create_dir_all(&dir).unwrap();
        }
        dir
    }

    // rm -rf
    fn rmdir(dir: &Path) {
        std::fs::remove_dir_all(&dir).unwrap();
    }

    impl TempSolidityFile {
        fn new(dir: &Path, version: &str) -> Self {
            let path = dir.join(format!("temp-{}-{}.sol", version, random::<u64>()));
            let mut file = File::create(&path).unwrap();
            file.write(format!("pragma solidity {};\n", version).as_bytes()).unwrap();
            Self { path, version: version.to_string() }
        }
    }

    #[test]
    fn test_version_req() {
        let dir = mkdir();

        let versions = ["=0.1.2", "^0.5.6", ">=0.7.1", ">0.8.0"];
        let files = versions.iter().map(|version| TempSolidityFile::new(&dir, version));

        files.for_each(|file| {
            let version_req = SolcBuilder::version_req(&file.path).unwrap();
            assert_eq!(version_req, VersionReq::from_str(&file.version).unwrap());
        });

        // Solidity defines version ranges with a space, whereas the semver package
        // requires them to be separated with a comma
        let version_range = ">=0.8.0 <0.9.0";
        let file = TempSolidityFile::new(&dir, version_range);
        let version_req = SolcBuilder::version_req(&file.path).unwrap();
        assert_eq!(version_req, VersionReq::from_str(">=0.8.0,<0.9.0").unwrap());

        rmdir(&dir);
    }

    #[test]
    // This test might be a bit hard t omaintain
    fn test_detect_version() {
        let dir = mkdir();

        let builder = SolcBuilder::new("", &[], &[]).unwrap();
        for (pragma, expected) in [
            // pinned
            ("=0.4.14", "0.4.14"),
            // pinned too
            ("0.4.14", "0.4.14"),
            // The latest patch is 0.4.26
            ("^0.4.14", "0.4.26"),
            // latest version above 0.5.0 -> we have to
            // update this test whenever there's a new sol
            // version. that's ok! good reminder to check the
            // patch notes.
            (">=0.5.0", "0.8.9"),
            // range
            (">=0.4.0 <0.5.0", "0.4.26"),
        ]
        .iter()
        {
            // println!("Checking {}", pragma);
            let file = TempSolidityFile::new(&dir, pragma);
            let res = builder.detect_version(&file.path).unwrap().unwrap();
            assert_eq!(res.0, Version::from_str(expected).unwrap());
        }

        rmdir(&dir);
    }

    #[test]
    // Ensures that the contract versions get correctly assigned to a compiler
    // version given a glob
    fn test_contract_versions() {
        let dir = mkdir();

        let versions = [
            // pinned
            "=0.4.14",
            // Up to later patches (caret implied)
            "0.4.14",
            // Up to later patches
            "^0.4.14",
            // any version above 0.5.0
            ">=0.5.0",
            // range
            ">=0.4.0 <0.5.0",
        ];
        versions.iter().for_each(|version| {
            TempSolidityFile::new(&dir, version);
        });

        let dir_str = dir.clone().into_os_string().into_string().unwrap();
        let glob = format!("{}/**/*.sol", dir_str);
        let builder = SolcBuilder::new(&glob, &[], &[]).unwrap();

        let versions = builder.contract_versions().unwrap();
        assert_eq!(versions["0.4.14"].len(), 2);
        assert_eq!(versions["0.4.26"].len(), 2);
        assert_eq!(versions["0.8.9"].len(), 1);

        rmdir(&dir);
    }

    fn get_glob(path: &str) -> String {
        let path = std::fs::canonicalize(path).unwrap();
        let mut path = path.into_os_string().into_string().unwrap();
        path.push_str("/**/*.sol");
        path
    }

    #[test]
    fn test_build_all_versions() {
        let path = get_glob("testdata/test-contract-versions");
        let builder = SolcBuilder::new(&path, &[], &[]).unwrap();
        let res = builder.build_all().unwrap();
        // Contracts A to F
        assert_eq!(res.keys().count(), 5);
    }

    #[test]
    fn test_remappings() {
        // Need to give the full paths here because we're running solc from the current
        // directory and not the repo's root directory
        let path = get_glob("testdata/test-contract-remappings");
        let remappings = vec!["bar/=testdata/test-contract-remappings/lib/bar/".to_owned()];
        let lib = std::fs::canonicalize("testdata/test-contract-remappings")
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap();
        let libs = vec![lib];
        let builder = SolcBuilder::new(&path, &remappings, &libs).unwrap();
        let res = builder.build_all().unwrap();
        // Foo & Bar
        assert_eq!(res.keys().count(), 2);
    }

    fn canonicalized_path(path: &str) -> String {
        std::fs::canonicalize(path).unwrap().into_os_string().into_string().unwrap()
    }

    #[test]
    // This is useful if you want to import a library from e.g. `node_modules` (for
    // whatever reason that may be) and from another path at the same time
    fn test_multiple_libs() {
        // Need to give the full paths here because we're running solc from the current
        // directory and not the repo's root directory
        let path = get_glob("testdata/test-contract-libs");
        let libs = vec![
            canonicalized_path("testdata/test-contract-libs/lib1"),
            canonicalized_path("testdata/test-contract-libs/lib2"),
        ];
        let builder = SolcBuilder::new(&path, &[], &libs).unwrap();
        let res = builder.build_all().unwrap();
        // Foo & Bar
        assert_eq!(res.keys().count(), 3);
    }
}
