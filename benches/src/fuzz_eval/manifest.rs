//! Benchmark manifest format.

use eyre::{Result, WrapErr, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
};

/// Default per-invocation timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Train/test split of a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    /// Targets a change may be tuned against.
    Train,
    /// Held-out targets that decide whether a change is kept.
    Test,
}

impl Split {
    /// Both splits in report order.
    pub const ALL: [Self; 2] = [Self::Train, Self::Test];

    /// Lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Test => "test",
        }
    }
}

impl fmt::Display for Split {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Fuzzing budget for a single forge invocation. Each value is passed to forge through the
/// matching `FOUNDRY_*` environment variable so the same budget applies to `forge test` and
/// `forge coverage` and works across forge versions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Stateless fuzz runs per test (`FOUNDRY_FUZZ_RUNS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuzz_runs: Option<u64>,
    /// Stateless fuzz timeout per test in seconds (`FOUNDRY_FUZZ_TIMEOUT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuzz_timeout: Option<u64>,
    /// Invariant runs per campaign (`FOUNDRY_INVARIANT_RUNS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_runs: Option<u64>,
    /// Calls per invariant run (`FOUNDRY_INVARIANT_DEPTH`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_depth: Option<u64>,
    /// Invariant campaign timeout in seconds (`FOUNDRY_INVARIANT_TIMEOUT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_timeout: Option<u64>,
}

impl Budget {
    /// Returns `self` with unset values taken from `fallback`.
    pub fn or(&self, fallback: &Self) -> Self {
        Self {
            fuzz_runs: self.fuzz_runs.or(fallback.fuzz_runs),
            fuzz_timeout: self.fuzz_timeout.or(fallback.fuzz_timeout),
            invariant_runs: self.invariant_runs.or(fallback.invariant_runs),
            invariant_depth: self.invariant_depth.or(fallback.invariant_depth),
            invariant_timeout: self.invariant_timeout.or(fallback.invariant_timeout),
        }
    }

    /// Environment variables that apply this budget, skipping values whose forge flag appears in
    /// `args`.
    ///
    /// Forge's `FOUNDRY_*` environment configuration takes precedence over the matching CLI flag,
    /// so a budget flag passed as a treatment (for example `--fuzz-runs 5000`) only takes effect
    /// if the harness does not also set the variable.
    pub fn env(&self, args: &[String]) -> Vec<(&'static str, String)> {
        [
            ("FOUNDRY_FUZZ_RUNS", "--fuzz-runs", self.fuzz_runs),
            ("FOUNDRY_FUZZ_TIMEOUT", "--fuzz-timeout", self.fuzz_timeout),
            ("FOUNDRY_INVARIANT_RUNS", "--invariant-runs", self.invariant_runs),
            ("FOUNDRY_INVARIANT_DEPTH", "--invariant-depth", self.invariant_depth),
            ("FOUNDRY_INVARIANT_TIMEOUT", "--invariant-timeout", self.invariant_timeout),
        ]
        .into_iter()
        .filter(|(_, flag, _)| {
            !args.iter().any(|arg| {
                arg == flag || arg.strip_prefix(flag).is_some_and(|rest| rest.starts_with('='))
            })
        })
        .filter_map(|(name, _, value)| value.map(|value| (name, value.to_string())))
        .collect()
    }
}

/// Top-level manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Settings shared by every target.
    #[serde(default)]
    pub defaults: Defaults,
    /// Benchmark targets.
    pub targets: Vec<TargetSpec>,
}

/// Settings shared by every target. Per-target values extend or override these.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Default fuzzing budget.
    #[serde(default)]
    pub budget: Budget,
    /// Per-invocation timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// Extra arguments for every `forge test` and `forge coverage` invocation.
    #[serde(default)]
    pub forge_args: Vec<String>,
    /// Extra arguments for `forge coverage` only.
    #[serde(default)]
    pub coverage_args: Vec<String>,
    /// Source path prefixes, relative to the project root, counted for coverage.
    pub coverage_paths: Option<Vec<String>>,
    /// Extra environment variables for every forge invocation.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// One benchmark target as written in the manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    /// Unique target name.
    pub name: String,
    /// Train or test split.
    pub split: Split,
    /// Foundry project directory relative to the manifest, or a subdirectory of `repo`.
    pub path: Option<PathBuf>,
    /// Git repository to clone into the cache directory.
    pub repo: Option<String>,
    /// Branch, tag, or commit of `repo`.
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// `--match-contract` filter.
    pub match_contract: Option<String>,
    /// `--match-test` filter.
    pub match_test: Option<String>,
    /// `--match-path` filter.
    pub match_path: Option<String>,
    /// Tests that fail when the planted bug is found, as `test` or `Contract::test`.
    #[serde(default)]
    pub expected_failures: Vec<String>,
    /// Count every failing test as a found bug. Used for targets whose bugs are not enumerated.
    #[serde(default)]
    pub failures_are_bugs: bool,
    /// Fuzzing budget overrides.
    #[serde(default)]
    pub budget: Budget,
    /// Per-invocation timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// Extra arguments for `forge test` and `forge coverage`, after the defaults.
    #[serde(default)]
    pub forge_args: Vec<String>,
    /// Extra arguments for `forge coverage` only, after the defaults.
    #[serde(default)]
    pub coverage_args: Vec<String>,
    /// Source path prefixes counted for coverage. Defaults to `["src/"]`.
    pub coverage_paths: Option<Vec<String>>,
    /// Extra environment variables, after the defaults.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Give every run a fresh, empty fuzz and invariant corpus directory. This enables
    /// coverage-guided mode; leave it unset for targets that do not configure a corpus.
    #[serde(default)]
    pub fresh_corpus: bool,
}

/// Where a target's project comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A local directory, copied into the work directory before running.
    Local {
        /// Absolute project directory.
        path: PathBuf,
    },
    /// A git checkout in the cache directory.
    Git {
        /// Repository URL.
        repo: String,
        /// Branch, tag, or commit.
        git_ref: String,
        /// Project subdirectory inside the checkout.
        subdir: Option<PathBuf>,
    },
}

/// A target with defaults applied.
#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub split: Split,
    pub source: Source,
    pub filters: Vec<String>,
    pub expected_failures: Vec<String>,
    pub failures_are_bugs: bool,
    pub budget: Budget,
    pub timeout_secs: u64,
    pub forge_args: Vec<String>,
    pub coverage_args: Vec<String>,
    pub coverage_paths: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub fresh_corpus: bool,
}

impl Manifest {
    /// Reads a TOML manifest.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::parse(&content).wrap_err_with(|| format!("invalid manifest {}", path.display()))
    }

    /// Parses a TOML manifest.
    pub fn parse(content: &str) -> Result<Self> {
        Ok(toml::from_str(content)?)
    }

    /// Validates the manifest and applies defaults. Relative local paths resolve against
    /// `base_dir`.
    pub fn resolve(&self, base_dir: &Path) -> Result<Vec<Target>> {
        ensure!(!self.targets.is_empty(), "manifest has no targets");
        let mut names = HashSet::new();
        let mut targets = Vec::with_capacity(self.targets.len());
        for spec in &self.targets {
            let target = self.resolve_target(spec, base_dir)?;
            ensure!(names.insert(target.name.clone()), "duplicate target name `{}`", target.name);
            targets.push(target);
        }
        Ok(targets)
    }

    fn resolve_target(&self, spec: &TargetSpec, base_dir: &Path) -> Result<Target> {
        let name = &spec.name;
        ensure!(
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "target name `{name}` must be non-empty and use only [A-Za-z0-9._-]"
        );
        let source = match (&spec.repo, &spec.git_ref, &spec.path) {
            (Some(repo), Some(git_ref), subdir) => {
                if let Some(subdir) = subdir {
                    ensure!(
                        subdir.is_relative()
                            && !subdir.components().any(|c| c == std::path::Component::ParentDir),
                        "target `{name}`: `path` must be a relative subdirectory when `repo` is set"
                    );
                }
                Source::Git { repo: repo.clone(), git_ref: git_ref.clone(), subdir: subdir.clone() }
            }
            (Some(_), None, _) => bail!("target `{name}`: `repo` requires `ref`"),
            (None, Some(_), _) => bail!("target `{name}`: `ref` requires `repo`"),
            (None, None, Some(path)) => Source::Local { path: base_dir.join(path) },
            (None, None, None) => bail!("target `{name}`: set either `path` or `repo` + `ref`"),
        };
        ensure!(
            !(spec.failures_are_bugs && !spec.expected_failures.is_empty()),
            "target `{name}`: `failures_are_bugs` and `expected_failures` are mutually exclusive"
        );
        let mut seen = HashSet::new();
        for expected in &spec.expected_failures {
            ensure!(
                seen.insert(expected),
                "target `{name}`: duplicate expected failure `{expected}`"
            );
        }

        let mut filters = Vec::new();
        for (flag, value) in [
            ("--match-contract", &spec.match_contract),
            ("--match-test", &spec.match_test),
            ("--match-path", &spec.match_path),
        ] {
            if let Some(value) = value {
                filters.extend([flag.to_string(), value.clone()]);
            }
        }
        let defaults = &self.defaults;
        let mut env = defaults.env.clone();
        env.extend(spec.env.clone());
        Ok(Target {
            name: name.clone(),
            split: spec.split,
            source,
            filters,
            expected_failures: spec.expected_failures.clone(),
            failures_are_bugs: spec.failures_are_bugs,
            budget: spec.budget.or(&defaults.budget),
            timeout_secs: spec
                .timeout_secs
                .or(defaults.timeout_secs)
                .unwrap_or(DEFAULT_TIMEOUT_SECS),
            forge_args: defaults.forge_args.iter().chain(&spec.forge_args).cloned().collect(),
            coverage_args: defaults
                .coverage_args
                .iter()
                .chain(&spec.coverage_args)
                .cloned()
                .collect(),
            coverage_paths: spec
                .coverage_paths
                .clone()
                .or_else(|| defaults.coverage_paths.clone())
                .unwrap_or_else(|| vec!["src/".to_string()]),
            env,
            fresh_corpus: spec.fresh_corpus,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
[defaults]
timeout_secs = 30
forge_args = ["--threads", "1"]
env = { FOUNDRY_INVARIANT_SHRINK_RUN_LIMIT = "0" }

[defaults.budget]
fuzz_runs = 100
invariant_runs = 8

[[targets]]
name = "local"
split = "train"
path = "project"
match_contract = "VaultTest"
expected_failures = ["invariant_solvent"]
budget = { invariant_runs = 2, invariant_depth = 4 }
coverage_paths = ["src/Vault.sol"]

[[targets]]
name = "remote"
split = "test"
repo = "https://example.com/repo.git"
ref = "v1"
path = "contracts"
failures_are_bugs = true
forge_args = ["--fail-fast"]
env = { FOUNDRY_INVARIANT_SHRINK_RUN_LIMIT = "5" }
"#;

    #[test]
    fn resolves_defaults_and_overrides() {
        let targets = Manifest::parse(MANIFEST).unwrap().resolve(Path::new("/m")).unwrap();
        let local = &targets[0];
        assert_eq!(local.source, Source::Local { path: PathBuf::from("/m/project") });
        assert_eq!(local.filters, ["--match-contract", "VaultTest"]);
        assert_eq!(
            local.budget,
            Budget {
                fuzz_runs: Some(100),
                invariant_runs: Some(2),
                invariant_depth: Some(4),
                ..Default::default()
            }
        );
        assert_eq!(local.timeout_secs, 30);
        assert_eq!(local.coverage_paths, ["src/Vault.sol"]);

        let remote = &targets[1];
        assert_eq!(
            remote.source,
            Source::Git {
                repo: "https://example.com/repo.git".to_string(),
                git_ref: "v1".to_string(),
                subdir: Some(PathBuf::from("contracts")),
            }
        );
        assert_eq!(remote.forge_args, ["--threads", "1", "--fail-fast"]);
        assert_eq!(remote.coverage_paths, ["src/"]);
        assert_eq!(remote.env["FOUNDRY_INVARIANT_SHRINK_RUN_LIMIT"], "5");
    }

    #[test]
    fn budget_env_skips_unset_values() {
        let budget = Budget { fuzz_runs: Some(7), invariant_depth: Some(3), ..Default::default() };
        assert_eq!(
            budget.env(&[]),
            [("FOUNDRY_FUZZ_RUNS", "7".to_string()), ("FOUNDRY_INVARIANT_DEPTH", "3".to_string())]
        );
    }

    #[test]
    fn budget_env_yields_to_forge_flags() {
        let budget = Budget { fuzz_runs: Some(7), invariant_depth: Some(3), ..Default::default() };
        let args = ["--fuzz-runs".to_string(), "5000".to_string()];
        assert_eq!(budget.env(&args), [("FOUNDRY_INVARIANT_DEPTH", "3".to_string())]);
        let args = ["--invariant-depth=64".to_string(), "--fuzz-runs-extra".to_string()];
        assert_eq!(budget.env(&args), [("FOUNDRY_FUZZ_RUNS", "7".to_string())]);
    }

    #[test]
    fn rejects_invalid_targets() {
        let cases = [
            ("name = \"a\"\nsplit = \"train\"", "target `a`: set either `path` or `repo` + `ref`"),
            ("name = \"a\"\nsplit = \"train\"\nrepo = \"r\"", "target `a`: `repo` requires `ref`"),
            (
                "name = \"a b\"\nsplit = \"train\"\npath = \"p\"",
                "target name `a b` must be non-empty and use only [A-Za-z0-9._-]",
            ),
            (
                "name = \"a\"\nsplit = \"train\"\npath = \"p\"\nfailures_are_bugs = true\nexpected_failures = [\"t\"]",
                "target `a`: `failures_are_bugs` and `expected_failures` are mutually exclusive",
            ),
            (
                "name = \"a\"\nsplit = \"train\"\npath = \"p\"\nexpected_failures = [\"t\", \"t\"]",
                "target `a`: duplicate expected failure `t`",
            ),
        ];
        for (target, message) in cases {
            let manifest = Manifest::parse(&format!("[[targets]]\n{target}\n")).unwrap();
            let err = manifest.resolve(Path::new("/")).unwrap_err();
            assert_eq!(err.to_string(), message);
        }
        let duplicate = "[[targets]]\nname = \"a\"\nsplit = \"train\"\npath = \"p\"\n\
                         [[targets]]\nname = \"a\"\nsplit = \"test\"\npath = \"q\"\n";
        let err = Manifest::parse(duplicate).unwrap().resolve(Path::new("/")).unwrap_err();
        assert_eq!(err.to_string(), "duplicate target name `a`");
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = Manifest::parse("[[targets]]\nname = \"a\"\nsplit = \"train\"\nfuzz_runs = 1\n")
            .unwrap_err();
        assert!(err.to_string().contains("unknown field `fuzz_runs`"), "{err}");
    }
}
