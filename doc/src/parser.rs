use forge_fmt::{Visitable, Visitor};
use solang_parser::{
    doccomment::{parse_doccomments, DocComment, DocCommentTag},
    pt::{
        Comment, ContractDefinition, EventDefinition, FunctionDefinition, Loc, SourceUnit,
        SourceUnitPart, StructDefinition, VariableDefinition,
    },
};
use thiserror::Error;

/// The parser error
#[derive(Error, Debug)]
pub(crate) enum DocParserError {}

type Result<T, E = DocParserError> = std::result::Result<T, E>;

#[derive(Debug)]
pub(crate) struct DocParser {
    pub(crate) parts: Vec<DocPart>,
    comments: Vec<Comment>,
    start_at: usize,
    context: DocContext,
}

#[derive(Debug, Default)]
struct DocContext {
    parent: Option<DocPart>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct DocPart {
    pub(crate) comments: Vec<DocCommentTag>,
    pub(crate) element: DocElement,
    pub(crate) children: Vec<DocPart>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum DocElement {
    Contract(Box<ContractDefinition>),
    Function(FunctionDefinition),
    Variable(VariableDefinition),
    Event(EventDefinition),
    Struct(StructDefinition),
}

impl DocParser {
    pub(crate) fn new(comments: Vec<Comment>) -> Self {
        DocParser { parts: vec![], comments, start_at: 0, context: Default::default() }
    }

    fn with_parent(
        &mut self,
        mut parent: DocPart,
        mut visit: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<DocPart> {
        let curr = self.context.parent.take();
        self.context.parent = Some(parent);
        visit(self)?;
        parent = self.context.parent.take().unwrap();
        self.context.parent = curr;
        Ok(parent)
    }

    fn add_element_to_parent(&mut self, element: DocElement, loc: Loc) {
        let child = DocPart { comments: self.parse_docs(loc.start()), element, children: vec![] };
        if let Some(parent) = self.context.parent.as_mut() {
            parent.children.push(child);
        } else {
            self.parts.push(child);
        }
        self.start_at = loc.end();
    }

    fn parse_docs(&mut self, end: usize) -> Vec<DocCommentTag> {
        let mut res = vec![];
        for comment in parse_doccomments(&self.comments, self.start_at, end) {
            match comment {
                DocComment::Line { comment } => res.push(comment),
                DocComment::Block { comments } => res.extend(comments.into_iter()),
            }
        }
        res
    }
}

impl Visitor for DocParser {
    type Error = DocParserError;

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> Result<()> {
        for source in source_unit.0.iter_mut() {
            match source {
                SourceUnitPart::ContractDefinition(def) => {
                    let contract = DocPart {
                        element: DocElement::Contract(def.clone()),
                        comments: self.parse_docs(def.loc.start()),
                        children: vec![],
                    };
                    self.start_at = def.loc.start();
                    let contract = self.with_parent(contract, |doc| {
                        def.parts.iter_mut().map(|d| d.visit(doc)).collect::<Result<Vec<_>>>()?;
                        Ok(())
                    })?;

                    self.start_at = def.loc.end();
                    self.parts.push(contract);
                }
                SourceUnitPart::FunctionDefinition(func) => self.visit_function(func)?,
                SourceUnitPart::EventDefinition(event) => self.visit_event(event)?,
                SourceUnitPart::StructDefinition(structure) => self.visit_struct(structure)?,
                _ => {}
            };
        }

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Function(func.clone()), func.loc);
        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Variable(var.clone()), var.loc);
        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Event(event.clone()), event.loc);
        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> Result<()> {
        self.add_element_to_parent(DocElement::Struct(structure.clone()), structure.loc);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    #[test]
    fn parse_docs() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("oz-governance")
            .join("Governor.sol");
        let target = fs::read_to_string(path).unwrap();
        let (mut source_pt, source_comments) = solang_parser::parse(&target, 1).unwrap();
        let mut d = DocParser::new(source_comments);
        assert!(source_pt.visit(&mut d).is_ok());
    }
}
