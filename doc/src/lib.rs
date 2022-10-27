use forge_fmt::{Visitable, Visitor};
use solang_parser::{
    doccomment::{parse_doccomments, DocComment, DocCommentTag},
    pt::{
        Comment, ContractDefinition, EventDefinition, FunctionDefinition, Loc, SourceUnit,
        SourceUnitPart, VariableDefinition,
    },
};
use thiserror::Error;

mod as_code;
pub mod builder;
mod format;
mod helpers;
mod macros;
mod output;

#[derive(Error, Debug)]
enum DocError {} // TODO:

type Result<T, E = DocError> = std::result::Result<T, E>;

#[derive(Debug)]
struct SolidityDoc {
    parts: Vec<SolidityDocPart>,
    comments: Vec<Comment>,
    start_at: usize,
    context: DocContext,
}

#[derive(Debug, Default)]
struct DocContext {
    parent: Option<SolidityDocPart>,
}

#[derive(Debug, PartialEq)]
struct SolidityDocPart {
    comments: Vec<DocCommentTag>,
    element: SolidityDocPartElement,
    children: Vec<SolidityDocPart>,
}

impl SolidityDoc {
    fn new(comments: Vec<Comment>) -> Self {
        SolidityDoc { parts: vec![], comments, start_at: 0, context: Default::default() }
    }

    fn with_parent(
        &mut self,
        mut parent: SolidityDocPart,
        mut visit: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<SolidityDocPart> {
        let curr = self.context.parent.take();
        self.context.parent = Some(parent);
        visit(self)?;
        parent = self.context.parent.take().unwrap();
        self.context.parent = curr;
        Ok(parent)
    }

    fn add_element_to_parent(&mut self, element: SolidityDocPartElement, loc: Loc) {
        let child =
            SolidityDocPart { comments: self.parse_docs(loc.start()), element, children: vec![] };
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

#[derive(Debug, PartialEq)]
enum SolidityDocPartElement {
    Contract(Box<ContractDefinition>),
    Function(FunctionDefinition),
    Variable(VariableDefinition),
    Event(EventDefinition),
}

impl Visitor for SolidityDoc {
    type Error = DocError;

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> Result<()> {
        for source in source_unit.0.iter_mut() {
            match source {
                SourceUnitPart::ContractDefinition(def) => {
                    let contract = SolidityDocPart {
                        element: SolidityDocPartElement::Contract(def.clone()),
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
                _ => {}
            };
        }

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<()> {
        self.add_element_to_parent(SolidityDocPartElement::Function(func.clone()), func.loc);
        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<()> {
        self.add_element_to_parent(SolidityDocPartElement::Variable(var.clone()), var.loc);
        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> Result<()> {
        self.add_element_to_parent(SolidityDocPartElement::Event(event.clone()), event.loc);
        Ok(())
    }

    // TODO: structs
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
        let mut d = SolidityDoc::new(source_comments);
        assert!(source_pt.visit(&mut d).is_ok());
    }
}
