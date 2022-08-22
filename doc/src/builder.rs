use eyre;
use forge_fmt::Visitable;
use rayon::prelude::*;
use solang_parser::pt::{Base, ContractTy, Identifier};
use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    format::DocFormat, macros::*, output::DocOutput, SolidityDoc, SolidityDocPart,
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
            .collect::<eyre::Result<Vec<_>>>()?;

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

                    for child in part.children.iter() {
                        match &child.element {
                            SolidityDocPartElement::Function(func) => {
                                funcs.push((func.clone(), child.comments.clone()))
                            }
                            SolidityDocPartElement::Variable(var) => {
                                attributes.push((var.clone(), child.comments.clone()))
                            }
                            _ => {}
                        }
                    }

                    if !attributes.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Attributes"))?;
                        for (var, comments) in attributes {
                            writeln_doc!(doc_file, "{}\n{}\n", var, comments)?;
                        }
                    }

                    if !funcs.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Functions"))?;
                        for (func, comments) in funcs {
                            writeln_doc!(doc_file, "{}\n{}\n", func, comments)?;
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

        // TODO:
        let mut interfaces = vec![];
        let mut contracts = vec![];
        let mut abstract_contracts = vec![];
        let mut libraries = vec![];
        for entry in filenames.into_iter() {
            match entry.1 {
                ContractTy::Abstract(_) => abstract_contracts.push(entry),
                ContractTy::Contract(_) => contracts.push(entry),
                ContractTy::Interface(_) => interfaces.push(entry),
                ContractTy::Library(_) => libraries.push(entry),
            }
        }

        let mut write_section = |title: &str,
                                 entries: &[(Identifier, ContractTy, PathBuf)]|
         -> Result<(), std::fmt::Error> {
            if !entries.is_empty() {
                writeln_doc!(summary, DocOutput::H1(title))?;
                for (src, _, filename) in entries {
                    writeln_doc!(
                        summary,
                        "- {}",
                        DocOutput::Link(&src.name, &filename.display().to_string())
                    )?;
                }
                writeln!(summary)?;
            }
            Ok(())
        };

        write_section("Contracts", &contracts)?;
        write_section("Libraries", &libraries)?;
        write_section("Interfaces", &interfaces)?;
        write_section("Abstract Contracts", &abstract_contracts)?;

        fs::write(out_dir_src.join("SUMMARY.md"), summary)?;
        // TODO: take values from config
        fs::write(
            out_dir.join("book.toml"),
            format!("[book]\ntitle = \"{}\"\nsrc = \"src\"", self.config.title),
        )?;
        Ok(())
    }

    fn lookup_contract_base(
        &self,
        docs: &Vec<(PathBuf, Vec<SolidityDocPart>)>,
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
