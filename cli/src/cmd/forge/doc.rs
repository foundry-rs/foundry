use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use forge_doc::{SolidityDoc, SolidityDocPartElement};
use forge_fmt::Visitable;
use foundry_config::{find_project_root_path, load_config_with_root};
use rayon::prelude::*;
use solang_parser::doccomment::DocComment;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Parser)]
pub struct DocArgs {
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,
}

impl DocArgs {
    fn format_comments(&self, comments: &Vec<DocComment>) -> String {
        comments
            .iter()
            .map(|c| match c {
                DocComment::Line { comment } => comment.value.to_owned(),
                DocComment::Block { comments } => comments
                    .iter()
                    .map(|comment| comment.value.to_owned())
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Cmd for DocArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = load_config_with_root(self.root.clone());
        let paths: Vec<PathBuf> = config.project_paths().input_files();
        let docs = paths
            .par_iter()
            .enumerate()
            .map(|(i, path)| {
                let source =  fs::read_to_string(&path)?;
                let (mut source_unit, comments) = solang_parser::parse(&source, i)
                    .map_err(|diags| eyre::eyre!(
                            "Failed to parse Solidity code for {}. Leaving source unchanged.\nDebug info: {:?}",
                            path.display(),
                            diags
                        ))?;
                let mut doc = SolidityDoc::new(comments);
                source_unit.visit(&mut doc)?;

                Ok((path.clone(), doc.parts))
            })
            .collect::<eyre::Result<Vec<_>>>()?;

        let out_dir = self.root.as_ref().unwrap_or(&find_project_root_path()?).join("docs");
        fs::create_dir_all(&out_dir.join("src"))?;

        let mut filenames = vec![];
        for (path, doc) in docs.iter() {
            for part in doc.iter() {
                if let SolidityDocPartElement::Contract(ref contract) = part.element {
                    let mut out = vec![
                        format!("# {}", contract.name),
                        "".to_owned(),
                        self.format_comments(&part.comments),
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

                    out.push("## Attributes".to_owned());
                    out.extend(attributes.iter().flat_map(|(var, comments)| {
                        vec![
                            format!("### {}", var.name.name),
                            self.format_comments(&comments),
                            "".to_owned(),
                        ]
                    }));

                    out.push("## Functions".to_owned());
                    out.extend(funcs.iter().flat_map(|(func, comments)| {
                        vec![
                            format!(
                                "### {}",
                                func.name
                                    .as_ref()
                                    .map_or(func.ty.to_string(), |n| n.name.to_owned())
                            ),
                            self.format_comments(&comments),
                            "".to_owned(),
                        ]
                    }));

                    let src_filename = Path::new(path).file_stem().unwrap().to_str().unwrap();
                    let filename = format!("{}.md", src_filename);
                    std::fs::write(out_dir.join("src").join(&filename), out.join("\n"))?;
                    filenames.push((src_filename, filename));
                }
            }
        }
        let mut summary = String::from("# Summary\n");
        summary.push_str(
            &filenames
                .iter()
                .map(|(src, filename)| format!("- [{}]({})", src, filename))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        std::fs::write(out_dir.join("src").join("SUMMARY.md"), summary)?;
        std::fs::write(out_dir.join("book.toml"), "[book]\ntitle = \"OZ\"\nsrc = \"src\"")?;

        Ok(())
    }
}
