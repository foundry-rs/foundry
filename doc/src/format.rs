use itertools::Itertools;
use solang_parser::{
    doccomment::DocComment,
    pt::{Base, FunctionDefinition, VariableDefinition},
};

use crate::output::DocOutput;

pub trait DocFormat {
    fn doc(&self) -> String;
}

impl<'a> DocFormat for DocOutput<'a> {
    fn doc(&self) -> String {
        match self {
            Self::H1(val) => format!("# {val}"),
            Self::H2(val) => format!("## {val}"),
            Self::H3(val) => format!("### {val}"),
            Self::Bold(val) => format!("**{val}**"),
            Self::Link(val, link) => format!("[{val}]({link})"),
        }
    }
}

// TODO:
impl DocFormat for String {
    fn doc(&self) -> String {
        self.to_owned()
    }
}

impl DocFormat for DocComment {
    fn doc(&self) -> String {
        match self {
            DocComment::Line { comment } => comment.value.to_owned(),
            DocComment::Block { comments } => {
                comments.iter().map(|comment| comment.value.to_owned()).join("\n")
            }
        }
    }
}

impl DocFormat for Vec<DocComment> {
    fn doc(&self) -> String {
        self.iter().map(DocComment::doc).join("\n")
    }
}

impl DocFormat for Base {
    fn doc(&self) -> String {
        self.name.identifiers.iter().map(|ident| ident.name.clone()).join(".")
    }
}

impl DocFormat for Vec<Base> {
    fn doc(&self) -> String {
        self.iter().map(|base| base.doc()).join(", ")
    }
}

impl DocFormat for VariableDefinition {
    fn doc(&self) -> String {
        DocOutput::H3(&self.name.name).doc()
    }
}

impl DocFormat for FunctionDefinition {
    fn doc(&self) -> String {
        let name = self.name.as_ref().map_or(self.ty.to_string(), |n| n.name.to_owned());
        DocOutput::H3(&name).doc()
    }
}
