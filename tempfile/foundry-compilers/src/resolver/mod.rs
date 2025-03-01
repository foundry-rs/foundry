//! Resolution of the entire dependency graph for a project.
//!
//! This module implements the core logic in taking all contracts of a project and creating a
//! resolved graph with applied remappings for all source contracts.
//!
//! Some constraints we're working with when resolving contracts
//!
//!   1. Each file can contain several source units and can have any number of imports/dependencies
//!      (using the term interchangeably). Each dependency can declare a version range that it is
//!      compatible with, solidity version pragma.
//!   2. A dependency can be imported from any directory, see `Remappings`
//!
//! Finding all dependencies is fairly simple, we're simply doing a DFS, starting from the source
//! contracts
//!
//! ## Solc version auto-detection
//!
//! Solving a constraint graph is an NP-hard problem. The algorithm for finding the "best" solution
//! makes several assumptions and tries to find a version of "Solc" that is compatible with all
//! source files.
//!
//! The algorithm employed here is fairly simple, we simply do a DFS over all the source files and
//! find the set of Solc versions that the file and all its imports are compatible with, and then we
//! try to find a single Solc version that is compatible with all the files. This is effectively the
//! intersection of all version sets.
//!
//! We always try to activate the highest (installed) solc version first. Uninstalled solc is only
//! used if this version is the only compatible version for a single file or in the intersection of
//! all version sets.
//!
//! This leads to finding the optimal version, if there is one. If there is no single Solc version
//! that is compatible with all sources and their imports, then suddenly this becomes a very
//! difficult problem, because what would be the "best" solution. In this case, just choose the
//! latest (installed) Solc version and try to minimize the number of Solc versions used.
//!
//! ## Performance
//!
//! Note that this is a relatively performance-critical portion of the ethers-solc preprocessing.
//! The data that needs to be processed is proportional to the size of the dependency
//! graph, which can, depending on the project, often be quite large.
//!
//! Note that, unlike the solidity compiler, we work with the filesystem, where we have to resolve
//! remappings and follow relative paths. We're also limiting the nodes in the graph to solidity
//! files, since we're only interested in their
//! [version pragma](https://docs.soliditylang.org/en/develop/layout-of-source-files.html#version-pragma),
//! which is defined on a per source file basis.

use crate::{
    compilers::{Compiler, CompilerVersion, Language, ParsedSource},
    project::VersionedSources,
    ArtifactOutput, CompilerSettings, Project, ProjectPathsConfig,
};
use core::fmt;
use foundry_compilers_artifacts::sources::{Source, Sources};
use foundry_compilers_core::{
    error::{Result, SolcError},
    utils::{self, find_case_sensitive_existing_file},
};
use parse::SolData;
use rayon::prelude::*;
use semver::{Version, VersionReq};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
};
use yansi::{Color, Paint};

pub mod parse;
mod tree;

pub use parse::SolImportAlias;
pub use tree::{print, Charset, TreeOptions};

/// Container for result of version and profile resolution of sources contained in [`Graph`].
#[derive(Debug)]
pub struct ResolvedSources<'a, C: Compiler> {
    /// Resolved set of sources.
    ///
    /// Mapping from language to a [`Vec`] of compiler inputs consisting of version, sources set
    /// and settings.
    pub sources: VersionedSources<'a, C::Language, C::Settings>,
    /// A mapping from a source file path to the primary profile name selected for it.
    ///
    /// This is required because the same source file might be compiled with multiple different
    /// profiles if it's present as a dependency for other sources. We want to keep a single name
    /// of the profile which was chosen specifically for each source so that we can default to it.
    /// Right now, this is used when generating artifact names, "primary" artifact will never have
    /// a profile suffix.
    pub primary_profiles: HashMap<PathBuf, &'a str>,
    /// Graph edges.
    pub edges: GraphEdges<C::ParsedSource>,
}

/// The underlying edges of the graph which only contains the raw relationship data.
///
/// This is kept separate from the `Graph` as the `Node`s get consumed when the `Solc` to `Sources`
/// set is determined.
#[derive(Debug)]
pub struct GraphEdges<D> {
    /// The indices of `edges` correspond to the `nodes`. That is, `edges[0]`
    /// is the set of outgoing edges for `nodes[0]`.
    edges: Vec<Vec<usize>>,
    /// Reverse of `edges`. That is, `rev_edges[0]` is the set of incoming edges for `nodes[0]`.
    rev_edges: Vec<Vec<usize>>,
    /// index maps for a solidity file to an index, for fast lookup.
    indices: HashMap<PathBuf, usize>,
    /// reverse of `indices` for reverse lookup
    rev_indices: HashMap<usize, PathBuf>,
    /// the identified version requirement of a file
    versions: HashMap<usize, Option<VersionReq>>,
    /// the extracted data from the source file
    data: HashMap<usize, D>,
    /// with how many input files we started with, corresponds to `let input_files =
    /// nodes[..num_input_files]`.
    ///
    /// Combined with the `indices` this way we can determine if a file was original added to the
    /// graph as input or was added as resolved import, see [`Self::is_input_file()`]
    num_input_files: usize,
    /// tracks all imports that we failed to resolve for a file
    unresolved_imports: HashSet<(PathBuf, PathBuf)>,
    /// tracks additional include paths resolved by scanning all imports of the graph
    ///
    /// Absolute imports, like `import "src/Contract.sol"` are possible, but this does not play
    /// nice with the standard-json import format, since the VFS won't be able to resolve
    /// "src/Contract.sol" without help via `--include-path`
    resolved_solc_include_paths: BTreeSet<PathBuf>,
}

impl<D> GraphEdges<D> {
    /// How many files are source files
    pub fn num_source_files(&self) -> usize {
        self.num_input_files
    }

    /// Returns an iterator over all file indices
    pub fn files(&self) -> impl Iterator<Item = usize> + '_ {
        0..self.edges.len()
    }

    /// Returns an iterator over all source file indices
    pub fn source_files(&self) -> impl Iterator<Item = usize> + '_ {
        0..self.num_input_files
    }

    /// Returns an iterator over all library files
    pub fn library_files(&self) -> impl Iterator<Item = usize> + '_ {
        self.files().skip(self.num_input_files)
    }

    /// Returns all additional `--include-paths`
    pub fn include_paths(&self) -> &BTreeSet<PathBuf> {
        &self.resolved_solc_include_paths
    }

    /// Returns all imports that we failed to resolve
    pub fn unresolved_imports(&self) -> &HashSet<(PathBuf, PathBuf)> {
        &self.unresolved_imports
    }

    /// Returns a list of nodes the given node index points to for the given kind.
    pub fn imported_nodes(&self, from: usize) -> &[usize] {
        &self.edges[from]
    }

    /// Returns an iterator that yields all imports of a node and all their imports
    pub fn all_imported_nodes(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        NodesIter::new(from, self).skip(1)
    }

    /// Returns all files imported by the given file
    pub fn imports(&self, file: &Path) -> HashSet<&PathBuf> {
        if let Some(start) = self.indices.get(file).copied() {
            NodesIter::new(start, self).skip(1).map(move |idx| &self.rev_indices[&idx]).collect()
        } else {
            HashSet::new()
        }
    }

    /// Returns all files that import the given file
    pub fn importers(&self, file: &Path) -> HashSet<&PathBuf> {
        if let Some(start) = self.indices.get(file).copied() {
            self.rev_edges[start].iter().map(move |idx| &self.rev_indices[idx]).collect()
        } else {
            HashSet::new()
        }
    }

    /// Returns the id of the given file
    pub fn node_id(&self, file: &Path) -> usize {
        self.indices[file]
    }

    /// Returns the path of the given node
    pub fn node_path(&self, id: usize) -> &PathBuf {
        &self.rev_indices[&id]
    }

    /// Returns true if the `file` was originally included when the graph was first created and not
    /// added when all `imports` were resolved
    pub fn is_input_file(&self, file: &Path) -> bool {
        if let Some(idx) = self.indices.get(file).copied() {
            idx < self.num_input_files
        } else {
            false
        }
    }

    /// Returns the `VersionReq` for the given file
    pub fn version_requirement(&self, file: &Path) -> Option<&VersionReq> {
        self.indices.get(file).and_then(|idx| self.versions.get(idx)).and_then(Option::as_ref)
    }

    /// Returns the parsed source data for the given file
    pub fn get_parsed_source(&self, file: &Path) -> Option<&D> {
        self.indices.get(file).and_then(|idx| self.data.get(idx))
    }
}

/// Represents a fully-resolved solidity dependency graph.
///
/// Each node in the graph is a file and edges represent dependencies between them.
///
/// See also <https://docs.soliditylang.org/en/latest/layout-of-source-files.html?highlight=import#importing-other-source-files>
#[derive(Debug)]
pub struct Graph<D = SolData> {
    /// all nodes in the project, a `Node` represents a single file
    pub nodes: Vec<Node<D>>,
    /// relationship of the nodes
    edges: GraphEdges<D>,
    /// the root of the project this graph represents
    root: PathBuf,
}

impl<L: Language, D: ParsedSource<Language = L>> Graph<D> {
    /// Print the graph to `StdOut`
    pub fn print(&self) {
        self.print_with_options(Default::default())
    }

    /// Print the graph to `StdOut` using the provided `TreeOptions`
    pub fn print_with_options(&self, opts: TreeOptions) {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        tree::print(self, &opts, &mut out).expect("failed to write to stdout.")
    }

    /// Returns a list of nodes the given node index points to for the given kind.
    pub fn imported_nodes(&self, from: usize) -> &[usize] {
        self.edges.imported_nodes(from)
    }

    /// Returns an iterator that yields all imports of a node and all their imports
    pub fn all_imported_nodes(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        self.edges.all_imported_nodes(from)
    }

    /// Returns `true` if the given node has any outgoing edges.
    pub(crate) fn has_outgoing_edges(&self, index: usize) -> bool {
        !self.edges.edges[index].is_empty()
    }

    /// Returns all the resolved files and their index in the graph.
    pub fn files(&self) -> &HashMap<PathBuf, usize> {
        &self.edges.indices
    }

    /// Returns `true` if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Gets a node by index.
    ///
    /// # Panics
    ///
    /// if the `index` node id is not included in the graph
    pub fn node(&self, index: usize) -> &Node<D> {
        &self.nodes[index]
    }

    pub(crate) fn display_node(&self, index: usize) -> DisplayNode<'_, D> {
        DisplayNode { node: self.node(index), root: &self.root }
    }

    /// Returns an iterator that yields all nodes of the dependency tree that the given node id
    /// spans, starting with the node itself.
    ///
    /// # Panics
    ///
    /// if the `start` node id is not included in the graph
    pub fn node_ids(&self, start: usize) -> impl Iterator<Item = usize> + '_ {
        NodesIter::new(start, &self.edges)
    }

    /// Same as `Self::node_ids` but returns the actual `Node`
    pub fn nodes(&self, start: usize) -> impl Iterator<Item = &Node<D>> + '_ {
        self.node_ids(start).map(move |idx| self.node(idx))
    }

    fn split(self) -> (Vec<(PathBuf, Source)>, GraphEdges<D>) {
        let Self { nodes, mut edges, .. } = self;
        // need to move the extracted data to the edges, essentially splitting the node so we have
        // access to the data at a later stage in the compile pipeline
        let mut sources = Vec::new();
        for (idx, node) in nodes.into_iter().enumerate() {
            let Node { path, source, data } = node;
            sources.push((path, source));
            edges.data.insert(idx, data);
        }

        (sources, edges)
    }

    /// Consumes the `Graph`, effectively splitting the `nodes` and the `GraphEdges` off and
    /// returning the `nodes` converted to `Sources`
    pub fn into_sources(self) -> (Sources, GraphEdges<D>) {
        let (sources, edges) = self.split();
        (sources.into_iter().collect(), edges)
    }

    /// Returns an iterator that yields only those nodes that represent input files.
    /// See `Self::resolve_sources`
    /// This won't yield any resolved library nodes
    pub fn input_nodes(&self) -> impl Iterator<Item = &Node<D>> {
        self.nodes.iter().take(self.edges.num_input_files)
    }

    /// Returns all files imported by the given file
    pub fn imports(&self, path: &Path) -> HashSet<&PathBuf> {
        self.edges.imports(path)
    }

    /// Resolves a number of sources within the given config
    pub fn resolve_sources(
        paths: &ProjectPathsConfig<D::Language>,
        sources: Sources,
    ) -> Result<Self> {
        /// checks if the given target path was already resolved, if so it adds its id to the list
        /// of resolved imports. If it hasn't been resolved yet, it queues in the file for
        /// processing
        fn add_node<D: ParsedSource>(
            unresolved: &mut VecDeque<(PathBuf, Node<D>)>,
            index: &mut HashMap<PathBuf, usize>,
            resolved_imports: &mut Vec<usize>,
            target: PathBuf,
        ) -> Result<()> {
            if let Some(idx) = index.get(&target).copied() {
                resolved_imports.push(idx);
            } else {
                // imported file is not part of the input files
                let node = Node::read(&target)?;
                unresolved.push_back((target.clone(), node));
                let idx = index.len();
                index.insert(target, idx);
                resolved_imports.push(idx);
            }
            Ok(())
        }

        // we start off by reading all input files, which includes all solidity files from the
        // source and test folder
        let mut unresolved: VecDeque<_> = sources
            .0
            .into_par_iter()
            .map(|(path, source)| {
                let data = D::parse(source.as_ref(), &path)?;
                Ok((path.clone(), Node { path, source, data }))
            })
            .collect::<Result<_>>()?;

        // identifiers of all resolved files
        let mut index: HashMap<_, _> =
            unresolved.iter().enumerate().map(|(idx, (p, _))| (p.clone(), idx)).collect();

        let num_input_files = unresolved.len();

        // contains the files and their dependencies
        let mut nodes = Vec::with_capacity(unresolved.len());
        let mut edges = Vec::with_capacity(unresolved.len());
        let mut rev_edges = Vec::with_capacity(unresolved.len());

        // tracks additional paths that should be used with `--include-path`, these are libraries
        // that use absolute imports like `import "src/Contract.sol"`
        let mut resolved_solc_include_paths = BTreeSet::new();
        resolved_solc_include_paths.insert(paths.root.clone());

        // keep track of all unique paths that we failed to resolve to not spam the reporter with
        // the same path
        let mut unresolved_imports = HashSet::new();

        // now we need to resolve all imports for the source file and those imported from other
        // locations
        while let Some((path, node)) = unresolved.pop_front() {
            let mut resolved_imports = Vec::new();
            // parent directory of the current file
            let cwd = match path.parent() {
                Some(inner) => inner,
                None => continue,
            };

            for import_path in node.data.resolve_imports(paths, &mut resolved_solc_include_paths)? {
                match paths.resolve_import_and_include_paths(
                    cwd,
                    &import_path,
                    &mut resolved_solc_include_paths,
                ) {
                    Ok(import) => {
                        add_node(&mut unresolved, &mut index, &mut resolved_imports, import)
                            .map_err(|err| {
                                match err {
                                    SolcError::ResolveCaseSensitiveFileName { .. }
                                    | SolcError::Resolve(_) => {
                                        // make the error more helpful by providing additional
                                        // context
                                        SolcError::FailedResolveImport(
                                            Box::new(err),
                                            node.path.clone(),
                                            import_path.clone(),
                                        )
                                    }
                                    _ => err,
                                }
                            })?
                    }
                    Err(err) => {
                        unresolved_imports.insert((import_path.to_path_buf(), node.path.clone()));
                        trace!(
                            "failed to resolve import component \"{:?}\" for {:?}",
                            err,
                            node.path
                        )
                    }
                };
            }

            nodes.push(node);
            edges.push(resolved_imports);
            // Will be populated later
            rev_edges.push(Vec::new());
        }

        // Build `rev_edges`
        for (idx, edges) in edges.iter().enumerate() {
            for &edge in edges.iter() {
                rev_edges[edge].push(idx);
            }
        }

        if !unresolved_imports.is_empty() {
            // notify on all unresolved imports
            crate::report::unresolved_imports(
                &unresolved_imports
                    .iter()
                    .map(|(i, f)| (i.as_path(), f.as_path()))
                    .collect::<Vec<_>>(),
                &paths.remappings,
            );
        }

        let edges = GraphEdges {
            edges,
            rev_edges,
            rev_indices: index.iter().map(|(k, v)| (*v, k.clone())).collect(),
            indices: index,
            num_input_files,
            versions: nodes
                .iter()
                .enumerate()
                .map(|(idx, node)| (idx, node.data.version_req().cloned()))
                .collect(),
            data: Default::default(),
            unresolved_imports,
            resolved_solc_include_paths,
        };
        Ok(Self { nodes, edges, root: paths.root.clone() })
    }

    /// Resolves the dependencies of a project's source contracts
    pub fn resolve(paths: &ProjectPathsConfig<D::Language>) -> Result<Self> {
        Self::resolve_sources(paths, paths.read_input_files()?)
    }
}

impl<L: Language, D: ParsedSource<Language = L>> Graph<D> {
    /// Consumes the nodes of the graph and returns all input files together with their appropriate
    /// version and the edges of the graph
    ///
    /// First we determine the compatible version for each input file (from sources and test folder,
    /// see `Self::resolve`) and then we add all resolved library imports.
    pub fn into_sources_by_version<C, T>(
        self,
        project: &Project<C, T>,
    ) -> Result<ResolvedSources<'_, C>>
    where
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
        C: Compiler<ParsedSource = D, Language = L>,
    {
        /// insert the imports of the given node into the sources map
        /// There can be following graph:
        /// `A(<=0.8.10) imports C(>0.4.0)` and `B(0.8.11) imports C(>0.4.0)`
        /// where `C` is a library import, in which case we assign `C` only to the first input file.
        /// However, it's not required to include them in the solc `CompilerInput` as they would get
        /// picked up by solc otherwise, but we add them, so we can create a corresponding
        /// cache entry for them as well. This can be optimized however
        fn insert_imports(
            idx: usize,
            all_nodes: &mut HashMap<usize, (PathBuf, Source)>,
            sources: &mut Sources,
            edges: &[Vec<usize>],
            processed_sources: &mut HashSet<usize>,
        ) {
            // iterate over all dependencies not processed yet
            for dep in edges[idx].iter().copied() {
                // keep track of processed dependencies, if the dep was already in the set we have
                // processed it already
                if !processed_sources.insert(dep) {
                    continue;
                }

                // library import
                if let Some((path, source)) = all_nodes.get(&dep).cloned() {
                    sources.insert(path, source);
                    insert_imports(dep, all_nodes, sources, edges, processed_sources);
                }
            }
        }

        let versioned_nodes = self.get_input_node_versions(project)?;
        let versioned_nodes = self.resolve_settings(project, versioned_nodes)?;
        let (nodes, edges) = self.split();

        let mut all_nodes = nodes.into_iter().enumerate().collect::<HashMap<_, _>>();

        let mut resulted_sources = HashMap::new();
        let mut default_profiles = HashMap::new();

        let profiles = project.settings_profiles().collect::<Vec<_>>();

        // determine the `Sources` set for each solc version
        for (language, versioned_nodes) in versioned_nodes {
            let mut versioned_sources = Vec::with_capacity(versioned_nodes.len());

            for (version, profile_to_nodes) in versioned_nodes {
                for (profile_idx, input_node_indixies) in profile_to_nodes {
                    let mut sources = Sources::new();

                    // all input nodes will be processed
                    let mut processed_sources = input_node_indixies.iter().copied().collect();

                    // we only process input nodes (from sources, tests for example)
                    for idx in input_node_indixies {
                        // insert the input node in the sources set and remove it from the available
                        // set
                        let (path, source) =
                            all_nodes.get(&idx).cloned().expect("node is preset. qed");

                        default_profiles.insert(path.clone(), profiles[profile_idx].0);
                        sources.insert(path, source);
                        insert_imports(
                            idx,
                            &mut all_nodes,
                            &mut sources,
                            &edges.edges,
                            &mut processed_sources,
                        );
                    }
                    versioned_sources.push((version.clone(), sources, profiles[profile_idx]));
                }
            }

            resulted_sources.insert(language, versioned_sources);
        }

        Ok(ResolvedSources { sources: resulted_sources, primary_profiles: default_profiles, edges })
    }

    /// Writes the list of imported files into the given formatter:
    ///
    /// ```text
    /// path/to/a.sol (<version>) imports:
    ///     path/to/b.sol (<version>)
    ///     path/to/c.sol (<version>)
    ///     ...
    /// ```
    fn format_imports_list<
        C: Compiler,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
        W: std::fmt::Write,
    >(
        &self,
        idx: usize,
        incompatible: HashSet<usize>,
        project: &Project<C, T>,
        f: &mut W,
    ) -> std::result::Result<(), std::fmt::Error> {
        let format_node = |idx, f: &mut W| {
            let node = self.node(idx);
            let color = if incompatible.contains(&idx) { Color::Red } else { Color::White };

            let mut line = utils::source_name(&node.path, &self.root).display().to_string();
            if let Some(req) = self.version_requirement(idx, project) {
                line.push_str(&format!(" {req}"));
            }

            write!(f, "{}", line.paint(color))
        };
        format_node(idx, f)?;
        write!(f, " imports:")?;
        for dep in self.node_ids(idx).skip(1) {
            write!(f, "\n    ")?;
            format_node(dep, f)?;
        }

        Ok(())
    }

    /// Combines version requirement parsed from file and from project restrictions.
    fn version_requirement<
        C: Compiler,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        idx: usize,
        project: &Project<C, T>,
    ) -> Option<VersionReq> {
        let node = self.node(idx);
        let parsed_req = node.data.version_req();
        let other_req = project.restrictions.get(&node.path).and_then(|r| r.version.as_ref());

        match (parsed_req, other_req) {
            (Some(parsed_req), Some(other_req)) => {
                let mut req = parsed_req.clone();
                req.comparators.extend(other_req.comparators.clone());
                Some(req)
            }
            (Some(parsed_req), None) => Some(parsed_req.clone()),
            (None, Some(other_req)) => Some(other_req.clone()),
            _ => None,
        }
    }

    /// Checks that the file's version is even available.
    ///
    /// This returns an error if the file's version is invalid semver, or is not available such as
    /// 0.8.20, if the highest available version is `0.8.19`
    fn check_available_version<
        C: Compiler,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        idx: usize,
        all_versions: &[&CompilerVersion],
        project: &Project<C, T>,
    ) -> std::result::Result<(), SourceVersionError> {
        let Some(req) = self.version_requirement(idx, project) else { return Ok(()) };

        if !all_versions.iter().any(|v| req.matches(v.as_ref())) {
            return if project.offline {
                Err(SourceVersionError::NoMatchingVersionOffline(req))
            } else {
                Err(SourceVersionError::NoMatchingVersion(req))
            };
        }

        Ok(())
    }

    /// Filters incompatible versions from the `candidates`. It iterates over node imports and in
    /// case if there is no compatible version it returns the latest seen node id.
    fn retain_compatible_versions<
        C: Compiler,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        idx: usize,
        candidates: &mut Vec<&CompilerVersion>,
        project: &Project<C, T>,
    ) -> Result<(), String> {
        let mut all_versions = candidates.clone();

        let nodes: Vec<_> = self.node_ids(idx).collect();
        let mut failed_node_idx = None;
        for node in nodes.iter() {
            if let Some(req) = self.version_requirement(*node, project) {
                candidates.retain(|v| req.matches(v.as_ref()));

                if candidates.is_empty() {
                    failed_node_idx = Some(*node);
                    break;
                }
            }
        }

        let Some(failed_node_idx) = failed_node_idx else {
            // everything is fine
            return Ok(());
        };

        // This now keeps data for the node which were the last one before we had no candidates
        // left. It means that there is a node directly conflicting with it in `nodes` coming
        // before.
        let failed_node = self.node(failed_node_idx);

        if let Err(version_err) =
            self.check_available_version(failed_node_idx, &all_versions, project)
        {
            // check if the version is even valid
            let f = utils::source_name(&failed_node.path, &self.root).display();
            return Err(format!("Encountered invalid solc version in {f}: {version_err}"));
        } else {
            // if the node requirement makes sense, it means that there is at least one node
            // which requirement conflicts with it

            // retain only versions compatible with the `failed_node`
            if let Some(req) = self.version_requirement(failed_node_idx, project) {
                all_versions.retain(|v| req.matches(v.as_ref()));
            }

            // iterate over all the nodes once again and find the one incompatible
            for node in &nodes {
                if self.check_available_version(*node, &all_versions, project).is_err() {
                    let mut msg = "Found incompatible versions:\n".white().to_string();

                    self.format_imports_list(
                        idx,
                        [*node, failed_node_idx].into(),
                        project,
                        &mut msg,
                    )
                    .unwrap();
                    return Err(msg);
                }
            }
        }

        let mut msg = "Found incompatible versions:\n".white().to_string();
        self.format_imports_list(idx, nodes.into_iter().collect(), project, &mut msg).unwrap();
        Err(msg)
    }

    /// Filters profiles incompatible with the given node and its imports.
    fn retain_compatible_profiles<
        C: Compiler,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        idx: usize,
        project: &Project<C, T>,
        candidates: &mut Vec<(usize, (&str, &C::Settings))>,
    ) -> Result<(), String> {
        let mut all_profiles = candidates.clone();

        let nodes: Vec<_> = self.node_ids(idx).collect();
        let mut failed_node_idx = None;
        for node in nodes.iter() {
            if let Some(req) = project.restrictions.get(&self.node(*node).path) {
                candidates.retain(|(_, (_, settings))| settings.satisfies_restrictions(&**req));
                if candidates.is_empty() {
                    failed_node_idx = Some(*node);
                    break;
                }
            }
        }

        let Some(failed_node_idx) = failed_node_idx else {
            // everything is fine
            return Ok(());
        };

        let failed_node = self.node(failed_node_idx);

        // retain only profiles compatible with the `failed_node`
        if let Some(req) = project.restrictions.get(&failed_node.path) {
            all_profiles.retain(|(_, (_, settings))| settings.satisfies_restrictions(&**req));
        }

        if all_profiles.is_empty() {
            let f = utils::source_name(&failed_node.path, &self.root).display();
            return Err(format!("Missing profile satisfying settings restrictions for {f}"));
        }

        // iterate over all the nodes once again and find the one incompatible
        for node in &nodes {
            if let Some(req) = project.restrictions.get(&self.node(*node).path) {
                if !all_profiles
                    .iter()
                    .any(|(_, (_, settings))| settings.satisfies_restrictions(&**req))
                {
                    let mut msg = "Found incompatible settings restrictions:\n".white().to_string();

                    self.format_imports_list(
                        idx,
                        [*node, failed_node_idx].into(),
                        project,
                        &mut msg,
                    )
                    .unwrap();
                    return Err(msg);
                }
            }
        }

        let mut msg = "Found incompatible settings restrictions:\n".white().to_string();
        self.format_imports_list(idx, nodes.into_iter().collect(), project, &mut msg).unwrap();
        Err(msg)
    }

    fn input_nodes_by_language(&self) -> HashMap<D::Language, Vec<usize>> {
        let mut nodes = HashMap::new();

        for idx in 0..self.edges.num_input_files {
            nodes.entry(self.nodes[idx].data.language()).or_insert_with(Vec::new).push(idx);
        }

        nodes
    }

    /// Returns a map of versions together with the input nodes that are compatible with that
    /// version.
    ///
    /// This will essentially do a DFS on all input sources and their transitive imports and
    /// checking that all can compiled with the version stated in the input file.
    ///
    /// Returns an error message with __all__ input files that don't have compatible imports.
    ///
    /// This also attempts to prefer local installations over remote available.
    /// If `offline` is set to `true` then only already installed.
    fn get_input_node_versions<
        C: Compiler<Language = L>,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        project: &Project<C, T>,
    ) -> Result<HashMap<L, HashMap<Version, Vec<usize>>>> {
        trace!("resolving input node versions");

        let mut resulted_nodes = HashMap::new();

        for (language, nodes) in self.input_nodes_by_language() {
            // this is likely called by an application and will be eventually printed so we don't
            // exit on first error, instead gather all the errors and return a bundled
            // error message instead
            let mut errors = Vec::new();

            // the sorted list of all versions
            let all_versions = if project.offline {
                project
                    .compiler
                    .available_versions(&language)
                    .into_iter()
                    .filter(|v| v.is_installed())
                    .collect()
            } else {
                project.compiler.available_versions(&language)
            };

            if all_versions.is_empty() && !nodes.is_empty() {
                return Err(SolcError::msg(format!(
                    "Found {language} sources, but no compiler versions are available for it"
                )));
            }

            // stores all versions and their nodes that can be compiled
            let mut versioned_nodes = HashMap::new();

            // stores all files and the versions they're compatible with
            let mut all_candidates = Vec::with_capacity(self.edges.num_input_files);
            // walking through the node's dep tree and filtering the versions along the way
            for idx in nodes {
                let mut candidates = all_versions.iter().collect::<Vec<_>>();
                // remove all incompatible versions from the candidates list by checking the node
                // and all its imports
                if let Err(err) = self.retain_compatible_versions(idx, &mut candidates, project) {
                    errors.push(err);
                } else {
                    // found viable candidates, pick the most recent version that's already
                    // installed
                    let candidate =
                        if let Some(pos) = candidates.iter().rposition(|v| v.is_installed()) {
                            candidates[pos]
                        } else {
                            candidates.last().expect("not empty; qed.")
                        }
                        .clone();

                    // also store all possible candidates to optimize the set
                    all_candidates.push((idx, candidates.into_iter().collect::<HashSet<_>>()));

                    versioned_nodes
                        .entry(candidate)
                        .or_insert_with(|| Vec::with_capacity(1))
                        .push(idx);
                }
            }

            // detected multiple versions but there might still exist a single version that
            // satisfies all sources
            if versioned_nodes.len() > 1 {
                versioned_nodes = Self::resolve_multiple_versions(all_candidates);
            }

            if versioned_nodes.len() == 1 {
                trace!(
                    "found exact solc version for all sources  \"{}\"",
                    versioned_nodes.keys().next().unwrap()
                );
            }

            if errors.is_empty() {
                trace!("resolved {} versions {:?}", versioned_nodes.len(), versioned_nodes.keys());
                resulted_nodes.insert(
                    language,
                    versioned_nodes
                        .into_iter()
                        .map(|(v, nodes)| (Version::from(v), nodes))
                        .collect(),
                );
            } else {
                error!("failed to resolve versions");
                return Err(SolcError::msg(errors.join("\n")));
            }
        }

        Ok(resulted_nodes)
    }

    #[allow(clippy::complexity)]
    fn resolve_settings<
        C: Compiler<Language = L>,
        T: ArtifactOutput<CompilerContract = C::CompilerContract>,
    >(
        &self,
        project: &Project<C, T>,
        input_nodes_versions: HashMap<L, HashMap<Version, Vec<usize>>>,
    ) -> Result<HashMap<L, HashMap<Version, HashMap<usize, Vec<usize>>>>> {
        let mut resulted_sources = HashMap::new();
        let mut errors = Vec::new();
        for (language, versions) in input_nodes_versions {
            let mut versioned_sources = HashMap::new();
            for (version, nodes) in versions {
                let mut profile_to_nodes = HashMap::new();
                for idx in nodes {
                    let mut profile_candidates =
                        project.settings_profiles().enumerate().collect::<Vec<_>>();
                    if let Err(err) =
                        self.retain_compatible_profiles(idx, project, &mut profile_candidates)
                    {
                        errors.push(err);
                    } else {
                        let (profile_idx, _) = profile_candidates.first().expect("exists");
                        profile_to_nodes.entry(*profile_idx).or_insert_with(Vec::new).push(idx);
                    }
                }
                versioned_sources.insert(version, profile_to_nodes);
            }
            resulted_sources.insert(language, versioned_sources);
        }

        if errors.is_empty() {
            Ok(resulted_sources)
        } else {
            error!("failed to resolve settings");
            Err(SolcError::msg(errors.join("\n")))
        }
    }

    /// Tries to find the "best" set of versions to nodes, See [Solc version
    /// auto-detection](#solc-version-auto-detection)
    ///
    /// This is a bit inefficient but is fine, the max. number of versions is ~80 and there's
    /// a high chance that the number of source files is <50, even for larger projects.
    fn resolve_multiple_versions(
        all_candidates: Vec<(usize, HashSet<&CompilerVersion>)>,
    ) -> HashMap<CompilerVersion, Vec<usize>> {
        // returns the intersection as sorted set of nodes
        fn intersection<'a>(
            mut sets: Vec<&HashSet<&'a CompilerVersion>>,
        ) -> Vec<&'a CompilerVersion> {
            if sets.is_empty() {
                return Vec::new();
            }

            let mut result = sets.pop().cloned().expect("not empty; qed.");
            if !sets.is_empty() {
                result.retain(|item| sets.iter().all(|set| set.contains(item)));
            }

            let mut v = result.into_iter().collect::<Vec<_>>();
            v.sort_unstable();
            v
        }

        /// returns the highest version that is installed
        /// if the candidates set only contains uninstalled versions then this returns the highest
        /// uninstalled version
        fn remove_candidate(candidates: &mut Vec<&CompilerVersion>) -> CompilerVersion {
            debug_assert!(!candidates.is_empty());

            if let Some(pos) = candidates.iter().rposition(|v| v.is_installed()) {
                candidates.remove(pos)
            } else {
                candidates.pop().expect("not empty; qed.")
            }
            .clone()
        }

        let all_sets = all_candidates.iter().map(|(_, versions)| versions).collect();

        // find all versions that satisfy all nodes
        let mut intersection = intersection(all_sets);
        if !intersection.is_empty() {
            let exact_version = remove_candidate(&mut intersection);
            let all_nodes = all_candidates.into_iter().map(|(node, _)| node).collect();
            trace!("resolved solc version compatible with all sources  \"{}\"", exact_version);
            return HashMap::from([(exact_version, all_nodes)]);
        }

        // no version satisfies all nodes
        let mut versioned_nodes: HashMap<_, _> = HashMap::new();

        // try to minimize the set of versions, this is guaranteed to lead to `versioned_nodes.len()
        // > 1` as no solc version exists that can satisfy all sources
        for (node, versions) in all_candidates {
            // need to sort them again
            let mut versions = versions.into_iter().collect::<Vec<_>>();
            versions.sort_unstable();

            let candidate = if let Some(idx) =
                versions.iter().rposition(|v| versioned_nodes.contains_key(*v))
            {
                // use a version that's already in the set
                versions.remove(idx).clone()
            } else {
                // use the highest version otherwise
                remove_candidate(&mut versions)
            };

            versioned_nodes.entry(candidate).or_insert_with(|| Vec::with_capacity(1)).push(node);
        }

        trace!(
            "no solc version can satisfy all source files, resolved multiple versions  \"{:?}\"",
            versioned_nodes.keys()
        );

        versioned_nodes
    }
}

/// An iterator over a node and its dependencies
#[derive(Debug)]
pub struct NodesIter<'a, D> {
    /// stack of nodes
    stack: VecDeque<usize>,
    visited: HashSet<usize>,
    graph: &'a GraphEdges<D>,
}

impl<'a, D> NodesIter<'a, D> {
    fn new(start: usize, graph: &'a GraphEdges<D>) -> Self {
        Self { stack: VecDeque::from([start]), visited: HashSet::new(), graph }
    }
}

impl<D> Iterator for NodesIter<'_, D> {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop_front()?;

        if self.visited.insert(node) {
            // push the node's direct dependencies to the stack if we haven't visited it already
            self.stack.extend(self.graph.imported_nodes(node).iter().copied());
        }
        Some(node)
    }
}

#[derive(Debug)]
pub struct Node<D> {
    /// path of the solidity  file
    path: PathBuf,
    /// content of the solidity file
    source: Source,
    /// parsed data
    pub data: D,
}

impl<D: ParsedSource> Node<D> {
    /// Reads the content of the file and returns a [Node] containing relevant information
    pub fn read(file: &Path) -> Result<Self> {
        let source = Source::read(file).map_err(|err| {
            let exists = err.path().exists();
            if !exists && err.path().is_symlink() {
                SolcError::ResolveBadSymlink(err)
            } else {
                // This is an additional check useful on OS that have case-sensitive paths, See also <https://docs.soliditylang.org/en/v0.8.17/path-resolution.html#import-callback>
                if !exists {
                    // check if there exists a file with different case
                    if let Some(existing_file) = find_case_sensitive_existing_file(file) {
                        SolcError::ResolveCaseSensitiveFileName { error: err, existing_file }
                    } else {
                        SolcError::Resolve(err)
                    }
                } else {
                    SolcError::Resolve(err)
                }
            }
        })?;
        let data = D::parse(source.as_ref(), file)?;
        Ok(Self { path: file.to_path_buf(), source, data })
    }

    /// Returns the path of the file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the contents of the file.
    pub fn content(&self) -> &str {
        &self.source.content
    }

    pub fn unpack(&self) -> (&PathBuf, &Source) {
        (&self.path, &self.source)
    }
}

/// Helper type for formatting a node
pub(crate) struct DisplayNode<'a, D> {
    node: &'a Node<D>,
    root: &'a PathBuf,
}

impl<D: ParsedSource> fmt::Display for DisplayNode<'_, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = utils::source_name(&self.node.path, self.root);
        write!(f, "{}", path.display())?;
        if let Some(v) = self.node.data.version_req() {
            write!(f, " {v}")?;
        }
        Ok(())
    }
}

/// Errors thrown when checking the solc version of a file
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
enum SourceVersionError {
    #[error("Failed to parse solidity version {0}: {1}")]
    InvalidVersion(String, SolcError),
    #[error("No solc version exists that matches the version requirement: {0}")]
    NoMatchingVersion(VersionReq),
    #[error("No solc version installed that matches the version requirement: {0}")]
    NoMatchingVersionOffline(VersionReq),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_resolve_hardhat_dependency_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/hardhat-sample");
        let paths = ProjectPathsConfig::hardhat(&root).unwrap();

        let graph = Graph::<SolData>::resolve(&paths).unwrap();

        assert_eq!(graph.edges.num_input_files, 1);
        assert_eq!(graph.files().len(), 2);

        assert_eq!(
            graph.files().clone(),
            HashMap::from([
                (paths.sources.join("Greeter.sol"), 0),
                (paths.root.join("node_modules/hardhat/console.sol"), 1),
            ])
        );
    }

    #[test]
    fn can_resolve_dapp_dependency_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
        let paths = ProjectPathsConfig::dapptools(&root).unwrap();

        let graph = Graph::<SolData>::resolve(&paths).unwrap();

        assert_eq!(graph.edges.num_input_files, 2);
        assert_eq!(graph.files().len(), 3);
        assert_eq!(
            graph.files().clone(),
            HashMap::from([
                (paths.sources.join("Dapp.sol"), 0),
                (paths.sources.join("Dapp.t.sol"), 1),
                (paths.root.join("lib/ds-test/src/test.sol"), 2),
            ])
        );

        let dapp_test = graph.node(1);
        assert_eq!(dapp_test.path, paths.sources.join("Dapp.t.sol"));
        assert_eq!(
            dapp_test.data.imports.iter().map(|i| i.data().path()).collect::<Vec<&PathBuf>>(),
            vec![&PathBuf::from("ds-test/test.sol"), &PathBuf::from("./Dapp.sol")]
        );
        assert_eq!(graph.imported_nodes(1).to_vec(), vec![2, 0]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn can_print_dapp_sample_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/dapp-sample");
        let paths = ProjectPathsConfig::dapptools(&root).unwrap();
        let graph = Graph::<SolData>::resolve(&paths).unwrap();
        let mut out = Vec::<u8>::new();
        tree::print(&graph, &Default::default(), &mut out).unwrap();

        assert_eq!(
            "
src/Dapp.sol >=0.6.6
src/Dapp.t.sol >=0.6.6
├── lib/ds-test/src/test.sol >=0.4.23
└── src/Dapp.sol >=0.6.6
"
            .trim_start()
            .as_bytes()
            .to_vec(),
            out
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn can_print_hardhat_sample_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/hardhat-sample");
        let paths = ProjectPathsConfig::hardhat(&root).unwrap();
        let graph = Graph::<SolData>::resolve(&paths).unwrap();
        let mut out = Vec::<u8>::new();
        tree::print(&graph, &Default::default(), &mut out).unwrap();
        assert_eq!(
            "contracts/Greeter.sol >=0.6.0
└── node_modules/hardhat/console.sol >=0.4.22, <0.9.0
",
            String::from_utf8(out).unwrap()
        );
    }

    #[test]
    #[cfg(feature = "svm-solc")]
    fn test_print_unresolved() {
        use crate::{solc::SolcCompiler, ProjectBuilder};

        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-data/incompatible-pragmas");
        let paths = ProjectPathsConfig::dapptools(&root).unwrap();
        let graph = Graph::<SolData>::resolve(&paths).unwrap();
        let Err(SolcError::Message(err)) = graph.get_input_node_versions(
            &ProjectBuilder::<SolcCompiler>::default()
                .paths(paths)
                .build(SolcCompiler::AutoDetect)
                .unwrap(),
        ) else {
            panic!("expected error");
        };

        snapbox::assert_data_eq!(
            err,
            snapbox::str![[r#"
[37mFound incompatible versions:
[0m[31msrc/A.sol =0.8.25[0m imports:
    [37msrc/B.sol[0m
    [31msrc/C.sol =0.7.0[0m
"#]]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn can_read_different_case() {
        use crate::resolver::parse::SolData;
        use std::fs::{self, create_dir_all};
        use utils::tempdir;

        let tmp_dir = tempdir("out").unwrap();
        let path = tmp_dir.path().join("forge-std");
        create_dir_all(&path).unwrap();
        let existing = path.join("Test.sol");
        let non_existing = path.join("test.sol");
        fs::write(
            existing,
            "
pragma solidity ^0.8.10;
contract A {}
        ",
        )
        .unwrap();

        assert!(!non_existing.exists());

        let found = crate::resolver::Node::<SolData>::read(&non_existing).unwrap_err();
        matches!(found, SolcError::ResolveCaseSensitiveFileName { .. });
    }
}
