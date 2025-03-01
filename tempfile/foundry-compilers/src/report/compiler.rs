//! Additional logging [SolcInput] and [CompilerOutput]
//!
//! Useful for debugging purposes.
//! As solc compiler input and output can become quite large (in the tens of MB) we still want a way
//! to get this info when debugging an issue. Most convenient way to look at these object is as a
//! separate json file

use foundry_compilers_artifacts::{CompilerOutput, SolcInput};
use semver::Version;
use std::{env, path::PathBuf, str::FromStr};

/// Debug Helper type that can be used to write the [crate::compilers::solc::Solc] [SolcInput] and
/// [CompilerOutput] to disk if configured.
///
/// # Examples
///
/// If `foundry_compilers_LOG=in=in.json,out=out.json` is then the reporter will be configured to
/// write the compiler input as pretty formatted json to `in.{solc version}.json` and the compiler
/// output to `out.{solc version}.json`
///
/// ```no_run
/// use foundry_compilers::report::SolcCompilerIoReporter;
/// std::env::set_var("foundry_compilers_LOG", "in=in.json,out=out.json");
/// let rep = SolcCompilerIoReporter::from_default_env();
/// ```
#[derive(Clone, Debug, Default)]
pub struct SolcCompilerIoReporter {
    /// where to write the output to, `None` if not enabled
    target: Option<Target>,
}

impl SolcCompilerIoReporter {
    /// Returns a new `SolcCompilerIOLayer` from the fields in the given string,
    /// ignoring any that are invalid.
    pub fn new(value: &str) -> Self {
        Self { target: Some(value.parse().unwrap_or_default()) }
    }

    /// `foundry_compilers_LOG` is the default environment variable used by
    /// [`SolcCompilerIOLayer::from_default_env`]
    ///
    /// [`SolcCompilerIOLayer::from_default_env`]: #method.from_default_env
    pub const DEFAULT_ENV: &'static str = "foundry_compilers_LOG";

    /// Returns a new `SolcCompilerIOLayer` from the value of the `foundry_compilers_LOG`
    /// environment variable, ignoring any invalid filter directives.
    pub fn from_default_env() -> Self {
        Self::from_env(Self::DEFAULT_ENV)
    }

    /// Returns a new `SolcCompilerIOLayer` from the value of the given environment
    /// variable, ignoring any invalid filter directives.
    pub fn from_env(env: impl AsRef<std::ffi::OsStr>) -> Self {
        env::var(env).map(|var| Self::new(&var)).unwrap_or_default()
    }

    /// Callback to write the input to disk if target is set
    pub fn log_compiler_input(&self, input: &SolcInput, version: &Version) {
        if let Some(target) = &self.target {
            target.write_input(input, version)
        }
    }

    /// Callback to write the input to disk if target is set
    pub fn log_compiler_output(&self, output: &CompilerOutput, version: &Version) {
        if let Some(target) = &self.target {
            target.write_output(output, version)
        }
    }
}

impl<S: AsRef<str>> From<S> for SolcCompilerIoReporter {
    fn from(s: S) -> Self {
        Self::new(s.as_ref())
    }
}

/// Represents the `in=<path>,out=<path>` value
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    /// path where the compiler input file should be written to
    dest_input: PathBuf,
    /// path where the compiler output file should be written to
    dest_output: PathBuf,
}

impl Target {
    fn write_input(&self, input: &SolcInput, version: &Version) {
        trace!("logging compiler input to {}", self.dest_input.display());
        match serde_json::to_string_pretty(input) {
            Ok(json) => {
                if let Err(err) = std::fs::write(get_file_name(&self.dest_input, version), json) {
                    error!("Failed to write compiler input: {}", err)
                }
            }
            Err(err) => {
                error!("Failed to serialize compiler input: {}", err)
            }
        }
    }

    fn write_output(&self, output: &CompilerOutput, version: &Version) {
        trace!("logging compiler output to {}", self.dest_output.display());
        match serde_json::to_string_pretty(output) {
            Ok(json) => {
                if let Err(err) = std::fs::write(get_file_name(&self.dest_output, version), json) {
                    error!("Failed to write compiler output: {}", err)
                }
            }
            Err(err) => {
                error!("Failed to serialize compiler output: {}", err)
            }
        }
    }
}

impl Default for Target {
    fn default() -> Self {
        Self {
            dest_input: "compiler-input.json".into(),
            dest_output: "compiler-output.json".into(),
        }
    }
}

impl FromStr for Target {
    type Err = Box<dyn std::error::Error + Send + Sync>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut dest_input = None;
        let mut dest_output = None;
        for part in s.split(',') {
            let (name, val) =
                part.split_once('=').ok_or_else(|| BadName { name: part.to_string() })?;
            match name {
                "i" | "in" | "input" | "compilerinput" => {
                    dest_input = Some(PathBuf::from(val));
                }
                "o" | "out" | "output" | "compileroutput" => {
                    dest_output = Some(PathBuf::from(val));
                }
                _ => return Err(BadName { name: part.to_string() }.into()),
            };
        }

        Ok(Self {
            dest_input: dest_input.unwrap_or_else(|| "compiler-input.json".into()),
            dest_output: dest_output.unwrap_or_else(|| "compiler-output.json".into()),
        })
    }
}

/// Indicates that a field name specified in the env value was invalid.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{}", self.name)]
pub struct BadName {
    name: String,
}

/// Returns the file name for the given version
fn get_file_name(path: impl Into<PathBuf>, v: &Version) -> PathBuf {
    let mut path = path.into();
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str().map(|s| s.to_string())) {
        path.set_file_name(format!("{stem}.{}.{}.{}.json", v.major, v.minor, v.patch));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn can_set_file_name() {
        let s = "/a/b/c/in.json";
        let p = get_file_name(s, &Version::new(0, 8, 10));
        assert_eq!(PathBuf::from("/a/b/c/in.0.8.10.json"), p);

        let s = "abc.json";
        let p = get_file_name(s, &Version::new(0, 8, 10));
        assert_eq!(PathBuf::from("abc.0.8.10.json"), p);
    }

    #[test]
    fn can_parse_target() {
        let target: Target = "in=in.json,out=out.json".parse().unwrap();
        assert_eq!(target, Target { dest_input: "in.json".into(), dest_output: "out.json".into() });

        let target: Target = "in=in.json".parse().unwrap();
        assert_eq!(target, Target { dest_input: "in.json".into(), ..Default::default() });

        let target: Target = "out=out.json".parse().unwrap();
        assert_eq!(target, Target { dest_output: "out.json".into(), ..Default::default() });
    }

    #[test]
    fn can_init_reporter_from_env() {
        let rep = SolcCompilerIoReporter::from_default_env();
        assert!(rep.target.is_none());
        std::env::set_var("foundry_compilers_LOG", "in=in.json,out=out.json");
        let rep = SolcCompilerIoReporter::from_default_env();
        assert!(rep.target.is_some());
        assert_eq!(
            rep.target.unwrap(),
            Target { dest_input: "in.json".into(), dest_output: "out.json".into() }
        );
        std::env::remove_var("foundry_compilers_LOG");
    }

    #[test]
    fn check_no_write_when_no_target() {
        let reporter = SolcCompilerIoReporter::default();
        let version = Version::parse("0.8.10").unwrap();
        let input = SolcInput::default();
        let output = CompilerOutput::default();

        reporter.log_compiler_input(&input, &version);
        reporter.log_compiler_output(&output, &version);
    }

    #[test]
    fn serialize_and_write_to_file() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("input.json");
        let output_path = dir.path().join("output.json");
        let version = Version::parse("0.8.10").unwrap();
        let target = Target { dest_input: input_path.clone(), dest_output: output_path.clone() };

        let input = SolcInput::default();
        let output = CompilerOutput::default();

        target.write_input(&input, &version);
        target.write_output(&output, &version);

        let input_content = fs::read_to_string(get_file_name(&input_path, &version)).unwrap();
        let output_content = fs::read_to_string(get_file_name(&output_path, &version)).unwrap();

        assert!(!input_content.is_empty());
        assert!(!output_content.is_empty());

        dir.close().unwrap();
    }
}
