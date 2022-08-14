use forge_fmt::{Visitable, Visitor};
use solang_parser::{
    doccomment::{parse_doccomments, DocComment},
    pt::{
        Comment, ContractDefinition, FunctionDefinition, Loc, SourceUnit, SourceUnitPart,
        VariableDefinition,
    },
};
use thiserror::Error;

pub mod builder;
mod doc_format;
mod macros;

#[derive(Error, Debug)]
enum DocError {} // TODO:

type Result<T, E = DocError> = std::result::Result<T, E>;

#[derive(Debug)]
struct SolidityDoc {
    parts: Vec<SolidityDocPart>,
    comments: Vec<Comment>,
    start_at: usize,
    curr_parent: Option<SolidityDocPart>,
}

#[derive(Debug, PartialEq)]
struct SolidityDocPart {
    comments: Vec<DocComment>,
    element: SolidityDocPartElement,
    children: Vec<SolidityDocPart>,
}

impl SolidityDoc {
    fn new(comments: Vec<Comment>) -> Self {
        SolidityDoc { parts: vec![], comments, start_at: 0, curr_parent: None }
    }

    fn with_parent(
        &mut self,
        mut parent: SolidityDocPart,
        mut visit: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<SolidityDocPart> {
        let curr = self.curr_parent.take();
        self.curr_parent = Some(parent);
        visit(self)?;
        parent = self.curr_parent.take().unwrap();
        self.curr_parent = curr;
        Ok(parent)
    }

    fn add_element_to_parent(&mut self, element: SolidityDocPartElement, loc: Loc) {
        let child =
            SolidityDocPart { comments: self.parse_docs(loc.start()), element, children: vec![] };
        if let Some(parent) = self.curr_parent.as_mut() {
            parent.children.push(child);
        } else {
            self.parts.push(child);
        }
        self.start_at = loc.end();
    }

    fn parse_docs(&mut self, end: usize) -> Vec<DocComment> {
        parse_doccomments(&self.comments, self.start_at, end)
    }
}

#[derive(Debug, PartialEq)]
enum SolidityDocPartElement {
    Contract(Box<ContractDefinition>),
    Function(FunctionDefinition),
    Variable(VariableDefinition),
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
                        for d in def.parts.iter_mut() {
                            d.visit(doc)?;
                        }

                        Ok(())
                    })?;

                    self.start_at = def.loc.end();
                    self.parts.push(contract);
                }
                SourceUnitPart::FunctionDefinition(func) => self.visit_function(func)?,
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
