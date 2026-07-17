use crate::{Config, foundry_toml_dirs, remappings_from_env_var, remappings_from_newline};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    value::{Dict, Map},
};
use foundry_compilers::artifacts::remappings::{RelativeRemapping, Remapping};
use rayon::prelude::*;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet, btree_map::Entry},
    fs,
    path::{Component, Path, PathBuf},
};

/// Wrapper types over a `Vec<Remapping>` that only appends unique remappings.
#[derive(Clone, Debug, Default)]
pub struct Remappings {
    /// Remappings.
    remappings: Vec<Remapping>,
    /// Source, test and script configured project dirs.
    /// Remappings of these dirs from libs are ignored.
    project_paths: Vec<Remapping>,
}

impl Remappings {
    /// Create a new `Remappings` wrapper with an empty vector.
    pub const fn new() -> Self {
        Self { remappings: Vec::new(), project_paths: Vec::new() }
    }

    /// Create a new `Remappings` wrapper with a vector of remappings.
    pub const fn new_with_remappings(remappings: Vec<Remapping>) -> Self {
        Self { remappings, project_paths: Vec::new() }
    }

    /// Extract project paths that cannot be remapped by dependencies.
    pub fn with_figment(mut self, figment: &Figment) -> Self {
        let mut add_project_remapping = |path: &str| {
            if let Ok(path) = figment.find_value(path)
                && let Some(path) = path.into_string()
            {
                let remapping =
                    Remapping { context: None, name: format!("{path}/"), path: format!("{path}/") };
                self.project_paths.push(remapping);
            }
        };
        add_project_remapping("src");
        add_project_remapping("test");
        add_project_remapping("script");
        self
    }

    /// Filters the remappings vector by name and context.
    fn filter_key(r: &Remapping) -> String {
        match &r.context {
            Some(str) => str.clone() + &r.name.clone(),
            None => r.name.clone(),
        }
    }

    /// Consumes the wrapper and returns the inner remappings vector.
    pub fn into_inner(self) -> Vec<Remapping> {
        let mut seen = HashSet::new();
        self.remappings.iter().filter(|r| seen.insert(Self::filter_key(r))).cloned().collect()
    }

    /// Push an element to the remappings vector, but only if it's not already present.
    fn push(&mut self, remapping: Remapping) {
        // Special handling for .sol file remappings, only allow one remapping per source file.
        if remapping.name.ends_with(".sol") && !remapping.path.ends_with(".sol") {
            return;
        }

        // Root remappings remain authoritative over dependency remappings, including single-file
        // remappings which use different duplicate handling below.
        if remapping.context.is_some()
            && self.remappings.iter().any(|existing| {
                if existing.context.is_some() {
                    return false;
                }
                let mut existing_name_path = existing.name.clone();
                if !existing_name_path.ends_with('/') {
                    existing_name_path.push('/');
                }
                existing.name == remapping.name
                    || remapping.name.starts_with(&existing_name_path)
                    || existing.name.starts_with(&remapping.name)
            })
        {
            return;
        }

        if self.remappings.iter().any(|existing| {
            if remapping.name.ends_with(".sol") {
                // For .sol files, only prevent duplicate source names in the same context
                return existing.name == remapping.name
                    && existing.context == remapping.context
                    && existing.path == remapping.path;
            }

            // What we're doing here is filtering for ambiguous paths. For example, if we have
            // @prb/math/=node_modules/@prb/math/src/ as existing, and
            // @prb/=node_modules/@prb/ as the one being checked,
            // we want to keep the already existing one, which is the first one. This way we avoid
            // having to deal with ambiguous paths which is unwanted when autodetecting remappings.
            // Remappings are added from root of the project down to libraries, so
            // we also want to exclude any conflicting remappings added from libraries. For example,
            // if we have `@utils/=src/` added in project remappings and `@utils/libraries/=src/`
            // added in a dependency, we don't want to add the new one as it conflicts with project
            // existing remapping.
            let mut existing_name_path = existing.name.clone();
            if !existing_name_path.ends_with('/') {
                existing_name_path.push('/')
            }
            let is_conflicting = remapping.name.starts_with(&existing_name_path)
                || existing.name.starts_with(&remapping.name);
            is_conflicting && existing.context == remapping.context
        }) {
            return;
        };

        // Ignore remappings of root project src, test or script dir.
        // See <https://github.com/foundry-rs/foundry/issues/3440>.
        if self
            .project_paths
            .iter()
            .any(|project_path| remapping.name.eq_ignore_ascii_case(&project_path.name))
        {
            return;
        };

        self.remappings.push(remapping);
    }

    /// Extend the remappings vector, leaving out the remappings that are already present.
    pub fn extend(&mut self, remappings: Vec<Remapping>) {
        for remapping in remappings {
            self.push(remapping);
        }
    }
}

/// A figment provider that checks if the remappings were previously set and if they're unset looks
/// up the fs via
///   - `DAPP_REMAPPINGS` || `FOUNDRY_REMAPPINGS` env var
///   - `<root>/remappings.txt` file
///   - `Remapping::find_many`.
pub struct RemappingsProvider<'a> {
    /// Whether to auto detect remappings from the `lib_paths`
    pub auto_detect_remappings: bool,
    /// Whether to include remappings from the process environment.
    pub include_env_remappings: bool,
    /// The lib/dependency directories to scan for remappings
    pub lib_paths: Cow<'a, Vec<PathBuf>>,
    /// the root path used to turn an absolute `Remapping`, as we're getting it from
    /// `Remapping::find_many` into a relative one.
    pub root: &'a Path,
    /// This contains either:
    ///   - previously set remappings
    ///   - a `MissingField` error, which means previous provider didn't set the "remappings" field
    ///   - other error, like formatting
    pub remappings: Result<Vec<Remapping>, Error>,
}

impl RemappingsProvider<'_> {
    /// Find and parse remappings for the projects
    ///
    /// **Order**
    ///
    /// Remappings are built in this order (last item takes precedence)
    /// - Autogenerated remappings
    /// - toml remappings
    /// - `remappings.txt`
    /// - Environment variables
    /// - CLI parameters
    fn get_remappings(&self, remappings: Vec<Remapping>) -> Result<Vec<Remapping>, Error> {
        trace!("get all remappings from {:?}", self.root);
        /// prioritizes remappings that are closer: shorter `path`
        ///   - ("a", "1/2") over ("a", "1/2/3")
        ///
        /// grouped by remapping context
        fn insert_closest(
            mappings: &mut BTreeMap<Option<String>, BTreeMap<String, PathBuf>>,
            context: Option<String>,
            key: String,
            path: PathBuf,
        ) {
            let context_mappings = mappings.entry(context).or_default();
            match context_mappings.entry(key) {
                Entry::Occupied(mut e)
                    if e.get().components().count() > path.components().count() =>
                {
                    e.insert(path);
                }
                Entry::Vacant(e) => {
                    e.insert(path);
                }
                _ => {}
            }
        }

        // Let's first just extend the remappings with the ones that were passed in,
        // without any filtering.
        let mut user_remappings = Vec::new();

        // check env vars
        if self.include_env_remappings
            && let Some(env_remappings) = remappings_from_env_var("DAPP_REMAPPINGS")
                .or_else(|| remappings_from_env_var("FOUNDRY_REMAPPINGS"))
        {
            user_remappings
                .extend(env_remappings.map_err::<Error, _>(|err| err.to_string().into())?);
        }

        // check remappings.txt file
        let remappings_file = self.root.join("remappings.txt");
        if remappings_file.is_file() {
            let content = fs::read_to_string(remappings_file).map_err(|err| err.to_string())?;
            let remappings_from_file: Result<Vec<_>, _> =
                remappings_from_newline(&content).collect();
            user_remappings
                .extend(remappings_from_file.map_err::<Error, _>(|err| err.to_string().into())?);
        }

        user_remappings.extend(remappings);
        // Let's now use the wrapper to conditionally extend the remappings with the autodetected
        // ones. We want to avoid duplicates, and the wrapper will handle this for us.
        let mut all_remappings = Remappings::new_with_remappings(user_remappings);

        // scan all library dirs and autodetect remappings
        if self.auto_detect_remappings {
            let (nested_foundry_remappings, auto_detected_remappings) = rayon::join(
                || self.find_nested_foundry_remappings(),
                || self.auto_detect_remappings(),
            );

            // Root remappings remain first, followed by dependency-scoped remappings and global
            // filesystem fallbacks. The compiler resolves the first applicable remapping, so a
            // dependency's configuration applies within that dependency without affecting root or
            // sibling imports.
            let (dependency_remappings, package_remappings): (Vec<_>, Vec<_>) =
                nested_foundry_remappings.partition(|remapping| remapping.context.is_some());
            all_remappings.extend(dependency_remappings);

            let mut lib_remappings = BTreeMap::new();
            for r in auto_detected_remappings {
                // this is an additional safety check for weird auto-detected remappings
                if ["lib/", "src/", "contracts/"].contains(&r.name.as_str()) {
                    trace!(target: "forge", "- skipping the remapping");
                    continue;
                }
                insert_closest(&mut lib_remappings, r.context, r.name, r.path.into());
            }

            all_remappings.extend(
                lib_remappings
                    .into_iter()
                    .flat_map(|(context, remappings)| {
                        remappings.into_iter().map(move |(name, path)| Remapping {
                            context: context.clone(),
                            name,
                            path: path.to_string_lossy().into(),
                        })
                    })
                    .collect(),
            );
            // Configured source directories fill package aliases that filesystem detection could
            // not infer, without narrowing an existing package-root mapping.
            let mut package_lib_remappings = BTreeMap::new();
            for r in package_remappings {
                insert_closest(&mut package_lib_remappings, r.context, r.name, r.path.into());
            }
            all_remappings.extend(
                package_lib_remappings
                    .into_iter()
                    .flat_map(|(context, remappings)| {
                        remappings.into_iter().map(move |(name, path)| Remapping {
                            context: context.clone(),
                            name,
                            path: path.to_string_lossy().into(),
                        })
                    })
                    .collect(),
            );
        }

        Ok(all_remappings.into_inner())
    }

    /// Returns all remappings declared in foundry.toml files of libraries
    fn find_nested_foundry_remappings(&self) -> impl Iterator<Item = Remapping> + '_ {
        let mut pending = self
            .lib_paths
            .iter()
            .flat_map(|p| {
                if p.is_absolute() {
                    vec![p.clone(), self.root.join("lib")]
                } else {
                    vec![self.root.join(p)]
                }
            })
            .flat_map(foundry_toml_dirs)
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let mut remappings = Vec::new();

        while let Some(lib) = pending.pop() {
            let Ok(lib) = dunce::canonicalize(lib) else { continue };
            if !seen.insert(lib.clone()) {
                continue;
            }

            trace!(?lib, "find all remappings of nested foundry.toml");
            if let Some((mut nested_remappings, nested_libs)) = self.nested_foundry_remappings(&lib)
            {
                remappings.append(&mut nested_remappings);
                pending.extend(nested_libs.into_iter().flat_map(foundry_toml_dirs));
            }
        }

        // Import resolution uses the first applicable remapping. Prefer deeper dependency contexts
        // and longer import prefixes before their broader fallbacks.
        remappings.sort_by(|a, b| {
            let context_depth = |r: &Remapping| {
                r.context.as_deref().map(Path::new).map_or(0, |p| p.components().count())
            };
            context_depth(b)
                .cmp(&context_depth(a))
                .then_with(|| b.name.len().cmp(&a.name.len()))
                .then_with(|| a.context.cmp(&b.context))
                .then_with(|| a.name.cmp(&b.name))
        });
        remappings.into_iter()
    }

    fn nested_foundry_remappings(&self, lib: &Path) -> Option<(Vec<Remapping>, Vec<PathBuf>)> {
        // load config of the nested lib if it exists, using fallback mode since libs may not
        // define all profiles the main project uses
        let Ok(config) = Config::load_with_root_and_fallback_without_auto_detected_remappings(lib)
        else {
            return None;
        };
        let config = config.sanitized();
        let nested_libs = config.libs.clone();

        // Preserve a global package entry point for root and sibling imports. Dependency-declared
        // aliases are scoped below, but this synthesized remapping describes the package itself.
        let src_remapping = lib.file_name().and_then(|s| s.to_str()).map(|name| {
            let mut r = Remapping {
                context: None,
                name: format!("{name}/"),
                path: config.src.display().to_string(),
            };
            if !r.path.ends_with('/') {
                r.path.push('/')
            }
            r
        });

        let mut remappings =
            config.remappings.into_iter().map(Remapping::from).collect::<Vec<Remapping>>();

        remappings = remappings
            .into_iter()
            .filter_map(|remapping| Self::with_dependency_context(remapping, lib))
            .collect();
        if let Some(r) = src_remapping {
            remappings.push(r);
        }
        Some((remappings, nested_libs))
    }

    fn with_dependency_context(mut remapping: Remapping, lib: &Path) -> Option<Remapping> {
        let context = if let Some(context) = remapping.context.take() {
            let context = Path::new(&context);
            if context.is_absolute()
                || context.components().any(|component| component == Component::ParentDir)
            {
                trace!(?context, ?lib, "skipping dependency remapping with escaping context");
                return None;
            }
            lib.join(context)
        } else {
            lib.to_path_buf()
        };
        let Ok(context) = dunce::canonicalize(context) else { return None };
        if !context.starts_with(lib) {
            trace!(?context, ?lib, "skipping dependency remapping with escaping context");
            return None;
        }

        let mut context = context.to_string_lossy().into_owned();
        if !context.ends_with('/') {
            context.push('/');
        }
        remapping.context = Some(context);
        Some(remapping)
    }

    /// Auto detect remappings from the lib paths
    fn auto_detect_remappings(&self) -> impl Iterator<Item = Remapping> + '_ {
        self.lib_paths
            .par_iter()
            .flat_map_iter(|lib| {
                let lib = self.root.join(lib);
                trace!(?lib, "find all remappings");
                Remapping::find_many(&lib)
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl Provider for RemappingsProvider<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Remapping Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let remappings = match &self.remappings {
            Ok(remappings) => self.get_remappings(remappings.clone()),
            Err(err) => {
                if let figment::error::Kind::MissingField(_) = err.kind {
                    self.get_remappings(vec![])
                } else {
                    return Err(err.clone());
                }
            }
        }?;

        // turn the absolute remapping into a relative one by stripping the `root`
        let remappings = remappings
            .into_iter()
            .map(|r| {
                let mut r = RelativeRemapping::new(r, self.root);
                if let Some(context) = &mut r.context
                    && !context.ends_with('/')
                {
                    context.push('/');
                }
                r.to_string()
            })
            .collect::<Vec<_>>();

        Ok(Map::from([(
            Config::selected_profile(),
            Dict::from([("remappings".to_string(), figment::value::Value::from(remappings))]),
        )]))
    }

    fn profile(&self) -> Option<Profile> {
        Some(Config::selected_profile())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn test_sol_file_remappings() {
        let mut remappings = Remappings::new();

        // First valid remapping
        remappings.push(Remapping {
            context: None,
            name: "MyContract.sol".to_string(),
            path: "implementations/Contract1.sol".to_string(),
        });

        // Same source to different target (should be rejected)
        remappings.push(Remapping {
            context: None,
            name: "MyContract.sol".to_string(),
            path: "implementations/Contract2.sol".to_string(),
        });

        // Different source to same target (should be allowed)
        remappings.push(Remapping {
            context: None,
            name: "OtherContract.sol".to_string(),
            path: "implementations/Contract1.sol".to_string(),
        });

        // Exact duplicate (should be silently ignored)
        remappings.push(Remapping {
            context: None,
            name: "MyContract.sol".to_string(),
            path: "implementations/Contract1.sol".to_string(),
        });

        // Invalid .sol remapping (target not .sol)
        remappings.push(Remapping {
            context: None,
            name: "Invalid.sol".to_string(),
            path: "implementations/Contract1.txt".to_string(),
        });

        let result = remappings.into_inner();
        assert_eq!(result.len(), 2, "Should only have 2 valid remappings");

        // Verify the correct remappings exist
        assert!(
            result
                .iter()
                .any(|r| r.name == "MyContract.sol" && r.path == "implementations/Contract1.sol"),
            "Should keep first mapping of MyContract.sol"
        );
        assert!(
            !result
                .iter()
                .any(|r| r.name == "MyContract.sol" && r.path == "implementations/Contract2.sol"),
            "Should keep first mapping of MyContract.sol"
        );
        assert!(result.iter().any(|r| r.name == "OtherContract.sol" && r.path == "implementations/Contract1.sol"),
            "Should allow different source to same target");

        // Verify the rejected remapping doesn't exist
        assert!(
            !result
                .iter()
                .any(|r| r.name == "MyContract.sol" && r.path == "implementations/Contract2.sol"),
            "Should reject same source to different target"
        );
    }

    #[test]
    fn test_mixed_remappings() {
        let mut remappings = Remappings::new();

        remappings.push(Remapping {
            context: None,
            name: "@openzeppelin-contracts/".to_string(),
            path: "lib/openzeppelin-contracts/".to_string(),
        });
        remappings.push(Remapping {
            context: None,
            name: "@openzeppelin/contracts/".to_string(),
            path: "lib/openzeppelin/contracts/".to_string(),
        });

        remappings.push(Remapping {
            context: None,
            name: "MyContract.sol".to_string(),
            path: "os/Contract.sol".to_string(),
        });

        let result = remappings.into_inner();
        assert_eq!(result.len(), 3, "Should have 3 remappings");
        assert_eq!(result.first().unwrap().name, "@openzeppelin-contracts/");
        assert_eq!(result.first().unwrap().path, "lib/openzeppelin-contracts/");
        assert_eq!(result.get(1).unwrap().name, "@openzeppelin/contracts/");
        assert_eq!(result.get(1).unwrap().path, "lib/openzeppelin/contracts/");
        assert_eq!(result.get(2).unwrap().name, "MyContract.sol");
        assert_eq!(result.get(2).unwrap().path, "os/Contract.sol");
    }

    #[test]
    fn test_remappings_with_context() {
        let mut remappings = Remappings::new();

        // Same name but different contexts
        remappings.push(Remapping {
            context: Some("test/".to_string()),
            name: "MyContract.sol".to_string(),
            path: "test/Contract.sol".to_string(),
        });
        remappings.push(Remapping {
            context: Some("prod/".to_string()),
            name: "MyContract.sol".to_string(),
            path: "prod/Contract.sol".to_string(),
        });

        let result = remappings.into_inner();
        assert_eq!(result.len(), 2, "Should allow same name with different contexts");
        assert!(
            result
                .iter()
                .any(|r| r.context == Some("test/".to_string()) && r.path == "test/Contract.sol")
        );
        assert!(
            result
                .iter()
                .any(|r| r.context == Some("prod/".to_string()) && r.path == "prod/Contract.sol")
        );
    }

    #[test]
    fn root_remapping_overrides_contextual_single_file_remapping() {
        let mut remappings = Remappings::new_with_remappings(vec![Remapping {
            context: None,
            name: "Alias.sol".to_string(),
            path: "src/Root.sol".to_string(),
        }]);
        remappings.extend(vec![Remapping {
            context: Some("lib/dep".to_string()),
            name: "Alias.sol".to_string(),
            path: "lib/dep/src/Dependency.sol".to_string(),
        }]);

        assert_eq!(remappings.into_inner().len(), 1);
    }

    #[test]
    fn dependency_remapping_context_cannot_escape() {
        let temp = tempdir().unwrap();
        let lib = temp.path().join("lib");
        let outside = temp.path().join("outside");
        fs::create_dir_all(lib.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let lib = dunce::canonicalize(lib).unwrap();
        let outside = dunce::canonicalize(outside).unwrap();
        let src = lib.join("src");

        let remapping_with_context = |context: &Path| Remapping {
            context: Some(context.to_string_lossy().into_owned()),
            name: "dep/".to_string(),
            path: "lib/dep/".to_string(),
        };

        let contextual = RemappingsProvider::with_dependency_context(
            remapping_with_context(Path::new("src")),
            &lib,
        )
        .unwrap();
        let expected_context = format!("{}/", src.display());
        assert_eq!(contextual.context.as_deref(), Some(expected_context.as_str()));
        assert!(
            RemappingsProvider::with_dependency_context(
                remapping_with_context(Path::new("../outside")),
                &lib,
            )
            .is_none()
        );
        assert!(
            RemappingsProvider::with_dependency_context(remapping_with_context(&outside), &lib)
                .is_none()
        );

        #[cfg(unix)]
        {
            symlink(&outside, lib.join("link")).unwrap();
            assert!(
                RemappingsProvider::with_dependency_context(
                    remapping_with_context(Path::new("link")),
                    &lib,
                )
                .is_none()
            );
            assert!(
                RemappingsProvider::with_dependency_context(
                    remapping_with_context(Path::new("link/missing")),
                    &lib,
                )
                .is_none()
            );
        }
    }
}
