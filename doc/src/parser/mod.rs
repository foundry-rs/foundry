//! The parser module.

use forge_fmt::{Visitable, Visitor};
use solang_parser::{
    doccomment::{parse_doccomments, DocComment, DocCommentTag},
    pt::{
        Comment, EnumDefinition, ErrorDefinition, EventDefinition, FunctionDefinition, Loc,
        SourceUnit, SourceUnitPart, StructDefinition, VariableDefinition,
    },
};

/// Parser error.
pub mod error;
use error::{NoopError, ParserResult};

mod item;
pub use item::{ParseItem, ParseSource};

/// The documentation parser. This type implements a [Visitor] trait. While walking the parse tree,
/// [Parser] will collect relevant source items and corresponding doc comments. The resulting
/// [ParseItem]s can be accessed by calling [Parser::items].
#[derive(Debug, Default)]
pub struct Parser {
    items: Vec<ParseItem>,
    comments: Vec<Comment>,
    start_at: usize,
    context: ParserContext,
}

/// [Parser]'s context. Holds information about current parent that's being visited.
#[derive(Debug, Default)]
struct ParserContext {
    parent: Option<ParseItem>,
}

impl Parser {
    /// Create a new instance of [Parser].
    pub fn new(comments: Vec<Comment>) -> Self {
        Parser { comments, ..Default::default() }
    }

    /// Return the parsed items. Consumes the parser.
    pub fn items(self) -> Vec<ParseItem> {
        self.items
    }

    /// Visit the children elements with parent context.
    /// This function memoizes the previous parent, sets the context
    /// to a new one and invokes a visit function. The context will be reset
    /// to the previous parent at the end of the function.
    fn with_parent(
        &mut self,
        mut parent: ParseItem,
        mut visit: impl FnMut(&mut Self) -> ParserResult<()>,
    ) -> ParserResult<ParseItem> {
        let curr = self.context.parent.take();
        self.context.parent = Some(parent);
        visit(self)?;
        parent = self.context.parent.take().unwrap();
        self.context.parent = curr;
        Ok(parent)
    }

    /// Adds a child element to the parent parsed item.
    /// Moves the doc comment pointer to the end location of the child element.
    fn add_element_to_parent(&mut self, source: ParseSource, loc: Loc) {
        let child = ParseItem { source, comments: self.parse_docs(loc.start()), children: vec![] };
        if let Some(parent) = self.context.parent.as_mut() {
            parent.children.push(child);
        } else {
            self.items.push(child);
        }
        self.start_at = loc.end();
    }

    /// Parse the doc comments from the current start location.
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

impl Visitor for Parser {
    type Error = NoopError;

    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> ParserResult<()> {
        for source in source_unit.0.iter_mut() {
            match source {
                SourceUnitPart::ContractDefinition(def) => {
                    let contract = ParseItem {
                        source: ParseSource::Contract(def.clone()),
                        comments: self.parse_docs(def.loc.start()),
                        children: vec![],
                    };
                    self.start_at = def.loc.start();
                    let contract = self.with_parent(contract, |doc| {
                        def.parts
                            .iter_mut()
                            .map(|d| d.visit(doc))
                            .collect::<ParserResult<Vec<_>>>()?;
                        Ok(())
                    })?;

                    self.start_at = def.loc.end();
                    self.items.push(contract);
                }
                SourceUnitPart::FunctionDefinition(func) => self.visit_function(func)?,
                SourceUnitPart::EventDefinition(event) => self.visit_event(event)?,
                SourceUnitPart::ErrorDefinition(error) => self.visit_error(error)?,
                SourceUnitPart::StructDefinition(structure) => self.visit_struct(structure)?,
                SourceUnitPart::EnumDefinition(enumerable) => self.visit_enum(enumerable)?,
                _ => {}
            };
        }

        Ok(())
    }

    fn visit_function(&mut self, func: &mut FunctionDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Function(func.clone()), func.loc);
        Ok(())
    }

    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Variable(var.clone()), var.loc);
        Ok(())
    }

    fn visit_event(&mut self, event: &mut EventDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Event(event.clone()), event.loc);
        Ok(())
    }

    fn visit_error(&mut self, error: &mut ErrorDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Error(error.clone()), error.loc);
        Ok(())
    }

    fn visit_struct(&mut self, structure: &mut StructDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Struct(structure.clone()), structure.loc);
        Ok(())
    }

    fn visit_enum(&mut self, enumerable: &mut EnumDefinition) -> ParserResult<()> {
        self.add_element_to_parent(ParseSource::Enum(enumerable.clone()), enumerable.loc);
        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::{fs, path::PathBuf};

//     // TODO: write tests w/ sol source code

//     #[test]
//     fn parse_docs() {
//         let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
//             .join("testdata")
//             .join("oz-governance")
//             .join("Governor.sol");
//         let target = fs::read_to_string(path).unwrap();
//         let (mut source_pt, source_comments) = solang_parser::parse(&target, 1).unwrap();
//         let mut d = DocParser::new(source_comments);
//         assert!(source_pt.visit(&mut d).is_ok());
//     }
// }
