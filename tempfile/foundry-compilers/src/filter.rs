//! Types to apply filter to input types

use crate::{
    compilers::{multi::MultiCompilerParsedSource, CompilerSettings, ParsedSource},
    resolver::{parse::SolData, GraphEdges},
    Sources,
};
use foundry_compilers_artifacts::output_selection::OutputSelection;
use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
};

/// A predicate property that determines whether a file satisfies a certain condition
pub trait FileFilter: dyn_clone::DynClone + Send + Sync {
    /// The predicate function that should return if the given `file` should be included.
    fn is_match(&self, file: &Path) -> bool;
}

dyn_clone::clone_trait_object!(FileFilter);

impl<F: Fn(&Path) -> bool + Clone + Send + Sync> FileFilter for F {
    fn is_match(&self, file: &Path) -> bool {
        (self)(file)
    }
}

/// An [FileFilter] that matches all solidity files that end with `.t.sol`
#[derive(Clone, Default)]
pub struct TestFileFilter {
    _priv: (),
}

impl fmt::Debug for TestFileFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestFileFilter").finish()
    }
}

impl fmt::Display for TestFileFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestFileFilter")
    }
}

impl FileFilter for TestFileFilter {
    fn is_match(&self, file: &Path) -> bool {
        file.file_name().and_then(|s| s.to_str()).map(|s| s.ends_with(".t.sol")).unwrap_or_default()
    }
}

pub trait MaybeSolData {
    fn sol_data(&self) -> Option<&SolData>;
}

impl MaybeSolData for SolData {
    fn sol_data(&self) -> Option<&SolData> {
        Some(self)
    }
}

impl MaybeSolData for MultiCompilerParsedSource {
    fn sol_data(&self) -> Option<&SolData> {
        match self {
            Self::Solc(data) => Some(data),
            _ => None,
        }
    }
}

/// A type that can apply a filter to a set of preprocessed [Sources] in order to set sparse
/// output for specific files
#[derive(Default)]
pub enum SparseOutputFilter<'a> {
    /// Sets the configured [OutputSelection] for dirty files only.
    ///
    /// In other words, we request the output of solc only for files that have been detected as
    /// _dirty_.
    #[default]
    Optimized,
    /// Apply an additional filter to [Sources] to
    Custom(&'a dyn FileFilter),
}

impl<'a> SparseOutputFilter<'a> {
    pub fn new(filter: Option<&'a dyn FileFilter>) -> Self {
        if let Some(f) = filter {
            SparseOutputFilter::Custom(f)
        } else {
            SparseOutputFilter::Optimized
        }
    }

    /// While solc needs all the files to compile the actual _dirty_ files, we can tell solc to
    /// output everything for those dirty files as currently configured in the settings, but output
    /// nothing for the other files that are _not_ dirty.
    ///
    /// This will modify the [OutputSelection] of the [CompilerSettings] so that we explicitly
    /// select the files' output based on their state.
    ///
    /// This also takes the project's graph as input, this allows us to check if the files the
    /// filter matches depend on libraries that need to be linked
    pub fn sparse_sources<D: ParsedSource, S: CompilerSettings>(
        &self,
        sources: &Sources,
        settings: &mut S,
        graph: &GraphEdges<D>,
    ) -> Vec<PathBuf> {
        let mut full_compilation: HashSet<PathBuf> = sources
            .dirty_files()
            .flat_map(|file| {
                // If we have a custom filter and file does not match, we skip it.
                if let Self::Custom(f) = self {
                    if !f.is_match(file) {
                        return vec![];
                    }
                }

                // Collect compilation dependencies for sources needing compilation.
                let mut required_sources = vec![file.clone()];
                if let Some(data) = graph.get_parsed_source(file) {
                    let imports = graph.imports(file).into_iter().filter_map(|import| {
                        graph.get_parsed_source(import).map(|data| (import.as_path(), data))
                    });
                    for import in data.compilation_dependencies(imports) {
                        let import = import.to_path_buf();

                        #[cfg(windows)]
                        let import = {
                            use path_slash::PathBufExt;

                            PathBuf::from(import.to_slash_lossy().to_string())
                        };

                        required_sources.push(import);
                    }
                }

                required_sources
            })
            .collect();

        // Remove clean sources, those will be read from cache.
        full_compilation.retain(|file| sources.0.get(file).is_some_and(|s| s.is_dirty()));

        settings.update_output_selection(|selection| {
            trace!(
                "optimizing output selection for {} sources",
                sources.len() - full_compilation.len()
            );
            let default_selection = selection
                .as_mut()
                .remove("*")
                .unwrap_or_else(OutputSelection::default_file_output_selection);

            // set output selections
            for file in sources.0.keys() {
                let key = file.display().to_string();
                let output = if full_compilation.contains(file) {
                    default_selection.clone()
                } else {
                    OutputSelection::empty_file_output_select()
                };
                selection.as_mut().insert(key, output);
            }
        });

        full_compilation.into_iter().collect()
    }
}

impl fmt::Debug for SparseOutputFilter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SparseOutputFilter::Optimized => f.write_str("Optimized"),
            SparseOutputFilter::Custom(_) => f.write_str("Custom"),
        }
    }
}
