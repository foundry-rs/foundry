use self::{input::VyperVersionedInput, parser::VyperParsedSource};
use super::{Compiler, CompilerOutput, Language};
pub use crate::artifacts::vyper::{VyperCompilationError, VyperInput, VyperOutput, VyperSettings};
use core::fmt;
use foundry_compilers_artifacts::{sources::Source, Contract};
use foundry_compilers_core::error::{Result, SolcError};
use semver::Version;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};

pub mod error;
pub mod input;
mod output;
pub mod parser;
pub mod settings;

/// File extensions that are recognized as Vyper source files.
pub const VYPER_EXTENSIONS: &[&str] = &["vy", "vyi"];

/// Extension of Vyper interface file.
pub const VYPER_INTERFACE_EXTENSION: &str = "vyi";

/// Vyper language, used as [Compiler::Language] for the Vyper compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VyperLanguage;

impl serde::Serialize for VyperLanguage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("vyper")
    }
}

impl<'de> serde::Deserialize<'de> for VyperLanguage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let res = String::deserialize(deserializer)?;
        if res != "vyper" {
            Err(serde::de::Error::custom(format!("Invalid Vyper language: {res}")))
        } else {
            Ok(Self)
        }
    }
}

impl Language for VyperLanguage {
    const FILE_EXTENSIONS: &'static [&'static str] = VYPER_EXTENSIONS;
}

impl fmt::Display for VyperLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vyper")
    }
}

/// Vyper compiler. Wrapper aound vyper binary.
#[derive(Clone, Debug)]
pub struct Vyper {
    pub path: PathBuf,
    pub version: Version,
}

impl Vyper {
    /// Creates a new instance of the Vyper compiler. Uses the `vyper` binary in the system `PATH`.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let version = Self::version(path.clone())?;
        Ok(Self { path, version })
    }

    /// Convenience function for compiling all sources under the given path
    pub fn compile_source(&self, path: &Path) -> Result<VyperOutput> {
        let input = VyperInput::new(
            Source::read_all_from(path, VYPER_EXTENSIONS)?,
            Default::default(),
            &self.version,
        );
        self.compile(&input)
    }

    /// Same as [`Self::compile()`], but only returns those files which are included in the
    /// `CompilerInput`.
    ///
    /// In other words, this removes those files from the `VyperOutput` that are __not__
    /// included in the provided `CompilerInput`.
    ///
    /// # Examples
    pub fn compile_exact(&self, input: &VyperInput) -> Result<VyperOutput> {
        let mut out = self.compile(input)?;
        out.retain_files(input.sources.keys().map(|p| p.as_path()));
        Ok(out)
    }

    /// Compiles with `--standard-json` and deserializes the output as [`VyperOutput`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use foundry_compilers::{
    ///     artifacts::{
    ///         vyper::{VyperInput, VyperSettings},
    ///         Source,
    ///     },
    ///     Vyper,
    /// };
    /// use std::path::Path;
    ///
    /// let vyper = Vyper::new("vyper")?;
    /// let path = Path::new("path/to/sources");
    /// let sources = Source::read_all_from(path, &["vy", "vyi"])?;
    /// let input = VyperInput::new(sources, VyperSettings::default(), &vyper.version);
    /// let output = vyper.compile(&input)?;
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn compile<T: Serialize>(&self, input: &T) -> Result<VyperOutput> {
        self.compile_as(input)
    }

    /// Compiles with `--standard-json` and deserializes the output as the given `D`.
    pub fn compile_as<T: Serialize, D: DeserializeOwned>(&self, input: &T) -> Result<D> {
        let output = self.compile_output(input)?;

        // Only run UTF-8 validation once.
        let output = std::str::from_utf8(&output).map_err(|_| SolcError::InvalidUtf8)?;

        trace!("vyper compiler output: {}", output);

        Ok(serde_json::from_str(output)?)
    }

    /// Compiles with `--standard-json` and returns the raw `stdout` output.
    #[instrument(name = "compile", level = "debug", skip_all)]
    pub fn compile_output<T: Serialize>(&self, input: &T) -> Result<Vec<u8>> {
        let mut cmd = Command::new(&self.path);
        cmd.arg("--standard-json")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        trace!(input=%serde_json::to_string(input).unwrap_or_else(|e| e.to_string()));
        debug!(?cmd, "compiling");

        let mut child = cmd.spawn().map_err(self.map_io_err())?;
        debug!("spawned");

        {
            let mut stdin = io::BufWriter::new(child.stdin.take().unwrap());
            serde_json::to_writer(&mut stdin, input)?;
            stdin.flush().map_err(self.map_io_err())?;
        }
        debug!("wrote JSON input to stdin");

        let output = child.wait_with_output().map_err(self.map_io_err())?;
        debug!(%output.status, output.stderr = ?String::from_utf8_lossy(&output.stderr), "finished");

        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(SolcError::solc_output(&output))
        }
    }

    /// Invokes `vyper --version` and parses the output as a SemVer [`Version`].
    #[instrument(level = "debug", skip_all)]
    pub fn version(vyper: impl Into<PathBuf>) -> Result<Version> {
        crate::cache_version(vyper.into(), &[], |vyper| {
            let mut cmd = Command::new(vyper);
            cmd.arg("--version")
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped());
            debug!(?cmd, "getting Vyper version");
            let output = cmd.output().map_err(|e| SolcError::io(e, vyper))?;
            trace!(?output);
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(Version::from_str(
                    &stdout.trim().replace("rc", "-rc").replace("b", "-b").replace("a", "-a"),
                )?)
            } else {
                Err(SolcError::solc_output(&output))
            }
        })
    }

    fn map_io_err(&self) -> impl FnOnce(std::io::Error) -> SolcError + '_ {
        move |err| SolcError::io(err, &self.path)
    }
}

impl Compiler for Vyper {
    type Settings = VyperSettings;
    type CompilationError = VyperCompilationError;
    type ParsedSource = VyperParsedSource;
    type Input = VyperVersionedInput;
    type Language = VyperLanguage;
    type CompilerContract = Contract;

    fn compile(
        &self,
        input: &Self::Input,
    ) -> Result<CompilerOutput<VyperCompilationError, Contract>> {
        self.compile(input).map(Into::into)
    }

    fn available_versions(&self, _language: &Self::Language) -> Vec<super::CompilerVersion> {
        vec![super::CompilerVersion::Installed(Version::new(
            self.version.major,
            self.version.minor,
            self.version.patch,
        ))]
    }
}
