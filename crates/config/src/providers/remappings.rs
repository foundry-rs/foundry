use crate::{Config, foundry_toml_dirs, remappings_from_env_var, remappings_from_newline};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    value::{Dict, Map},
};
use foundry_compilers::artifacts::remappings::{RelativeRemapping, Remapping};
use rayon::prelude::*;
use std::{
    borrow::Cow,
    cmp::Reverse,
    collections::{BTreeMap, HashSet, btree_map::Entry},
    fs,
    path::{MAIN_SEPARATOR, Path, PathBuf},
};

const GENERATED_REMAPPINGS_KEY: &str = "__generated_remappings";

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
    fn push(&mut self, remapping: Remapping) -> bool {
        // Special handling for .sol file remappings, only allow one remapping per source file.
        if remapping.name.ends_with(".sol") && !remapping.path.ends_with(".sol") {
            return false;
        }

        if self.remappings.iter().any(|existing| {
            if remapping.name.ends_with(".sol") {
                // For .sol files, only prevent duplicate source names in the same context
                return existing.name == remapping.name
                    && existing.context == remapping.context
                    && existing.path == remapping.path;
            }

            // Autodetected remappings are added from the root project down through its libraries,
            // so an existing root alias remains authoritative over an equal or more specific
            // dependency alias. For example, an existing `@utils/=src/` suppresses an incoming
            // `@utils/libraries/=lib/utils/`, preventing a dependency from overriding part of the
            // root namespace. The reverse direction is intentional: an existing
            // `@prb/math/=src/math/` can coexist with an incoming `@prb/=lib/prb/`; the root alias
            // resolves its subtree while the dependency alias acts as a fallback for the rest of
            // the namespace.
            let mut existing_name_path = existing.name.clone();
            if !existing_name_path.ends_with('/') {
                existing_name_path.push('/')
            }
            let is_conflicting = remapping.name.starts_with(&existing_name_path);
            is_conflicting && existing.context == remapping.context
        }) {
            return false;
        };

        // Ignore remappings of root project src, test or script dir.
        // See <https://github.com/foundry-rs/foundry/issues/3440>.
        if self
            .project_paths
            .iter()
            .any(|project_path| remapping.name.eq_ignore_ascii_case(&project_path.name))
        {
            return false;
        };

        self.remappings.push(remapping);
        true
    }

    /// Extend the remappings vector, leaving out the remappings that are already present.
    pub fn extend(&mut self, remappings: Vec<Remapping>) {
        for remapping in remappings {
            self.push(remapping);
        }
    }

    /// Extract generated contextual refinements from a Figment.
    pub fn generated_from_figment(figment: &Figment) -> Vec<Remapping> {
        figment.extract_inner(GENERATED_REMAPPINGS_KEY).unwrap_or_default()
    }

    /// Extend with lower-precedence remappings, preserving global alias precedence over generated
    /// contextual refinements without discarding their broader fallbacks.
    pub fn extend_with_lower_precedence(
        &mut self,
        remappings: Vec<Remapping>,
        generated: &[Remapping],
    ) {
        let authoritative = self
            .remappings
            .iter()
            .filter(|remapping| remapping.context.is_none())
            .cloned()
            .collect::<Vec<_>>();
        let mut suppressed = HashSet::new();
        let mut overlays = Vec::new();
        for (index, remapping) in remappings
            .iter()
            .enumerate()
            .filter(|(_, remapping)| remapping.context.is_some() && generated.contains(remapping))
        {
            if let Some(contextual) = contextual_overlays(&authoritative, remapping) {
                overlays.extend(contextual);
            } else {
                suppressed.insert(index);
            }
        }
        self.extend(overlays);
        for (index, remapping) in remappings.into_iter().enumerate() {
            if !suppressed.contains(&index) {
                self.push(remapping);
            }
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
    fn get_remappings(
        &self,
        remappings: Vec<Remapping>,
    ) -> Result<(Vec<Remapping>, Vec<Remapping>), Error> {
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
        if let Some(env_remappings) = remappings_from_env_var("DAPP_REMAPPINGS")
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
        let global_user_remappings =
            user_remappings.iter().filter(|r| r.context.is_none()).cloned().collect::<Vec<_>>();
        // Let's now use the wrapper to conditionally extend the remappings with the autodetected
        // ones. We want to avoid duplicates, and the wrapper will handle this for us.
        let mut all_remappings = Remappings::new_with_remappings(user_remappings);

        // scan all library dirs and autodetect remappings
        // TODO: if a lib specifies contexts for remappings manually, we need to figure out how to
        // resolve that
        if self.auto_detect_remappings {
            let (nested_foundry_remappings, auto_detected_remappings) = rayon::join(
                || self.find_nested_foundry_remappings(),
                || self.auto_detect_remappings(),
            );
            let auto_detected_remappings = auto_detected_remappings.collect::<Vec<_>>();

            let mut lib_remappings = BTreeMap::new();
            let mut contextual_remappings = Vec::new();
            for (lib, r) in nested_foundry_remappings {
                // A dependency can intentionally refine an auto-detected package root to its
                // source directory. Scope that refinement to the dependency so root imports keep
                // the broader package mapping.
                if r.context.is_none()
                    && auto_detected_remappings.iter().any(|auto| {
                        auto.context == r.context
                            && auto.name == r.name
                            && auto.path != r.path
                            && Path::new(&r.path).starts_with(&auto.path)
                    })
                {
                    let mut contextual = r.clone();
                    contextual.context = Some(format!("{}/", lib.display()));
                    if let Some(overlays) =
                        contextual_overlays(&global_user_remappings, &contextual)
                    {
                        contextual_remappings.extend(overlays);
                        contextual_remappings.push(contextual);
                    }
                }
                insert_closest(&mut lib_remappings, r.context, r.name, r.path.into());
            }
            for r in auto_detected_remappings {
                // this is an additional safety check for weird auto-detected remappings
                if ["lib/", "src/", "contracts/"].contains(&r.name.as_str()) {
                    trace!(target: "forge", "- skipping the remapping");
                    continue;
                }
                insert_closest(&mut lib_remappings, r.context, r.name, r.path.into());
            }

            let explicit_remappings = all_remappings
                .remappings
                .iter()
                .map(|r| relative_remapping_preserving_context_boundary(r.clone(), self.root))
                .collect::<Vec<_>>();
            let mut generated_remappings = Vec::new();
            for contextual in contextual_remappings {
                let relative =
                    relative_remapping_preserving_context_boundary(contextual.clone(), self.root);
                if !explicit_remappings.contains(&relative)
                    && all_remappings.push(contextual.clone())
                {
                    generated_remappings.push(contextual);
                }
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

            return Ok((all_remappings.into_inner(), generated_remappings));
        }

        Ok((all_remappings.into_inner(), Vec::new()))
    }

    /// Returns all remappings declared in foundry.toml files of libraries
    fn find_nested_foundry_remappings(&self) -> impl Iterator<Item = (PathBuf, Remapping)> + '_ {
        let mut groups = self
            .lib_paths
            .par_iter()
            .map(|p| if p.is_absolute() { self.root.join("lib") } else { self.root.join(p) })
            .flat_map(foundry_toml_dirs)
            .map(|lib| {
                trace!(?lib, "find all remappings of nested foundry.toml");
                let remappings = self.nested_foundry_remappings(&lib);
                (lib, remappings)
            })
            .collect::<Vec<_>>();
        groups.sort_by(|(a, _), (b, _)| a.cmp(b));
        groups
            .into_iter()
            .flat_map(|(lib, remappings)| remappings.into_iter().map(move |r| (lib.clone(), r)))
    }

    fn nested_foundry_remappings(&self, lib: &Path) -> Vec<Remapping> {
        // load config of the nested lib if it exists, using fallback mode since libs may not
        // define all profiles the main project uses
        let Ok(config) = Config::load_with_root_and_fallback(lib) else { return vec![] };
        let config = config.sanitized();

        // if the configured _src_ directory is set to something that
        // `Remapping::find_many` doesn't classify as a src directory (src, contracts,
        // lib), then we need to manually add a remapping here
        let src_remapping = if ![Path::new("src"), Path::new("contracts"), Path::new("lib")]
            .contains(&config.src.as_path())
            && let Some(name) = lib.file_name().and_then(|s| s.to_str())
        {
            let mut r = Remapping {
                context: None,
                name: format!("{name}/"),
                path: format!("{}", lib.join(&config.src).display()),
            };
            if !r.path.ends_with('/') {
                r.path.push('/')
            }
            Some(r)
        } else {
            None
        };

        // Eventually, we could set context for remappings at this location,
        // taking into account the OS platform. We'll need to be able to handle nested
        // contexts depending on dependencies for this to work.
        // For now, we just leave the default context (none).
        let mut remappings =
            config.remappings.into_iter().map(Remapping::from).collect::<Vec<Remapping>>();

        if let Some(r) = src_remapping {
            remappings.push(r);
        }
        remappings
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

fn remapping_name_is_prefix(prefix: &str, name: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    let name = name.trim_end_matches('/');
    prefix == name || name.strip_prefix(prefix).is_some_and(|suffix| suffix.starts_with('/'))
}

fn contextual_overlays(
    authoritative: &[Remapping],
    refinement: &Remapping,
) -> Option<Vec<Remapping>> {
    if authoritative.iter().any(|mapping| remapping_name_is_prefix(&mapping.name, &refinement.name))
    {
        return None;
    }
    let mut overlays = authoritative
        .iter()
        .filter(|mapping| remapping_name_is_prefix(&refinement.name, &mapping.name))
        .cloned()
        .collect::<Vec<_>>();
    overlays.sort_by_key(|mapping| Reverse(mapping.name.len()));
    for overlay in &mut overlays {
        overlay.context.clone_from(&refinement.context);
    }
    Some(overlays)
}

pub fn relative_remapping_preserving_context_boundary(
    remapping: Remapping,
    root: &Path,
) -> RelativeRemapping {
    let has_boundary =
        remapping.context.as_deref().is_some_and(|context| context.ends_with(['/', '\\']));
    let mut remapping = RelativeRemapping::new(remapping, root);
    if has_boundary
        && let Some(context) = &mut remapping.context
        && !context.ends_with(['/', '\\'])
    {
        context.push(MAIN_SEPARATOR);
    }
    remapping
}

impl Provider for RemappingsProvider<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Remapping Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let (remappings, generated_remappings) = match &self.remappings {
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
            .map(|r| relative_remapping_preserving_context_boundary(r, self.root).to_string())
            .collect::<Vec<_>>();
        let generated_remappings = generated_remappings
            .into_iter()
            .map(|r| relative_remapping_preserving_context_boundary(r, self.root).to_string())
            .collect::<Vec<_>>();

        Ok(Map::from([(
            Config::selected_profile(),
            Dict::from([
                ("remappings".to_string(), figment::value::Value::from(remappings)),
                (
                    GENERATED_REMAPPINGS_KEY.to_string(),
                    figment::value::Value::from(generated_remappings),
                ),
            ]),
        )]))
    }

    fn profile(&self) -> Option<Profile> {
        Some(Config::selected_profile())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_remapping_preserves_context_directory_boundary() {
        let remapping = Remapping {
            context: Some(format!("lib{MAIN_SEPARATOR}outer{MAIN_SEPARATOR}")),
            name: "inner/".to_string(),
            path: format!("lib{MAIN_SEPARATOR}outer{MAIN_SEPARATOR}lib{MAIN_SEPARATOR}inner"),
        };

        let remapping = relative_remapping_preserving_context_boundary(remapping, Path::new("."));
        assert!(remapping.context.unwrap().ends_with(MAIN_SEPARATOR));
    }

    #[test]
    fn lower_precedence_merge_only_suppresses_generated_refinements() {
        let cli = Remapping {
            context: None,
            name: "pkg/sub/".to_string(),
            path: "src/local/".to_string(),
        };
        let contextual = Remapping {
            context: Some("lib/dep/".to_string()),
            name: "pkg/".to_string(),
            path: "lib/dep/vendor/pkg/".to_string(),
        };
        let mut explicit = Remappings::new_with_remappings(vec![cli.clone()]);
        explicit.extend_with_lower_precedence(vec![contextual.clone()], &[]);
        assert_eq!(explicit.into_inner(), vec![cli.clone(), contextual.clone()]);

        let global = Remapping {
            context: None,
            name: "pkg/".to_string(),
            path: "lib/dep/lib/pkg/".to_string(),
        };
        let refinement = Remapping {
            context: Some("lib/dep/".to_string()),
            name: "pkg/".to_string(),
            path: "lib/dep/lib/pkg/contracts/".to_string(),
        };
        let cli_deep = Remapping {
            context: None,
            name: "pkg/sub/deep/".to_string(),
            path: "src/deep/".to_string(),
        };
        let mut generated = Remappings::new_with_remappings(vec![cli.clone(), cli_deep.clone()]);
        generated.extend_with_lower_precedence(
            vec![refinement.clone(), global.clone()],
            std::slice::from_ref(&refinement),
        );
        let mut overlay = cli.clone();
        overlay.context.clone_from(&refinement.context);
        let mut deep_overlay = cli_deep.clone();
        deep_overlay.context.clone_from(&refinement.context);
        assert_eq!(
            generated.into_inner(),
            vec![cli.clone(), cli_deep, deep_overlay, overlay, refinement, global.clone()]
        );

        let broad_cli =
            Remapping { context: None, name: "pkg/".to_string(), path: "src/local/".to_string() };
        let mut generated = Remappings::new_with_remappings(vec![broad_cli.clone(), cli.clone()]);
        generated.extend_with_lower_precedence(
            vec![contextual.clone(), global],
            std::slice::from_ref(&contextual),
        );
        assert_eq!(generated.into_inner(), vec![broad_cli, cli]);
    }

    #[test]
    fn nested_remapping_groups_are_sorted() {
        let root = tempfile::tempdir().unwrap();
        for dependency in ["zeta", "alpha"] {
            let dependency = root.path().join("lib").join(dependency);
            fs::create_dir_all(&dependency).unwrap();
            fs::write(dependency.join(Config::FILE_NAME), "[profile.default]\n").unwrap();
            fs::write(dependency.join("remappings.txt"), "pkg/=src/\n").unwrap();
        }
        let libs = vec![PathBuf::from("lib")];
        let provider = RemappingsProvider {
            auto_detect_remappings: true,
            lib_paths: Cow::Borrowed(&libs),
            root: root.path(),
            remappings: Ok(Vec::new()),
        };

        let dependencies = provider
            .find_nested_foundry_remappings()
            .map(|(dependency, _)| dependency.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(dependencies, ["alpha", "alpha", "zeta", "zeta"]);
    }

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
    fn test_root_remapping_prefix_precedence_is_directional() {
        let remapping = |name: &str, path: &str| Remapping {
            context: None,
            name: name.to_string(),
            path: path.to_string(),
        };

        let mut narrow_root =
            Remappings::new_with_remappings(vec![remapping("pkg/sub/", "src/local/")]);
        narrow_root.extend(vec![remapping("pkg/", "lib/pkg/src/")]);
        assert_eq!(
            narrow_root.into_inner(),
            vec![remapping("pkg/sub/", "src/local/"), remapping("pkg/", "lib/pkg/src/")]
        );

        let mut broad_root = Remappings::new_with_remappings(vec![remapping("pkg/", "src/local/")]);
        broad_root.extend(vec![
            remapping("pkg/sub/", "lib/pkg/src/sub/"),
            remapping("pkg-other/", "lib/pkg-other/src/"),
        ]);
        assert_eq!(
            broad_root.into_inner(),
            vec![remapping("pkg/", "src/local/"), remapping("pkg-other/", "lib/pkg-other/src/")]
        );

        let mut duplicate = Remappings::new_with_remappings(vec![remapping("pkg/", "src/local/")]);
        duplicate.extend(vec![remapping("pkg/", "lib/pkg/src/")]);
        assert_eq!(duplicate.remappings, vec![remapping("pkg/", "src/local/")]);

        let contextual_remapping = |context: &str, name: &str, path: &str| Remapping {
            context: Some(context.to_string()),
            name: name.to_string(),
            path: path.to_string(),
        };
        let mut same_context = Remappings::new_with_remappings(vec![contextual_remapping(
            "src/",
            "pkg/",
            "src/local/",
        )]);
        same_context.extend(vec![contextual_remapping("src/", "pkg/sub/", "lib/pkg/src/sub/")]);
        assert_eq!(
            same_context.remappings,
            vec![contextual_remapping("src/", "pkg/", "src/local/")]
        );

        let mut different_context = Remappings::new_with_remappings(vec![contextual_remapping(
            "src/",
            "pkg/",
            "src/local/",
        )]);
        different_context.extend(vec![contextual_remapping(
            "test/",
            "pkg/sub/",
            "lib/pkg/src/sub/",
        )]);
        assert_eq!(
            different_context.remappings,
            vec![
                contextual_remapping("src/", "pkg/", "src/local/"),
                contextual_remapping("test/", "pkg/sub/", "lib/pkg/src/sub/"),
            ]
        );

        let mut narrow_root_without_slash =
            Remappings::new_with_remappings(vec![remapping("pkg/sub", "src/local/")]);
        narrow_root_without_slash.extend(vec![remapping("pkg", "lib/pkg/src/")]);
        assert_eq!(
            narrow_root_without_slash.remappings,
            vec![remapping("pkg/sub", "src/local/"), remapping("pkg", "lib/pkg/src/")]
        );

        let mut broad_root_without_slash =
            Remappings::new_with_remappings(vec![remapping("pkg", "src/local/")]);
        broad_root_without_slash.extend(vec![
            remapping("pkg/sub", "lib/pkg/src/sub/"),
            remapping("pkg-other", "lib/pkg-other/src/"),
        ]);
        assert_eq!(
            broad_root_without_slash.remappings,
            vec![remapping("pkg", "src/local/"), remapping("pkg-other", "lib/pkg-other/src/")]
        );
    }
}
