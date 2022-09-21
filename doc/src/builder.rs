use eyre;
use forge_fmt::Visitable;
use itertools::Itertools;
use rayon::prelude::*;
use solang_parser::{
    doccomment::DocComment,
    pt::{Base, ContractTy, Identifier},
};
use std::{
    collections::HashMap,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    as_code::AsCode, format::DocFormat, macros::*, output::DocOutput, SolidityDoc, SolidityDocPart,
    SolidityDocPartElement,
};

pub struct DocBuilder {
    config: DocConfig,
}

// TODO: move & merge w/ Figment
#[derive(Debug)]
pub struct DocConfig {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub out: PathBuf,
    pub title: String,
}

impl DocConfig {
    pub fn new(root: &Path) -> Self {
        DocConfig { root: root.to_owned(), ..Default::default() }
    }

    fn out_dir(&self) -> PathBuf {
        self.root.join(&self.out)
    }
}

impl Default for DocConfig {
    fn default() -> Self {
        DocConfig {
            root: PathBuf::new(),
            out: PathBuf::from("docs"),
            title: "Title".to_owned(),
            paths: vec![],
        }
    }
}

impl DocBuilder {
    pub fn new() -> Self {
        DocBuilder { config: DocConfig::default() }
    }

    pub fn from_config(config: DocConfig) -> Self {
        DocBuilder { config }
    }

    pub fn build(self) -> eyre::Result<()> {
        let docs = self
            .config
            .paths
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
                let mut doc = SolidityDoc::new(comments);
                source_unit.visit(&mut doc)?;

                Ok((path.clone(), doc.parts))
            })
            .collect::<eyre::Result<HashMap<_, _>>>()?;

        let out_dir = self.config.out_dir();
        let out_dir_src = out_dir.join("src");
        fs::create_dir_all(&out_dir_src)?;

        let mut filenames = vec![];

        for (path, doc) in docs.iter() {
            for part in doc.iter() {
                if let SolidityDocPartElement::Contract(ref contract) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1(&contract.name.name))?;

                    if !contract.base.is_empty() {
                        let bases = contract
                            .base
                            .iter()
                            .map(|base| {
                                Ok(self.lookup_contract_base(&docs, base)?.unwrap_or(base.doc()))
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

                    for child in part.children.iter() {
                        // TODO: remove `clone`s
                        match &child.element {
                            SolidityDocPartElement::Function(func) => {
                                funcs.push((func.clone(), child.comments.clone()))
                            }
                            SolidityDocPartElement::Variable(var) => {
                                attributes.push((var.clone(), child.comments.clone()))
                            }
                            SolidityDocPartElement::Event(event) => {
                                events.push((event.clone(), child.comments.clone()))
                            }
                            _ => {}
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
                            writeln_doc!(doc_file, "{}\n{}\n", func, comments)?;
                            writeln_code!(doc_file, "{}", func)?;
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

                    let new_path = path.strip_prefix(&self.config.root)?.to_owned();
                    fs::create_dir_all(out_dir_src.join(&new_path))?;
                    let new_path = new_path.join(&format!("contract.{}.md", contract.name));

                    fs::write(out_dir_src.join(&new_path), doc_file)?;
                    filenames.push((contract.name.clone(), contract.ty.clone(), new_path));
                }
            }
        }

        let mut summary = String::new();
        writeln_doc!(summary, DocOutput::H1("Summary"))?;

        self.write_section(&mut summary, 0, None, &filenames)?;

        fs::write(out_dir_src.join("SUMMARY.md"), summary)?;
        fs::write(
            out_dir.join("book.toml"),
            format!(
                "[book]\ntitle = \"{}\"\nsrc = \"src\"\n\n[output.html]\nno-section-label = true\n\n[output.html.fold]\nenable = true",
                self.config.title
            ),
        )?;
        Ok(())
    }

    fn write_section(
        &self,
        out: &mut String,
        depth: usize,
        path: Option<&Path>,
        entries: &[(Identifier, ContractTy, PathBuf)],
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

            let mut grouped = HashMap::new();
            for entry in entries {
                let key = entry.2.iter().take(depth + 1).collect::<PathBuf>();
                grouped.entry(key).or_insert_with(Vec::new).push(entry.clone());
            }
            // TODO:
            let grouped = grouped.iter().sorted_by(|(left_key, _), (right_key, _)| {
                (left_key.extension().map(|ext| ext.eq("sol")).unwrap_or_default() as i32).cmp(
                    &(right_key.extension().map(|ext| ext.eq("sol")).unwrap_or_default() as i32),
                )
            });

            let indent = " ".repeat(2 * depth);
            let mut readme = String::from("\n\n# Contents\n");
            for (path, entries) in grouped {
                if path.extension().map(|ext| ext.eq("sol")).unwrap_or_default() {
                    for (src, _, filename) in entries {
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
                            &path.join("README.md").display().to_string()
                        )
                    )?;
                    // let subsection = path.iter().last().unwrap().to_string_lossy();
                    self.write_section(out, depth + 1, Some(&path), &entries)?;
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

    fn lookup_contract_base(
        &self,
        docs: &HashMap<PathBuf, Vec<SolidityDocPart>>,
        base: &Base,
    ) -> eyre::Result<Option<String>> {
        for (base_path, base_doc) in docs.iter() {
            for base_part in base_doc.iter() {
                if let SolidityDocPartElement::Contract(base_contract) = &base_part.element {
                    if base.name.identifiers.last().unwrap().name == base_contract.name.name {
                        return Ok(Some(format!(
                            "[{}]({})",
                            base.doc(),
                            PathBuf::from("/")
                                .join(
                                    base_path
                                        .strip_prefix(&self.config.root)?
                                        .join(&format!("contract.{}.md", base_contract.name))
                                )
                                .display(),
                        )))
                    }
                }
            }
        }

        Ok(None)
    }
}
