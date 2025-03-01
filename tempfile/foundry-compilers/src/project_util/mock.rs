//! Helpers to generate mock projects

use foundry_compilers_artifacts::Remapping;
use foundry_compilers_core::error::{Result, SolcError};
use rand::{
    distributions::{Distribution, Uniform},
    seq::SliceRandom,
    Rng,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

use crate::{
    compilers::{multi::MultiCompilerParsedSource, Language, ParsedSource},
    resolver::GraphEdges,
    Graph, ProjectPathsConfig,
};

/// Represents the layout of a project
#[derive(Default, Serialize, Deserialize)]
pub struct MockProjectSkeleton {
    /// all files for the project
    pub files: Vec<MockFile>,
    /// all libraries
    pub libraries: Vec<MockLib>,
}

impl MockProjectSkeleton {
    /// Returns a list of file ids the given file id imports.
    pub fn imported_nodes(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        self.files[from].imports.iter().map(|i| i.file_id())
    }
}

/// Represents a virtual project
#[derive(Serialize)]
pub struct MockProjectGenerator {
    /// how to name things
    #[serde(skip)]
    name_strategy: Box<dyn NamingStrategy + 'static>,

    #[serde(flatten)]
    inner: MockProjectSkeleton,
}

impl MockProjectGenerator {
    /// Create a new project and populate it using the given settings
    pub fn new(settings: &MockProjectSettings) -> Self {
        let mut mock = Self::default();
        mock.populate(settings);
        mock
    }

    /// Create a skeleton of a real project
    pub fn create<D: ParsedSource>(paths: &ProjectPathsConfig) -> Result<Self> {
        fn get_libs<D: ParsedSource>(
            edges: &GraphEdges<D>,
            lib_folder: &Path,
        ) -> Option<HashMap<PathBuf, Vec<usize>>> {
            let mut libs: HashMap<_, Vec<_>> = HashMap::new();
            for lib_file in edges.library_files() {
                let component =
                    edges.node_path(lib_file).strip_prefix(lib_folder).ok()?.components().next()?;
                libs.entry(lib_folder.join(component)).or_default().push(lib_file);
            }
            Some(libs)
        }

        let graph = Graph::<MultiCompilerParsedSource>::resolve(paths)?;
        let mut gen = Self::default();
        let (_, edges) = graph.into_sources();

        // add all files as source files
        gen.add_sources(edges.files().count());

        // stores libs and their files
        let libs = get_libs(
            &edges,
            &paths.libraries.first().cloned().unwrap_or_else(|| paths.root.join("lib")),
        )
        .ok_or_else(|| SolcError::msg("Failed to detect libs"))?;

        // mark all files as libs
        for (lib_id, lib_files) in libs.into_values().enumerate() {
            let lib_name = gen.name_strategy.new_lib_name(lib_id);
            let offset = gen.inner.files.len();
            let lib = MockLib { name: lib_name, id: lib_id, num_files: lib_files.len(), offset };
            for lib_file in lib_files {
                let file = &mut gen.inner.files[lib_file];
                file.lib_id = Some(lib_id);
                file.name = gen.name_strategy.new_lib_name(file.id);
            }
            gen.inner.libraries.push(lib);
        }

        for id in edges.files() {
            for import in edges.imported_nodes(id).iter().copied() {
                let import = gen.get_import(import);
                gen.inner.files[id].imports.insert(import);
            }
        }

        Ok(gen)
    }

    /// Consumes the type and returns the underlying skeleton
    pub fn into_inner(self) -> MockProjectSkeleton {
        self.inner
    }

    /// Generate all solidity files and write under the paths config
    pub fn write_to<L: Language>(
        &self,
        paths: &ProjectPathsConfig<L>,
        version: &str,
    ) -> Result<()> {
        for file in self.inner.files.iter() {
            let imports = self.get_imports(file.id);
            let content = file.mock_content(version, imports.join("\n").as_str());
            super::create_contract_file(&file.target_path(self, paths), content)?;
        }

        Ok(())
    }

    fn get_imports(&self, file: usize) -> Vec<String> {
        let file = &self.inner.files[file];
        let mut imports = Vec::with_capacity(file.imports.len());

        for import in file.imports.iter() {
            match *import {
                MockImport::Internal(f) => {
                    imports.push(format!("import \"./{}.sol\";", self.inner.files[f].name));
                }
                MockImport::External(lib, f) => {
                    imports.push(format!(
                        "import \"{}/{}.sol\";",
                        self.inner.libraries[lib].name, self.inner.files[f].name
                    ));
                }
            }
        }
        imports
    }

    /// Returns all the remappings for the project for the given root path
    pub fn remappings_at(&self, root: &Path) -> Vec<Remapping> {
        self.inner
            .libraries
            .iter()
            .map(|lib| {
                let path = root.join("lib").join(&lib.name).join("src");
                format!("{}/={}/", lib.name, path.display()).parse().unwrap()
            })
            .collect()
    }

    /// Returns all the remappings for the project
    pub fn remappings(&self) -> Vec<Remapping> {
        self.inner
            .libraries
            .iter()
            .map(|lib| format!("{0}/=lib/{0}/src/", lib.name).parse().unwrap())
            .collect()
    }

    /// Generates a random project with random settings
    pub fn random() -> Self {
        let settings = MockProjectSettings::random();
        let mut mock = Self::default();
        mock.populate(&settings);
        mock
    }

    /// Adds sources and libraries and populates imports based on the settings
    pub fn populate(&mut self, settings: &MockProjectSettings) -> &mut Self {
        self.add_sources(settings.num_lib_files);
        for _ in 0..settings.num_libs {
            self.add_lib(settings.num_lib_files);
        }
        self.populate_imports(settings)
    }

    fn next_file_id(&self) -> usize {
        self.inner.files.len()
    }

    fn next_lib_id(&self) -> usize {
        self.inner.libraries.len()
    }

    /// Adds a new source file
    pub fn add_source(&mut self) -> &mut Self {
        let id = self.next_file_id();
        let name = self.name_strategy.new_source_file_name(id);
        let file =
            MockFile { id, name, imports: Default::default(), lib_id: None, emit_artifacts: true };
        self.inner.files.push(file);
        self
    }

    /// Adds `num` new source files
    pub fn add_sources(&mut self, num: usize) -> &mut Self {
        for _ in 0..num {
            self.add_source();
        }
        self
    }

    /// Adds a new lib file
    pub fn add_lib_file(&mut self, lib_id: usize) -> &mut Self {
        let id = self.next_file_id();
        let name = self.name_strategy.new_source_file_name(id);
        let file = MockFile {
            id,
            name,
            imports: Default::default(),
            lib_id: Some(lib_id),
            emit_artifacts: true,
        };
        self.inner.files.push(file);
        self
    }

    /// Adds `num` new source files
    pub fn add_lib_files(&mut self, num: usize, lib_id: usize) -> &mut Self {
        for _ in 0..num {
            self.add_lib_file(lib_id);
        }
        self
    }

    /// Adds a new lib with the number of lib files
    pub fn add_lib(&mut self, num_files: usize) -> &mut Self {
        let lib_id = self.next_lib_id();
        let lib_name = self.name_strategy.new_lib_name(lib_id);
        let offset = self.inner.files.len();
        self.add_lib_files(num_files, lib_id);
        self.inner.libraries.push(MockLib { name: lib_name, id: lib_id, num_files, offset });
        self
    }

    /// randomly assign empty file status so that mocked files don't emit artifacts
    pub fn assign_empty_files(&mut self) -> &mut Self {
        let mut rng = rand::thread_rng();
        let die = Uniform::from(0..self.inner.files.len());
        for file in self.inner.files.iter_mut() {
            let throw = die.sample(&mut rng);
            if throw == 0 {
                // give it a 1 in num(files) chance that the file will be empty
                file.emit_artifacts = false;
            }
        }
        self
    }

    /// Populates the imports of the project
    pub fn populate_imports(&mut self, settings: &MockProjectSettings) -> &mut Self {
        let mut rng = rand::thread_rng();

        // populate imports
        for id in 0..self.inner.files.len() {
            let imports = if let Some(lib) = self.inner.files[id].lib_id {
                let num_imports = rng
                    .gen_range(settings.min_imports..=settings.max_imports)
                    .min(self.inner.libraries[lib].num_files.saturating_sub(1));
                self.unique_imports_for_lib(&mut rng, lib, id, num_imports)
            } else {
                let num_imports = rng
                    .gen_range(settings.min_imports..=settings.max_imports)
                    .min(self.inner.files.len().saturating_sub(1));
                self.unique_imports_for_source(&mut rng, id, num_imports)
            };

            self.inner.files[id].imports = imports;
        }
        self
    }

    fn get_import(&self, id: usize) -> MockImport {
        if let Some(lib) = self.inner.files[id].lib_id {
            MockImport::External(lib, id)
        } else {
            MockImport::Internal(id)
        }
    }

    /// Returns the file for the given id
    pub fn get_file(&self, id: usize) -> &MockFile {
        &self.inner.files[id]
    }

    /// All file ids
    pub fn file_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.files.iter().map(|f| f.id)
    }

    /// Returns an iterator over all file ids that are source files or imported by source files
    ///
    /// In other words, all files that are relevant in order to compile the project's source files.
    pub fn used_file_ids(&self) -> impl Iterator<Item = usize> + '_ {
        let mut file_ids = BTreeSet::new();
        for file in self.internal_file_ids() {
            file_ids.extend(NodesIter::new(file, &self.inner))
        }
        file_ids.into_iter()
    }

    /// All ids of internal files
    pub fn internal_file_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.files.iter().filter(|f| !f.is_external()).map(|f| f.id)
    }

    /// All ids of external files
    pub fn external_file_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.inner.files.iter().filter(|f| f.is_external()).map(|f| f.id)
    }

    /// generates exactly `num` unique imports in the range of all files
    ///
    /// # Panics
    ///
    /// if `num` can't be satisfied because the range is too narrow
    fn unique_imports_for_source<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
        id: usize,
        num: usize,
    ) -> BTreeSet<MockImport> {
        assert!(self.inner.files.len() > num);
        let mut imports: Vec<_> = (0..self.inner.files.len()).collect();
        imports.shuffle(rng);
        imports.into_iter().filter(|i| *i != id).map(|id| self.get_import(id)).take(num).collect()
    }

    /// Modifies the content of the given file
    pub fn modify_file(
        &self,
        id: usize,
        paths: &ProjectPathsConfig,
        version: &str,
    ) -> Result<PathBuf> {
        let file = &self.inner.files[id];
        let target = file.target_path(self, paths);
        let content = file.modified_content(version, self.get_imports(id).join("\n").as_str());
        super::create_contract_file(&target, content)?;
        Ok(target)
    }

    /// generates exactly `num` unique imports in the range of a lib's files
    ///
    /// # Panics
    ///
    /// if `num` can't be satisfied because the range is too narrow
    fn unique_imports_for_lib<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
        lib_id: usize,
        id: usize,
        num: usize,
    ) -> BTreeSet<MockImport> {
        let lib = &self.inner.libraries[lib_id];
        assert!(lib.num_files > num);
        let mut imports: Vec<_> = (lib.offset..(lib.offset + lib.len())).collect();
        imports.shuffle(rng);
        imports.into_iter().filter(|i| *i != id).map(|id| self.get_import(id)).take(num).collect()
    }
}

impl Default for MockProjectGenerator {
    fn default() -> Self {
        Self { name_strategy: Box::<SimpleNamingStrategy>::default(), inner: Default::default() }
    }
}

impl From<MockProjectSkeleton> for MockProjectGenerator {
    fn from(inner: MockProjectSkeleton) -> Self {
        Self { inner, ..Default::default() }
    }
}

/// Used to determine the names for elements
trait NamingStrategy {
    /// Return a new name for the given source file id
    fn new_source_file_name(&mut self, id: usize) -> String;

    /// Return a new name for the given source file id
    #[allow(unused)]
    fn new_lib_file_name(&mut self, id: usize) -> String;

    /// Return a new name for the given lib id
    fn new_lib_name(&mut self, id: usize) -> String;
}

/// A primitive naming that simply uses ids to create unique names
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SimpleNamingStrategy;

impl NamingStrategy for SimpleNamingStrategy {
    fn new_source_file_name(&mut self, id: usize) -> String {
        format!("SourceFile{id}")
    }

    fn new_lib_file_name(&mut self, id: usize) -> String {
        format!("LibFile{id}")
    }

    fn new_lib_name(&mut self, id: usize) -> String {
        format!("Lib{id}")
    }
}

/// Skeleton of a mock source file
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MockFile {
    /// internal id of this file
    pub id: usize,
    /// The source name of this file
    pub name: String,
    /// all the imported files
    pub imports: BTreeSet<MockImport>,
    /// lib id if this file is part of a lib
    pub lib_id: Option<usize>,
    /// whether this file should emit artifacts
    pub emit_artifacts: bool,
}

impl MockFile {
    /// Returns `true` if this file is part of an external lib
    pub fn is_external(&self) -> bool {
        self.lib_id.is_some()
    }

    pub fn target_path<L: Language>(
        &self,
        gen: &MockProjectGenerator,
        paths: &ProjectPathsConfig<L>,
    ) -> PathBuf {
        let mut target = if let Some(lib) = self.lib_id {
            paths.root.join("lib").join(&gen.inner.libraries[lib].name).join("src").join(&self.name)
        } else {
            paths.sources.join(&self.name)
        };
        target.set_extension("sol");

        target
    }

    /// Returns the content to use for a modified file
    ///
    /// The content here is arbitrary, it should only differ from the mocked content
    pub fn modified_content(&self, version: &str, imports: &str) -> String {
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {};
{}
contract {} {{
    function hello() public {{}}
}}
            "#,
            version, imports, self.name
        )
    }

    /// Returns a mocked content for the file
    pub fn mock_content(&self, version: &str, imports: &str) -> String {
        if self.emit_artifacts {
            format!(
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {};
{}
contract {} {{}}
            "#,
                version, imports, self.name
            )
        } else {
            format!(
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity {version};
{imports}
            "#,
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MockImport {
    /// Import from the same project
    Internal(usize),
    /// external library import
    /// (`lib id`, `file id`)
    External(usize, usize),
}

impl MockImport {
    pub fn file_id(&self) -> usize {
        *match self {
            Self::Internal(id) => id,
            Self::External(_, id) => id,
        }
    }
}

/// Container of a mock lib
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MockLib {
    /// name of the lib, like `ds-test`
    pub name: String,
    /// internal id of this lib
    pub id: usize,
    /// offset in the total set of files
    pub offset: usize,
    /// number of files included in this lib
    pub num_files: usize,
}

impl MockLib {
    pub fn len(&self) -> usize {
        self.num_files
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Settings to use when generate a mock project
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockProjectSettings {
    /// number of source files to generate
    pub num_sources: usize,
    /// number of libraries to use
    pub num_libs: usize,
    /// how many lib files to generate per lib
    pub num_lib_files: usize,
    /// min amount of import statements a file can use
    pub min_imports: usize,
    /// max amount of import statements a file can use
    pub max_imports: usize,
    /// whether to also use files that don't emit artifacts
    pub allow_no_artifacts_files: bool,
}

impl MockProjectSettings {
    /// Generates a new instance with random settings within an arbitrary range
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        // arbitrary thresholds
        Self {
            num_sources: rng.gen_range(2..25),
            num_libs: rng.gen_range(0..5),
            num_lib_files: rng.gen_range(1..10),
            min_imports: rng.gen_range(0..3),
            max_imports: rng.gen_range(4..10),
            allow_no_artifacts_files: true,
        }
    }

    /// Generates settings for a large project
    pub fn large() -> Self {
        // arbitrary thresholds
        Self {
            num_sources: 35,
            num_libs: 4,
            num_lib_files: 15,
            min_imports: 3,
            max_imports: 12,
            allow_no_artifacts_files: true,
        }
    }
}

impl Default for MockProjectSettings {
    fn default() -> Self {
        // these are arbitrary
        Self {
            num_sources: 20,
            num_libs: 2,
            num_lib_files: 10,
            min_imports: 0,
            max_imports: 5,
            allow_no_artifacts_files: true,
        }
    }
}

/// An iterator over a node and its dependencies
struct NodesIter<'a> {
    /// stack of nodes
    stack: VecDeque<usize>,
    visited: HashSet<usize>,
    skeleton: &'a MockProjectSkeleton,
}

impl<'a> NodesIter<'a> {
    fn new(start: usize, skeleton: &'a MockProjectSkeleton) -> Self {
        Self { stack: VecDeque::from([start]), visited: HashSet::new(), skeleton }
    }
}

impl Iterator for NodesIter<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        let file = self.stack.pop_front()?;

        if self.visited.insert(file) {
            // push the file's direct imports to the stack if we haven't visited it already
            self.stack.extend(self.skeleton.imported_nodes(file));
        }
        Some(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_generate_mock_project() {
        let _ = MockProjectGenerator::random();
    }
}
