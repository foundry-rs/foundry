use eyre;
use forge_fmt::Visitable;
use rayon::prelude::*;
use solang_parser::pt::{ContractTy, Identifier};
use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use crate::{format::DocFormat, macros::*, output::DocOutput, SolidityDoc, SolidityDocPartElement};

pub struct DocBuilder {
    root: PathBuf,
    out_dir: PathBuf,
    paths: Vec<PathBuf>,
}

impl DocBuilder {
    pub fn new(root: &Path) -> Self {
        DocBuilder { root: root.to_owned(), out_dir: PathBuf::from("docs"), paths: vec![] }
    }

    pub fn with_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.paths = paths;
        self
    }

    pub fn with_out_dir(mut self, out_dir: &Path) -> Self {
        self.out_dir = out_dir.to_owned();
        self
    }

    pub fn build(self) -> eyre::Result<()> {
        let docs = self
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

        let out_dir = self.root.join("docs"); // TODO: config
        let out_dir_src = out_dir.join("src");
        fs::create_dir_all(&out_dir_src)?;

        let mut filenames = vec![];
        for (path, doc) in docs.iter() {
            for part in doc.iter() {
                if let SolidityDocPartElement::Contract(ref contract) = part.element {
                    let mut doc_file = String::new();
                    writeln_doc!(doc_file, DocOutput::H1(&contract.name.name))?;
                    if !contract.base.is_empty() {
                        writeln_doc!(
                            doc_file,
                            "{} {}\n",
                            DocOutput::Bold("Inherits:"),
                            contract.base
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
                            writeln_doc!(doc_file, "{}{}\n", var, comments)?;
                        }
                    }

                    if !funcs.is_empty() {
                        writeln_doc!(doc_file, DocOutput::H2("Functions"))?;
                        for (func, comments) in funcs {
                            writeln_doc!(doc_file, "{}{}\n", func, comments)?;
                        }
                    }

                    let mut new_path = path.strip_prefix(&self.root)?.to_owned();
                    new_path.pop();
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
        fs::write(out_dir.join("book.toml"), "[book]\ntitle = \"Title\"\nsrc = \"src\"")?;
        Ok(())
    }
}
