use eyre;
use forge_fmt::Visitable;
use rayon::prelude::*;
use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

use crate::{doc_format::DocFormat, SolidityDoc, SolidityDocPartElement};

pub struct DocBuilder {
    root: PathBuf,
    paths: Vec<PathBuf>,
}

impl DocBuilder {
    pub fn new(root: &Path) -> Self {
        DocBuilder { root: root.to_owned(), paths: vec![] }
    }

    pub fn with_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.paths = paths;
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

        let out_dir = self.root.join("docs");
        let out_dir_src = out_dir.join("src");
        fs::create_dir_all(&out_dir_src)?;

        let mut filenames = vec![];
        for (path, doc) in docs.iter() {
            println!("processing {}", path.display());
            for part in doc.iter() {
                if let SolidityDocPartElement::Contract(ref contract) = part.element {
                    println!("\twriting {}", contract.name);
                    let mut out = vec![
                        format!("# {}", contract.name),
                        "".to_owned(),
                        part.comments.doc(),
                        "".to_owned(),
                    ];

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
                        out.push("## Attributes".to_owned());
                        out.extend(attributes.iter().flat_map(|(var, comments)| {
                            vec![var.doc(), comments.doc(), "".to_owned()]
                        }));
                    }

                    if !funcs.is_empty() {
                        out.push("## Functions".to_owned());
                        out.extend(funcs.iter().flat_map(|(func, comments)| {
                            vec![func.doc(), comments.doc(), "".to_owned()]
                        }));
                    }

                    let filename = format!("contract.{}.md", contract.name);
                    let mut new_path = path.strip_prefix(&self.root)?.to_owned();
                    new_path.pop();
                    fs::create_dir_all(out_dir_src.join(&new_path))?;
                    let new_path = new_path.join(&filename);
                    fs::write(out_dir_src.join(&new_path), out.join("\n"))?;
                    filenames.push((contract.name.clone(), new_path));
                }
            }
        }
        let mut summary = String::new();
        write!(
            summary,
            "# Summary\n{}",
            &filenames
                .iter()
                .map(|(src, filename)| format!("- [{}]({})", src, filename.display()))
                .collect::<Vec<_>>()
                .join("\n")
        )?;
        fs::write(out_dir_src.join("SUMMARY.md"), summary)?;
        // TODO: take values from config
        fs::write(out_dir.join("book.toml"), "[book]\ntitle = \"OZ\"\nsrc = \"src\"")?;
        Ok(())
    }
}
