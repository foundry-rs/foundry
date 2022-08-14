use solang_parser::{
    doccomment::DocComment,
    pt::{FunctionDefinition, VariableDefinition},
};

pub trait DocFormat {
    fn doc(&self) -> String;
}

impl DocFormat for DocComment {
    fn doc(&self) -> String {
        match self {
            DocComment::Line { comment } => comment.value.to_owned(),
            DocComment::Block { comments } => comments
                .iter()
                .map(|comment| comment.value.to_owned())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl DocFormat for Vec<DocComment> {
    fn doc(&self) -> String {
        self.iter().map(DocComment::doc).collect::<Vec<_>>().join("\n")
    }
}

impl DocFormat for VariableDefinition {
    fn doc(&self) -> String {
        format!("### {}", self.name.name)
    }
}

impl DocFormat for FunctionDefinition {
    fn doc(&self) -> String {
        let name = self.name.as_ref().map_or(self.ty.to_string(), |n| n.name.to_owned());
        format!("### {}\n{}", name, self.ty)
    }
}
