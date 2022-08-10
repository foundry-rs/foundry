use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use forge_doc::{SolidityDoc, SolidityDocPart, SolidityDocPartElement};
use forge_fmt::Visitable;
use foundry_config::load_config_with_root;
use rayon::prelude::*;
use solang_parser::doccomment::DocComment;
use std::{fs, path::PathBuf};

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
                DocComment::Line { comment } => {
                    // format!("Tag: {}. {}", comment.tag, comment.value)
                    format!("{}", comment.value)
                }
                DocComment::Block { comments } => comments
                    .iter()
                    .map(|comment|
                        // format!("Tag: {}. {}", comment.tag, comment.value)
                        format!("{}", comment.value))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn write_docs(&self, parts: &Vec<SolidityDocPart>) -> Result<(), eyre::Error> {
        // TODO:
        let out_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("oz-governance");
        std::fs::create_dir_all(&out_dir.join("docs"))?;
        for part in parts.iter() {
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
                            func.name.as_ref().map_or(func.ty.to_string(), |n| n.name.to_owned())
                        ),
                        self.format_comments(&comments),
                        "".to_owned(),
                    ]
                }));

                std::fs::write(out_dir.join("docs").join("Governor.md"), out.join("\n"))?;
            }

            std::fs::write(
                out_dir.join("docs").join("SUMMARY.md"),
                "# OZ Governance\n - [Governor](Governor.md)",
            )?;
            std::fs::write(
                out_dir.join("book.toml"),
                "[book]\ntitle = \"OZ Governance\"\nsrc = \"docs\"",
            )?;
        }
        // let title = "Test";
        // let summary: Summary = Summary {
        //     title: Some(title.to_owned()),
        //     prefix_chapters: vec![],
        //     numbered_chapters: vec![
        //         SummaryItem::PartTitle("Contracts".to_owned()),
        //         SummaryItem::Link(Link::new("Governor2",
        // src_dir.join("src").join("Governor.md"))),     ],
        //     suffix_chapters: vec![],
        // };
        // let mut config = Config::default();
        // config.book = BookConfig {
        //     title: Some(title.to_owned()),
        //     language: Some("solidity".to_owned()),
        //     authors: vec![],
        //     description: None,
        //     multilingual: false,
        //     src: src_dir.clone(),
        // };
        // config.build = BuildConfig {
        //     build_dir: src_dir.join("book"),
        //     create_missing: false,
        //     use_default_preprocessors: true,
        // };
        // MDBook::load_with_config_and_summary(env!("CARGO_MANIFEST_DIR"), config, summary)
        //     .unwrap()
        //     .build()
        //     .unwrap();
        Ok(())
    }
}

impl Cmd for DocArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = load_config_with_root(self.root.clone());
        let paths: Vec<PathBuf> = config.project_paths().input_files();
        paths
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

                Ok(())
            })
            .collect::<eyre::Result<Vec<_>>>()?;

        Ok(())
    }
}
