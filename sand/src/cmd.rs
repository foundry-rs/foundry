//! cairo-lang/starknet cli bindings

use crate::{
    error::{Result, SandError},
    utils,
};
use semver::Version;
use starknet::core::types::ContractCode;
use std::{
    fmt,
    io::BufRead,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    str::FromStr,
};

/// The Compiler target
///
/// The [cairo-lang](https://github.com/starkware-libs/cairo-lang) package includes the
/// `cairo-compile` and `starknet-compile` executables
#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub enum Target {
    /// Represents the `cairo-compile` executable
    Cairo,
    /// Represents the `starkent-compile` executable
    Starknet,
}

impl Target {
    /// The name of the executable
    fn bin_name(&self) -> &'static str {
        match self {
            Target::Cairo => "cairo-compile",
            Target::Starknet => "starknet-compile",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.bin_name())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StarknetCompile {
    /// The path to the `starknet-compile` executable
    bin: PathBuf,
    /// Additional arguments passed to the compiler executable
    import_paths: Vec<PathBuf>,
}

impl StarknetCompile {
    /// Constructs a new `StarknetCompile` for launching the `starknet-compile` command `bin`
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into(), import_paths: Default::default() }
    }

    pub fn get_import_paths(&self) -> &Vec<PathBuf> {
        &self.import_paths
    }

    pub fn get_import_paths_mut(&mut self) -> &mut Vec<PathBuf> {
        &mut self.import_paths
    }

    /// Adds a path to pass to the compiler's `--cairo-path` argument
    #[must_use]
    pub fn import_path(mut self, p: impl Into<PathBuf>) -> Self {
        self.import_paths.push(p.into());
        self
    }

    /// Adds
    #[must_use]
    pub fn import_paths<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<PathBuf>,
    {
        for arg in args {
            self = self.import_path(arg);
        }
        self
    }

    /// returns the import paths as concatenated by ":".
    fn import_paths_arg(&self) -> Option<String> {
        if !self.import_paths.is_empty() {
            Some(
                self.import_paths
                    .iter()
                    .map(|p| format!("{}", p.display()))
                    .collect::<Vec<_>>()
                    .join(":"),
            )
        } else {
            None
        }
    }

    /// Bindings for [cairo-lang-docker](https://github.com/Shard-Labs/cairo-cli-docker)
    ///
    /// Compared to [`StarknetCompile::docker_unchecked()`] this also checks that docker daemon is
    /// running
    pub fn docker() -> Result<Self> {
        todo!()
    }

    pub fn docker_unchecked() -> Self {
        todo!()
    }

    pub fn docker_tag(_tag: impl AsRef<str>) -> Self {
        todo!()
    }

    pub fn compile_dir(&self, contracts_dir: impl AsRef<Path>) -> Result<Vec<ContractCode>> {
        self.compile_all(utils::cairo_files(contracts_dir))
    }

    pub fn compile_all(
        &self,
        files: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<Vec<ContractCode>> {
        files.into_iter().map(|file| self.compile_contract(file)).collect()
    }

    pub fn compile_contract(&self, file: impl AsRef<Path>) -> Result<ContractCode> {
        let mut cmd = Command::new(&self.bin);
        if let Some(cairo_path) = self.import_paths_arg() {
            cmd.arg("--cairo-path").arg(cairo_path);
        }
        cmd.arg(file.as_ref());

        let output = successful_output(
            cmd.stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|err| SandError::io(err, &self.bin))?
                .wait_with_output()
                .map_err(|err| SandError::io(err, &self.bin))?,
        )?;

        Ok(serde_json::from_slice(&output)?)
    }

    /// Returns the version from the configured `solc`
    pub fn version(&self) -> Result<Version> {
        version_from_output(
            Command::new(&self.bin)
                .arg("--version")
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()
                .map_err(|err| SandError::io(err, &self.bin))?,
            Target::Starknet,
        )
    }
}

impl Default for StarknetCompile {
    fn default() -> Self {
        if let Ok(starknet_compile) = std::env::var("STARKNET_COMPILE_PATH") {
            return StarknetCompile::new(starknet_compile)
        }
        StarknetCompile::new(Target::Starknet.bin_name())
    }
}

fn successful_output(output: Output) -> Result<Vec<u8>> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(SandError::CompilerError(String::from_utf8_lossy(&output.stderr).to_string()))
    }
}

/// Extracts the version from the command output, such as `starknet-compile 0.7.0`
fn version_from_output(output: Output, target: Target) -> Result<Version> {
    if output.status.success() {
        let version = output
            .stdout
            .lines()
            .last()
            .ok_or(SandError::VersionNotFound(target))?
            .map_err(|err| SandError::msg(format!("Failed to read output: {}", err)))?;
        Ok(Version::from_str(version.trim_start_matches(target.bin_name()).trim_start())?)
    } else {
        Err(SandError::msg(String::from_utf8_lossy(&output.stderr).to_string()))
    }
}

impl fmt::Display for StarknetCompile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.bin.display())?;
        if let Some(cairo_path) = self.import_paths_arg() {
            write!(f, "--cairo-path {}", cairo_path)?;
        }
        Ok(())
    }
}
