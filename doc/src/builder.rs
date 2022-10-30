use ethers_solc::utils::source_files_iter;
use eyre;
use forge_fmt::Visitable;
use itertools::Itertools;
use rayon::prelude::*;
use solang_parser::{
    doccomment::DocCommentTag,
    pt::{Base, Identifier, Parameter},
};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    as_code::AsCode,
    config::DocConfig,
    format::DocFormat,
    helpers::*,
    macros::*,
    output::DocOutput,
    parser::{DocElement, DocParser, DocPart},
};

/// Build Solidity documentation for a project from natspec comments.
/// The builder parses the source files using [DocParser],
/// then formats and writes the elements as the output.
#[derive(Debug)]
pub struct DocBuilder {
    config: DocConfig,
}

impl DocBuilder {
    /// Construct a new builder with default configuration.
    pub fn new() -> Self {
        DocBuilder { config: DocConfig::default() }
    }

    /// Construct a new builder with provided configuration
    pub fn from_config(config: DocConfig) -> Self {
        DocBuilder { config }
    }

    /// Get the output directory for generated doc files.
    pub fn out_dir_src(&self) -> PathBuf {
        self.config.out_dir().join("src")
    }

    /// Parse the sources and build the documentation.
    pub fn build(self) -> eyre::Result<()> {
        let sources: Vec<_> = source_files_iter(&self.config.sources).collect();
        let docs = sources
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let source = fs::read_to_string(&path)?;
                let (mut source_unit, comments) =
                    solang_parser::parse(&source, i).map_err(|diags| {
                        eyre::eyre!(
                            "Failed to parse Solidity code for {}\nDebug info: {:?}",
                            path.display(),
                            diags
                        )
                    })?;
                let mut doc = DocParser::new(comments);
                source_unit.visit(&mut doc)?;

                Ok((path.clone(), doc.parts))
            })
            .collect::<eyre::Result<Vec<_>>>()?;

        let docs = docs.into_iter().sorted_by(|(path1, _), (path2, _)| {
            path1.display().to_string().cmp(&path2.display().to_string())
        });

        fs::create_dir_all(self.out_dir_src())?;

        let mut filenames = vec![];
        for (path, doc) in docs.as_ref() {
            for part in doc.iter() {
                // TODO: other top level elements
                if let DocElement::Contract(ref contract) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1(&contract.name.name))?;

                    if !contract.base.is_empty() {
                        let bases = contract
                            .base
                            .iter()
                            .map(|base| {
                                Ok(self
                                    .lookup_contract_base(docs.as_ref(), base)?
                                    .unwrap_or(base.doc()))
                            })
                            .collect::<eyre::Result<Vec<_>>>()?;

                        writeln_doc!(
                            doc_file,
                            "{} {}\n",
                            DocOutput::Bold("Inherits:"),
                            bases.join(", ")
                        )?;
                    }

                    writeln_doc!(doc_file, part.comments)?;

                    let mut attributes = vec![];
                    let mut funcs = vec![];
                    let mut events = vec![];
                    let mut errors = vec![];
                    let mut structs = vec![];
                    let mut enumerables = vec![];

                    for child in part.children.iter() {
                        // TODO: remove `clone`s
                        match &child.element {
                            DocElement::Function(func) => funcs.push((func, &child.comments)),
                            DocElement::Variable(var) => attributes.push((var, &child.comments)),
                            DocElement::Event(event) => events.push((event, &child.comments)),
                            DocElement::Error(error) => errors.push((error, &child.comments)),
                            DocElement::Struct(structure) => {
                                structs.push((structure, &child.comments))
                            },
                            DocElement::Enum(enumerable) => enumerables.push((enumerable, &child.comments)),
                            _ => (),
                        }
                    }

                    if !attributes.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Attributes"))?;
                        for (var, comments) in attributes {
                            writeln_doc!(doc_file, "{}\n{}\n", var, comments)?;
                            writeln_code!(doc_file, "{}", var)?;
                            writeln!(doc_file)?;
                        }
                    }

                    if !funcs.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Functions"))?;
                        for (func, comments) in funcs {
                            writeln_doc!(doc_file, "{}\n", func)?;
                            writeln_doc!(
                                doc_file,
                                "{}\n",
                                filter_comments_without_tags(&comments, vec!["param", "return"])
                            )?;
                            writeln_code!(doc_file, "{}", func)?;

                            let params: Vec<_> =
                                func.params.iter().filter_map(|p| p.1.as_ref()).collect();
                            let param_comments = filter_comments_by_tag(&comments, "param");
                            if !params.is_empty() && !param_comments.is_empty() {
                                writeln_doc!(
                                    doc_file,
                                    "{}\n{}",
                                    DocOutput::H3("Parameters"),
                                    self.format_comment_table(
                                        &["Name", "Type", "Description"],
                                        &params,
                                        &param_comments
                                    )?
                                )?;
                            }

                            let returns: Vec<_> =
                                func.returns.iter().filter_map(|p| p.1.as_ref()).collect();
                            let returns_comments = filter_comments_by_tag(&comments, "return");
                            if !returns.is_empty() && !returns_comments.is_empty() {
                                writeln_doc!(
                                    doc_file,
                                    "{}\n{}",
                                    DocOutput::H3("Returns"),
                                    self.format_comment_table(
                                        &["Name", "Type", "Description"],
                                        &params,
                                        &returns_comments
                                    )?
                                )?;
                            }

                            writeln!(doc_file)?;
                        }
                    }

                    if !events.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Events"))?;
                        for (ev, comments) in events {
                            writeln_doc!(doc_file, "{}\n{}\n", ev, comments)?;
                            writeln_code!(doc_file, "{}", ev)?;
                            writeln!(doc_file)?;
                        }
                    }

                    if !errors.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Errors"))?;
                        for (err, comments) in errors {
                            writeln_doc!(doc_file, "{}\n{}\n", err, comments)?;
                            writeln_code!(doc_file, "{}", err)?;
                            writeln!(doc_file)?;
                        }
                    }

                    if !structs.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Structs"))?;
                        for (structure, comments) in structs {
                            writeln_doc!(doc_file, "{}\n{}\n", structure, comments)?;
                            writeln_code!(doc_file, "{}", structure)?;
                            writeln!(doc_file)?;
                        }
                    }

                    if !enumerables.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Enums"))?;
                        for (enumerable, comments) in enumerables {
                            writeln_doc!(doc_file, "{}\n{}\n", enumerable, comments)?;
                            writeln_code!(doc_file, "{}", enumerable)?;
                            writeln!(doc_file)?;
                        }
                    }

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("contract.{}.md", contract.name));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    filenames.push((contract.name.clone(), new_path));
                }

                if let DocElement::Error(ref error) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1("Error"))?;

                    writeln_doc!(doc_file, "{}\n{}\n", error, part.comments)?;
                    writeln_code!(doc_file, "{}", error)?;
                    writeln!(doc_file)?;

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("error.{}.md", error.name));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    // TODO: save new `filenames` for book_config?
                }

                if let DocElement::Event(ref event) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1("Event"))?;

                    writeln_doc!(doc_file, "{}\n{}\n", event, part.comments)?;
                    writeln_code!(doc_file, "{}", event)?;
                    writeln!(doc_file)?;

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("event.{}.md", event.name));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    // TODO: save new `filenames` for book_config?
                }

                if let DocElement::Struct(ref structure) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1("Struct"))?;

                    writeln_doc!(doc_file, "{}\n{}\n", structure, part.comments)?;
                    writeln_code!(doc_file, "{}", structure)?;
                    writeln!(doc_file)?;

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("struct.{}.md", structure.name));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    // TODO: save new `filenames` for book_config?
                }

                if let DocElement::Enum(ref enumerable) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1("Enum"))?;

                    writeln_doc!(doc_file, "{}\n{}\n", enumerable, part.comments)?;
                    writeln_code!(doc_file, "{}", enumerable)?;
                    writeln!(doc_file)?;

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("enum.{}.md", enumerable.name));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    // TODO: save new `filenames` for book_config?
                }

                if let DocElement::Function(ref func) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1("Function"))?;

                    writeln_doc!(doc_file, "{}\n", func)?;
                    writeln_doc!(
                        doc_file,
                        "{}\n",
                        filter_comments_without_tags(&part.comments, vec!["param", "return"])
                    )?;
                    writeln_code!(doc_file, "{}", func)?;

                    let params: Vec<_> =
                        func.params.iter().filter_map(|p| p.1.as_ref()).collect();
                    let param_comments = filter_comments_by_tag(&part.comments, "param");
                    if !params.is_empty() && !param_comments.is_empty() {
                        writeln_doc!(
                            doc_file,
                            "{}\n{}",
                            DocOutput::H3("Parameters"),
                            self.format_comment_table(
                                &["Name", "Type", "Description"],
                                &params,
                                &param_comments
                            )?
                        )?;
                    }

                    let returns: Vec<_> =
                        func.returns.iter().filter_map(|p| p.1.as_ref()).collect();
                    let returns_comments = filter_comments_by_tag(&part.comments, "return");
                    if !returns.is_empty() && !returns_comments.is_empty() {
                        writeln_doc!(
                            doc_file,
                            "{}\n{}",
                            DocOutput::H3("Returns"),
                            self.format_comment_table(
                                &["Name", "Type", "Description"],
                                &params,
                                &returns_comments
                            )?
                        )?;
                    }
                    writeln!(doc_file)?;
                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(self.out_dir_src().join(&new_path))?;
                    let new_path = new_path.join(&format!("function.{}.md", func.name.as_ref().map(|name| name.name.to_owned()).unwrap_or_default()));

                    fs::write(self.out_dir_src().join(&new_path), doc_file)?;
                    // TODO: save new `filenames` for book_config?
                }
            }
        }

        self.write_book_config(filenames)?;

        Ok(())
    }

    fn format_comment_table(
        &self,
        headers: &[&str],
        params: &[&Parameter],
        comments: &[&DocCommentTag],
    ) -> eyre::Result<String> {
        let mut out = String::new();
        let separator = headers.iter().map(|h| "-".repeat(h.len())).join("|");

        writeln!(out, "|{}|", headers.join("|"))?;
        writeln!(out, "|{separator}|")?;
        for param in params {
            let param_name = param.name.as_ref().map(|n| n.name.to_owned());
            let description = param_name
                .as_ref()
                .map(|name| {
                    comments.iter().find_map(|comment| {
                        match comment.value.trim_start().split_once(' ') {
                            Some((tag_name, description)) if tag_name.trim().eq(name.as_str()) => {
                                Some(description.replace("\n", " "))
                            }
                            _ => None,
                        }
                    })
                })
                .flatten()
                .unwrap_or_default();
            let row = [
                param_name.unwrap_or_else(|| "<none>".to_owned()),
                param.ty.as_code(),
                description,
            ];
            writeln!(out, "|{}|", row.join("|"))?;
        }

        Ok(out)
    }

    fn write_book_config(&self, filenames: Vec<(Identifier, PathBuf)>) -> eyre::Result<()> {
        let out_dir_src = self.out_dir_src();

        let readme_content = {
            let src_readme = self.config.sources.join("README.md");
            if src_readme.exists() {
                fs::read_to_string(src_readme)?
            } else {
                String::new()
            }
        };
        let readme_path = out_dir_src.join("README.md");
        fs::write(&readme_path, readme_content)?;

        let mut summary = String::new();
        writeln_doc!(summary, DocOutput::H1("Summary"))?;
        writeln_doc!(summary, "- {}", DocOutput::Link("README", "README.md"))?;

        self.write_summary_section(&mut summary, 0, None, &filenames)?;

        fs::write(out_dir_src.join("SUMMARY.md"), summary.clone())?;

        let out_dir_static = out_dir_src.join("static");
        fs::create_dir_all(&out_dir_static)?;
        fs::write(
            out_dir_static.join("solidity.min.js"),
            include_str!("../static/solidity.min.js"),
        )?;

        let mut book: toml::Value = toml::from_str(include_str!("../static/book.toml"))?;
        let book_entry = book["book"].as_table_mut().unwrap();
        book_entry.insert(String::from("title"), self.config.title.clone().into());
        fs::write(self.config.out_dir().join("book.toml"), toml::to_string_pretty(&book)?)?;

        Ok(())
    }

    fn write_summary_section(
        &self,
        out: &mut String,
        depth: usize,
        path: Option<&Path>,
        entries: &[(Identifier, PathBuf)],
    ) -> eyre::Result<()> {
        if !entries.is_empty() {
            if let Some(path) = path {
                let title = path.iter().last().unwrap().to_string_lossy();
                let section_title = if depth == 1 {
                    DocOutput::H1(&title).doc()
                } else {
                    format!(
                        "{}- {}",
                        " ".repeat((depth - 1) * 2),
                        DocOutput::Link(
                            &title,
                            &path.join("README.md").as_os_str().to_string_lossy()
                        )
                    )
                };
                writeln_doc!(out, section_title)?;
            }

            // group and sort entries
            let mut grouped = HashMap::new();
            for entry in entries {
                let key = entry.1.iter().take(depth + 1).collect::<PathBuf>();
                grouped.entry(key).or_insert_with(Vec::new).push(entry.clone());
            }
            let grouped = grouped.iter().sorted_by(|(lhs, _), (rhs, _)| {
                let lhs_at_end = lhs.extension().map(|ext| ext.eq("sol")).unwrap_or_default();
                let rhs_at_end = rhs.extension().map(|ext| ext.eq("sol")).unwrap_or_default();
                if lhs_at_end == rhs_at_end {
                    lhs.cmp(rhs)
                } else if lhs_at_end {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            });

            let indent = " ".repeat(2 * depth);
            let mut readme = String::from("\n\n# Contents\n");
            for (path, entries) in grouped {
                if path.extension().map(|ext| ext.eq("sol")).unwrap_or_default() {
                    for (src, filename) in entries {
                        writeln_doc!(
                            readme,
                            "- {}",
                            DocOutput::Link(
                                &src.name,
                                &Path::new("/").join(filename).display().to_string()
                            )
                        )?;

                        writeln_doc!(
                            out,
                            "{}- {}",
                            indent,
                            DocOutput::Link(&src.name, &filename.display().to_string())
                        )?;
                    }
                } else {
                    writeln_doc!(
                        readme,
                        "- {}",
                        DocOutput::Link(
                            &path.iter().last().unwrap().to_string_lossy(),
                            &Path::new("/").join(path).display().to_string()
                        )
                    )?;
                    self.write_summary_section(out, depth + 1, Some(&path), &entries)?;
                }
            }
            if !readme.is_empty() {
                if let Some(path) = path {
                    fs::write(
                        self.config.out_dir().join("src").join(path).join("README.md"),
                        readme,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn lookup_contract_base<'a>(
        &self,
        docs: &[(PathBuf, Vec<DocPart>)],
        base: &Base,
    ) -> eyre::Result<Option<String>> {
        for (base_path, base_doc) in docs {
            for base_part in base_doc.iter() {
                if let DocElement::Contract(base_contract) = &base_part.element {
                    if base.name.identifiers.last().unwrap().name == base_contract.name.name {
                        let path = PathBuf::from("/").join(
                            base_path
                                .strip_prefix(&self.config.root)?
                                .join(&format!("contract.{}.md", base_contract.name)),
                        );
                        return Ok(Some(
                            DocOutput::Link(&base.doc(), &path.display().to_string()).doc(),
                        ))
                    }
                }
            }
        }

        Ok(None)
    }
}
