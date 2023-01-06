use ethers_solc::utils::source_files_iter;
use forge_fmt::Visitable;
use itertools::Itertools;
use rayon::prelude::*;
use solang_parser::pt::Base;
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    format::DocFormat,
    output::DocOutput,
    parser::{ParseItem, ParseSource, Parser},
    writer::BufWriter,
};

#[derive(Debug)]
struct DocFile {
    source: ParseItem,
    source_path: PathBuf,
    target_path: PathBuf,
}

impl DocFile {
    fn new(source: ParseItem, source_path: PathBuf, target_path: PathBuf) -> Self {
        Self { source_path, source, target_path }
    }
}

/// Build Solidity documentation for a project from natspec comments.
/// The builder parses the source files using [Parser],
/// then formats and writes the elements as the output.
#[derive(Debug)]
pub struct DocBuilder {
    /// The project root
    pub root: PathBuf,
    /// Path to Solidity source files.
    pub sources: PathBuf,
    /// Output path.
    pub out: PathBuf,
    /// The documentation title.
    pub title: String,
}

// TODO: consider using `tfio`
impl DocBuilder {
    const README: &'static str = "README.md";
    const SUMMARY: &'static str = "SUMMARY.md";
    const SOL_EXT: &'static str = "sol";

    /// Get the output directory
    pub fn out_dir(&self) -> PathBuf {
        self.root.join(&self.out)
    }

    /// Parse the sources and build the documentation.
    pub fn build(self) -> eyre::Result<()> {
        // Collect and parse source files
        let sources: Vec<_> = source_files_iter(&self.sources).collect();
        let files = sources
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let source = fs::read_to_string(path)?;
                let (mut source_unit, comments) =
                    solang_parser::parse(&source, i).map_err(|diags| {
                        eyre::eyre!(
                            "Failed to parse Solidity code for {}\nDebug info: {:?}",
                            path.display(),
                            diags
                        )
                    })?;
                let mut doc = Parser::new(comments);
                source_unit.visit(&mut doc)?;
                doc.items()
                    .into_iter()
                    .map(|item| {
                        let relative_path = path.strip_prefix(&self.root)?.join(item.filename());
                        let target_path = self.out.join("src").join(relative_path);
                        Ok(DocFile::new(item, path.clone(), target_path))
                    })
                    .collect::<eyre::Result<Vec<_>>>()
            })
            .collect::<eyre::Result<Vec<_>>>()?;

        // Flatten and sort the results
        let files = files.into_iter().flatten().sorted_by(|file1, file2| {
            file1.source_path.display().to_string().cmp(&file2.source_path.display().to_string())
        });

        // Write mdbook related files
        self.write_mdbook(files.collect::<Vec<_>>())?;

        Ok(())
    }

    fn write_mdbook(&self, files: Vec<DocFile>) -> eyre::Result<()> {
        let out_dir = self.out_dir();
        let out_dir_src = out_dir.join("src");
        fs::create_dir_all(&out_dir_src)?;

        // Write readme content if any
        let readme_content = {
            let src_readme = self.sources.join(Self::README);
            if src_readme.exists() {
                fs::read_to_string(src_readme)?
            } else {
                String::new()
            }
        };
        let readme_path = out_dir_src.join(Self::README);
        fs::write(&readme_path, readme_content)?;

        // Write summary and section readmes
        let mut summary = BufWriter::default();
        summary.write_title("Summary")?;
        summary.write_link_list_item("README", Self::README, 0)?;
        // TODO:
        self.write_summary_section(&mut summary, &files.iter().collect::<Vec<_>>(), None, 0)?;
        fs::write(out_dir_src.join(Self::SUMMARY), summary.finish())?;

        // Create dir for static files
        let out_dir_static = out_dir_src.join("static");
        fs::create_dir_all(&out_dir_static)?;

        // Write solidity syntax highlighting
        fs::write(
            out_dir_static.join("solidity.min.js"),
            include_str!("../static/solidity.min.js"),
        )?;

        // Write book config
        let mut book: toml::Value = toml::from_str(include_str!("../static/book.toml"))?;
        let book_entry = book["book"].as_table_mut().unwrap();
        book_entry.insert(String::from("title"), self.title.clone().into());
        fs::write(self.out_dir().join("book.toml"), toml::to_string_pretty(&book)?)?;

        // Write .gitignore
        let gitignore = "book/";
        fs::write(self.out_dir().join(".gitignore"), gitignore)?;

        // Write doc files
        for file in files {
            let doc_content = file.source.doc()?;
            fs::create_dir_all(
                file.target_path.parent().ok_or(eyre::format_err!("empty target path; noop"))?,
            )?;
            fs::write(file.target_path, doc_content)?;
        }

        Ok(())
    }

    fn write_summary_section(
        &self,
        summary: &mut BufWriter,
        files: &[&DocFile],
        path: Option<&Path>,
        depth: usize,
    ) -> eyre::Result<()> {
        if files.is_empty() {
            return Ok(())
        }

        if let Some(path) = path {
            let title = path.iter().last().unwrap().to_string_lossy();
            if depth == 1 {
                summary.write_title(&title)?;
            } else {
                let summary_path = path.join(Self::README);
                summary.write_link_list_item(
                    &title,
                    &summary_path.display().to_string(),
                    depth - 1,
                )?;
            }
        }

        // Group entries by path depth
        let mut grouped = HashMap::new();
        for file in files {
            let path = file.source_path.strip_prefix(&self.root)?;
            let key = path.iter().take(depth + 1).collect::<PathBuf>();
            grouped.entry(key).or_insert_with(Vec::new).push(*file);
        }
        // Sort entries by path depth
        let grouped = grouped.into_iter().sorted_by(|(lhs, _), (rhs, _)| {
            let lhs_at_end = lhs.extension().map(|ext| ext.eq(Self::SOL_EXT)).unwrap_or_default();
            let rhs_at_end = rhs.extension().map(|ext| ext.eq(Self::SOL_EXT)).unwrap_or_default();
            if lhs_at_end == rhs_at_end {
                lhs.cmp(rhs)
            } else if lhs_at_end {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        });

        let mut readme = BufWriter::default();
        readme.write_raw("\n\n# Contents\n")?; // TODO:
        for (path, files) in grouped {
            if path.extension().map(|ext| ext.eq(Self::SOL_EXT)).unwrap_or_default() {
                for file in files {
                    let ident = file.source.source.ident();

                    let readme_path = Path::new("/").join(&path).display().to_string();
                    readme.write_link_list_item(&ident, &readme_path, 0)?;

                    let summary_path =
                        file.target_path.strip_prefix("docs/src")?.display().to_string();
                    summary.write_link_list_item(&ident, &summary_path, depth)?;
                }
            } else {
                let name = path.iter().last().unwrap().to_string_lossy();
                let readme_path = Path::new("/").join(&path).display().to_string();
                readme.write_link_list_item(&name, &readme_path, 0)?;
                self.write_summary_section(summary, &files, Some(&path), depth + 1)?;
            }
        }
        if !readme.is_empty() {
            if let Some(path) = path {
                let path = self.out_dir().join("src").join(path);
                fs::create_dir_all(&path)?;
                fs::write(path.join(Self::README), readme.finish())?;
            }
        }
        Ok(())
    }

    // TODO:
    fn lookup_contract_base<'a>(
        &self,
        docs: &[(PathBuf, Vec<ParseItem>)],
        base: &Base,
    ) -> eyre::Result<Option<String>> {
        for (base_path, base_doc) in docs {
            for base_part in base_doc.iter() {
                if let ParseSource::Contract(base_contract) = &base_part.source {
                    if base.name.identifiers.last().unwrap().name == base_contract.name.name {
                        let path = PathBuf::from("/").join(
                            base_path
                                .strip_prefix(&self.root)?
                                .join(format!("contract.{}.md", base_contract.name)),
                        );
                        return Ok(Some(
                            DocOutput::Link(&base.doc()?, &path.display().to_string()).doc()?,
                        ))
                    }
                }
            }
        }

        Ok(None)
    }
}
